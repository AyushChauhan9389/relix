//! Playback with mpv; Relix draws the video (video.rs) and the controls (player_ui.rs). ADR 0002.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use libmpv2::{Format, Mpv, events::Event, events::PropertyData};
pub use relix_proto::addon::Subtitle;

/// Where playback stopped, for Continue Watching.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Progress {
    pub position: f64,
    pub duration: f64,
}

/// How playback ended, beyond the file simply finishing or the player stopping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ended {
    /// "Next episode" in the controls.
    NextRequested,
    /// mpv couldn't open or decode the stream (dead link, unsupported format).
    Failed,
}

/// What the app sends a running player.
pub enum ToPlayer {
    /// `sub-add` arguments for one addon's subtitles.
    Subtitles(Vec<[String; 4]>),
    Stop,
    Next,
}

/// What the controls show around the video.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    pub title: String,
    /// e.g. "S1:E2 Cat's in the Bag..."; empty for movies.
    pub episode: String,
    /// Shows "Next episode" and the countdown card.
    pub has_next: bool,
}

/// A subtitle or audio track mpv knows about.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: i64,
    pub title: String,
}

/// The player's state for the controls; a fresh copy arrives whenever it changes.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub loaded: bool,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub subs: Vec<Track>,
    pub audio: Vec<Track>,
    /// The selected subtitle/audio track; `None` is off.
    pub sid: Option<i64>,
    pub aid: Option<i64>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            loaded: false,
            position: 0.,
            duration: 0.,
            paused: false,
            volume: 100.,
            muted: false,
            speed: 1.,
            subs: vec![],
            audio: vec![],
            sid: None,
            aid: None,
        }
    }
}

/// `sub-add` arguments for an addon's subtitles: added unselected, titled by language and addon,
/// numbered only when a language repeats.
pub fn sub_add_args(subs: &[Subtitle], addon: &str) -> Vec<[String; 4]> {
    let mut seen = HashMap::<&str, usize>::new();
    subs.iter()
        .map(|s| {
            let total = subs.iter().filter(|o| o.lang == s.lang).count();
            let n = seen.entry(&s.lang).and_modify(|n| *n += 1).or_insert(1);
            let title = if total > 1 { format!("{} · {addon} {n}", s.lang) } else { format!("{} · {addon}", s.lang) };
            [s.url.clone(), "auto".into(), title, s.lang.clone()]
        })
        .collect()
}

/// Plays `url` from `start` seconds and blocks until playback stops or the file ends.
///
/// With `video`, mpv renders for Relix to draw in its own window (video.rs): it's handed the
/// player as soon as it exists. Without it (the `--play` check) mpv opens its own window with its
/// own controls. `headers` are an addon stream's `proxyHeaders.request`. `inbox` brings subtitles
/// (attached once the file loads, so they never delay playback), stop and next requests; `status`
/// gets the player's state whenever it changes.
pub fn play(
    url: &str,
    headers: &HashMap<String, String>,
    start: f64,
    title: &str,
    video: Option<Box<dyn FnOnce(Arc<Mpv>) + Send>>,
    inbox: Receiver<ToPlayer>,
    status: impl Fn(Status),
) -> libmpv2::Result<(Progress, Option<Ended>)> {
    let embedded = video.is_some();
    let mpv = Arc::new(Mpv::with_initializer(|init| {
        init.set_option("hwdec", "auto-safe")?;
        init.set_option("keep-open", "no")?;
        if let Ok(log) = std::env::var("RELIX_MPV_LOG") {
            init.set_option("log-file", log.as_str())?;
        }
        if embedded {
            init.set_option("vo", "libmpv")?;
            // Relix draws the controls; mpv draws only the video and subtitles.
            init.set_option("osc", "no")?;
            init.set_option("osd-level", 0)?;
        } else {
            init.set_option("vo", "gpu-next")?;
            init.set_option("force-window", "immediate")?;
            init.set_option("input-default-bindings", "yes")?;
            init.set_option("input-vo-keyboard", "yes")?;
        }
        if crate::test_offscreen() {
            init.set_option("mute", "yes")?;
            // mpv's own window (the `--play` check) also stays out of sight.
            init.set_option("geometry", "1280x720+-20000+-20000")?;
            init.set_option("focus-on", "never")?;
        }
        init.set_option("title", if title.is_empty() { "Relix".to_owned() } else { format!("{title} · Relix") }.as_str())?;
        if start > 0. {
            init.set_option("start", start.to_string().as_str())?;
        }
        if !headers.is_empty() {
            // ponytail: mpv splits this list on ',', so header values containing commas break; escape when an addon needs it.
            let fields: Vec<String> = headers.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            init.set_option("http-header-fields", fields.join(",").as_str())?;
        }
        Ok(())
    })?);
    if let Some(hand_over) = video {
        hand_over(mpv.clone());
    }

    let played = run(&mpv, url, &inbox, &status);
    // Make sure the core stops even if something else still holds it.
    let _ = mpv.command("quit", &[]);
    played
}

