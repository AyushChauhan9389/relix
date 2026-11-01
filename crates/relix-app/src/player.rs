//! Playback with mpv; Relix draws the video (video.rs) and the controls (player_ui.rs). ADR 0002.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use libmpv2::{Format, Mpv, events::Event, events::PropertyData};
pub use relix_proto::addon::Subtitle;

/// Where playback stopped, for Continue Watching.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Progress {
    pub position: f64,
    pub duration: f64,
}

/// How playback ended, beyond the file simply finishing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ended {
    /// The viewer closed the player (Esc, Back): never auto-play the next episode.
    Stopped,
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
    /// Art for the loading screen.
    pub logo: Option<String>,
    pub background: Option<String>,
}

/// A subtitle or audio track mpv knows about.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: i64,
    pub title: String,
    /// ISO 639 code, when the track has one.
    pub lang: Option<String>,
}

/// How the viewer likes playback, applied when a stream starts. Picking tracks and subtitle
/// settings in the player update it; it's a synced setting.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Audio language (ISO 639-2, e.g. "eng"); `None` keeps the stream's default.
    pub audio_lang: Option<String>,
    /// Subtitle language, "off", or `None` for the stream's default.
    pub sub_lang: Option<String>,
    pub sub_scale: f64,
    /// A dark box behind subtitles.
    pub sub_background: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { audio_lang: None, sub_lang: None, sub_scale: 1., sub_background: false }
    }
}

impl Prefs {
    /// The subtitle background as mpv options.
    pub fn border_style(&self) -> &'static str {
        if self.sub_background { "background-box" } else { "outline-and-shadow" }
    }
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
            let name = lang_name(&s.lang);
            let title = if total > 1 { format!("{name} · {addon} {n}") } else { format!("{name} · {addon}") };
            [s.url.clone(), "auto".into(), title, s.lang.clone()]
        })
        .collect()
}

/// A subtitle language code (ISO 639-2, as addons send them) as a name people read.
fn lang_name(code: &str) -> String {
    // shortcut: the common languages only, others show their code; add names as people ask.
    let name = match code {
        "eng" => "English", "spa" => "Spanish", "fre" | "fra" => "French", "ger" | "deu" => "German",
        "ita" => "Italian", "por" => "Portuguese", "pob" => "Portuguese (Brazil)", "dut" | "nld" => "Dutch",
        "pol" => "Polish", "rus" => "Russian", "tur" => "Turkish", "ara" => "Arabic", "hin" => "Hindi",
        "jpn" => "Japanese", "kor" => "Korean", "chi" | "zho" => "Chinese", "swe" => "Swedish",
        "dan" => "Danish", "nor" => "Norwegian", "fin" => "Finnish", "est" => "Estonian", "gre" | "ell" => "Greek",
        "heb" => "Hebrew", "hun" => "Hungarian", "cze" | "ces" => "Czech", "rum" | "ron" => "Romanian",
        "ukr" => "Ukrainian", "vie" => "Vietnamese", "ind" => "Indonesian", "tha" => "Thai",
        _ => return code.to_uppercase(),
    };
    name.into()
}

/// Gives the UI the player and a sender for "render context ready".
pub type HandOver = Box<dyn FnOnce(Arc<Mpv>, Sender<()>) + Send>;

/// Windows loads libmpv on first use, and a missing DLL ends the whole process there: check first.
#[cfg(windows)]
pub fn library_missing() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
    }
    let name: Vec<u16> = "libmpv-2.dll\0".encode_utf16().collect();
    unsafe { LoadLibraryW(name.as_ptr()) }.is_null()
}

#[cfg(not(windows))]
pub fn library_missing() -> bool {
    false
}