fn run(mpv: &Mpv, url: &str, inbox: &Receiver<ToPlayer>, send: &impl Fn(Status)) -> libmpv2::Result<(Progress, Option<Ended>)> {
    for (id, (name, format)) in [
        ("time-pos", Format::Double),
        ("duration", Format::Double),
        ("pause", Format::Flag),
        ("volume", Format::Double),
        ("mute", Format::Flag),
        ("speed", Format::Double),
        ("track-list/count", Format::Int64),
        ("sid", Format::String),
        ("aid", Format::String),
    ]
    .into_iter()
    .enumerate()
    {
        mpv.observe_property(name, format, id as u64)?;
    }
    mpv.command("loadfile", &[url])?;

    let mut status = Status::default();
    // The position changes every frame; the controls get it a few times a second.
    let mut sent_position = f64::MIN;
    // Subtitles wait here until the file has loaded.
    let mut pending: Vec<[String; 4]> = vec![];
    loop {
        for msg in inbox.try_iter() {
            match msg {
                ToPlayer::Subtitles(subs) => pending.extend(subs),
                ToPlayer::Stop => return Ok((progress(&status), None)),
                ToPlayer::Next => return Ok((progress(&status), Some(Ended::NextRequested))),
            }
        }
        if status.loaded {
            for args in pending.drain(..) {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                if let Err(e) = mpv.command("sub-add", &args) {
                    eprintln!("sub-add {}: {e}", args[0]);
                }
            }
        }
        // Wakes now and then to pick up subtitles and requests that arrived meanwhile.
        let before = status.clone();
        match mpv.wait_event(0.25) {
            Some(Ok(Event::FileLoaded)) => status.loaded = true,
            Some(Ok(Event::PropertyChange { change, reply_userdata, .. })) => match (reply_userdata, change) {
                (0, PropertyData::Double(v)) => status.position = v,
                (1, PropertyData::Double(v)) => status.duration = v,
                (2, PropertyData::Flag(v)) => status.paused = v,
                (3, PropertyData::Double(v)) => status.volume = v,
                (4, PropertyData::Flag(v)) => status.muted = v,
                (5, PropertyData::Double(v)) => status.speed = v,
                (6, PropertyData::Int64(n)) => (status.subs, status.audio) = tracks(mpv, n),
                (7, PropertyData::Str(v)) => status.sid = v.parse().ok(),
                (8, PropertyData::Str(v)) => status.aid = v.parse().ok(),
                _ => {}
            },
            Some(Ok(Event::EndFile(libmpv2::mpv_end_file_reason::Error))) => return Ok((progress(&status), Some(Ended::Failed))),
            Some(Ok(Event::EndFile(_) | Event::Shutdown)) => return Ok((progress(&status), None)),
            Some(Err(e)) => return Err(e),
            _ => {}
        }
        let only_position = Status { position: before.position, ..status.clone() } == before;
        if status != before && (!only_position || (status.position - sent_position).abs() >= 0.25) {
            sent_position = status.position;
            send(status.clone());
        }
    }
}

fn progress(s: &Status) -> Progress {
    Progress { position: s.position, duration: s.duration }
}

/// The file's subtitle and audio tracks.
fn tracks(mpv: &Mpv, count: i64) -> (Vec<Track>, Vec<Track>) {
    let (mut subs, mut audio) = (vec![], vec![]);
    for i in 0..count {
        let text = |field: &str| mpv.get_property::<String>(&format!("track-list/{i}/{field}")).ok().filter(|s| !s.is_empty());
        let Ok(id) = mpv.get_property::<i64>(&format!("track-list/{i}/id")) else { continue };
        let title = text("title").or_else(|| text("lang")).unwrap_or_else(|| format!("Track {id}"));
        match text("type").as_deref() {
            Some("sub") => subs.push(Track { id, title }),
            Some("audio") => audio.push(Track { id, title }),
            _ => {}
        }
    }
    (subs, audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(lang: &str, url: &str) -> Subtitle {
        Subtitle { id: url.into(), url: url.into(), lang: lang.into() }
    }

    #[test]
    fn sub_add_numbers_tracks_only_when_a_language_repeats() {
        let args = sub_add_args(&[sub("eng", "a"), sub("fre", "c"), sub("eng", "b")], "OpenSubtitles v3");
        assert_eq!(
            args,
            [
                ["a", "auto", "eng · OpenSubtitles v3 1", "eng"],
                ["c", "auto", "fre · OpenSubtitles v3", "fre"],
                ["b", "auto", "eng · OpenSubtitles v3 2", "eng"],
            ]
        );
    }
}