/// Plays `url` from `start` seconds and blocks until playback stops or the file ends.
///
/// With `video`, mpv renders for Relix to draw in its own window (video.rs): it's handed the
/// player as soon as it exists, with a sender to signal once its render context is up (mpv's
/// video output needs it before the file opens, or the file plays as audio only). Without it (the `--play` check) mpv opens its own window with its
/// own controls. `headers` are an addon stream's `proxyHeaders.request`. `inbox` brings subtitles
/// (attached once the file loads, so they never delay playback), stop and next requests; `status`
/// gets the player's state whenever it changes.
// Each argument is a separate input of one playback; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
pub fn play(
    url: &str,
    headers: &HashMap<String, String>,
    start: f64,
    title: &str,
    prefs: &Prefs,
    video: Option<HandOver>,
    inbox: Receiver<ToPlayer>,
    status: impl Fn(Status),
) -> libmpv2::Result<(Progress, Option<Ended>)> {
    let embedded = video.is_some();
    let mpv = Arc::new(Mpv::with_initializer(|init| {
        init.set_option("hwdec", "auto-safe")?;
        if let Some(lang) = &prefs.audio_lang {
            init.set_option("alang", lang.as_str())?;
        }
        match prefs.sub_lang.as_deref() {
            Some("off") => init.set_option("sid", "no")?,
            Some(lang) => init.set_option("slang", lang)?,
            None => {}
        }
        init.set_option("sub-scale", prefs.sub_scale)?;
        init.set_option("sub-border-style", prefs.border_style())?;
        init.set_option("sub-back-color", "#B0000000")?;
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
        let (ready, ready_rx) = std::sync::mpsc::channel();
        hand_over(mpv.clone(), ready);
        // A minimized window may not draw for a while; video.rs reloads the video once it does.
        let _ = ready_rx.recv_timeout(Duration::from_secs(3));
    }

    let played = run(&mpv, url, prefs, &inbox, &status);
    // Make sure the core stops even if something else still holds it.
    let _ = mpv.command("quit", &[]);
    played
}

fn run(mpv: &Mpv, url: &str, prefs: &Prefs, inbox: &Receiver<ToPlayer>, send: &impl Fn(Status)) -> libmpv2::Result<(Progress, Option<Ended>)> {
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
    // The first addon subtitle in the preferred language is selected, unless one already is.
    let mut want_sub = prefs.sub_lang.clone().filter(|l| l != "off");
    loop {
        for msg in inbox.try_iter() {
            match msg {
                ToPlayer::Subtitles(subs) => pending.extend(subs),
                ToPlayer::Stop => return Ok((progress(&status), Some(Ended::Stopped))),
                ToPlayer::Next => return Ok((progress(&status), Some(Ended::NextRequested))),
            }
        }
        if status.loaded {
            for mut args in pending.drain(..) {
                if status.sid.is_none() && want_sub.as_deref() == Some(args[3].as_str()) {
                    args[1] = "select".into();
                    want_sub = None;
                }
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                if let Err(e) = mpv.command("sub-add", &args) {
                    eprintln!("sub-add {}: {e}", args[0]);
                }
            }
        }
        // Wakes often enough that Stop and Next feel instant, and picks up subtitles meanwhile.
        let before = status.clone();
        match mpv.wait_event(0.05) {
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
            // A playlist link expanding into its entries; playback goes on with them.
            Some(Ok(Event::EndFile(libmpv2::mpv_end_file_reason::Redirect))) => {}
            Some(Ok(Event::EndFile(_) | Event::Shutdown)) => return Ok((progress(&status), None)),
            // libmpv2 reports an errored end of file this way. Once playing, keep the progress.
            Some(Err(_)) if status.loaded => return Ok((progress(&status), Some(Ended::Failed))),
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
            Some("sub") => subs.push(Track { id, title, lang: text("lang") }),
            Some("audio") => audio.push(Track { id, title, lang: text("lang") }),
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
                ["a", "auto", "English · OpenSubtitles v3 1", "eng"],
                ["c", "auto", "French · OpenSubtitles v3", "fre"],
                ["b", "auto", "English · OpenSubtitles v3 2", "eng"],
            ]
        );
    }
}
