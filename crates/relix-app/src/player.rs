//! Playback in mpv's own window (ADR 0002).

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use libmpv2::{Format, Mpv, events::Event, events::PropertyData};
pub use relix_proto::addon::Subtitle;

/// Where playback stopped, for Continue Watching.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Progress {
    pub position: f64,
    pub duration: f64,
}

/// How playback ended, beyond the file simply finishing or the window closing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ended {
    /// The "Next episode" button in the player controls.
    NextRequested,
}

/// What a `script-message` from the player's Lua controls means for playback.
pub fn client_message(args: &[&str]) -> Option<Ended> {
    match args {
        ["relix-next", ..] => Some(Ended::NextRequested),
        _ => None,
    }
}

/// What the player shows around the video.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    pub title: String,
    /// e.g. "S1:E2 Cat's in the Bag..."; empty for movies.
    pub episode: String,
    /// Shows "Next episode" and the countdown card.
    pub has_next: bool,
}

/// The Netflix-style controls (player.lua), embedded so there's nothing extra to ship.
const SKIN: &str = include_str!("player.lua");

/// mpv `script-opts` for the skin; `%len%` quoting keeps commas and `=` in titles intact.
fn skin_opts(o: &Overlay) -> String {
    let q = |v: &str| format!("%{}%{v}", v.len());
    format!("relix-title={},relix-episode={},relix-has_next={}", q(&o.title), q(&o.episode), if o.has_next { "yes" } else { "no" })
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

/// Plays `url` from `start` seconds in a new mpv window and blocks until it closes or the file ends.
/// `headers` are an addon stream's `proxyHeaders.request`. Subtitle tracks arriving on `subs`
/// (as `sub-add` arguments) are attached once the file has loaded, so they never delay playback.
/// `overlay` fills in the player controls.
pub fn play(
    url: &str,
    headers: &HashMap<String, String>,
    start: f64,
    subs: Receiver<Vec<[String; 4]>>,
    overlay: &Overlay,
) -> libmpv2::Result<(Progress, Option<Ended>)> {
    let skin = std::env::temp_dir().join("relix-player.lua");
    if std::fs::read_to_string(&skin).ok().as_deref() != Some(SKIN)
        && let Err(e) = std::fs::write(&skin, SKIN)
    {
        eprintln!("writing player controls: {e}");
    }
    let mpv = Mpv::with_initializer(|init| {
        init.set_option("vo", "gpu-next")?;
        init.set_option("hwdec", "auto-safe")?;
        init.set_option("force-window", "immediate")?;
        init.set_option("keep-open", "no")?;
        if let Ok(log) = std::env::var("RELIX_MPV_LOG") {
            init.set_option("log-file", log.as_str())?;
        }
        // Netflix-style: fullscreen, our controls instead of mpv's.
        init.set_option("fullscreen", "yes")?;
        init.set_option("osc", "no")?;
        init.set_option("osd-bar", "no")?;
        init.set_option("scripts", skin.to_string_lossy().as_ref())?;
        init.set_option("script-opts", skin_opts(overlay).as_str())?;
        init.set_option("input-default-bindings", "yes")?;
        init.set_option("input-vo-keyboard", "yes")?;
        init.set_option("title", if overlay.title.is_empty() { "Relix".to_owned() } else { format!("{} · Relix", overlay.title) }.as_str())?;
        if start > 0. {
            init.set_option("start", start.to_string().as_str())?;
        }
        if !headers.is_empty() {
            // ponytail: mpv splits this list on ',', so header values containing commas break; escape when an addon needs it.
            let fields: Vec<String> = headers.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            init.set_option("http-header-fields", fields.join(",").as_str())?;
        }
        Ok(())
    })?;

    mpv.observe_property("time-pos", Format::Double, 0)?;
    mpv.observe_property("duration", Format::Double, 1)?;
    mpv.command("loadfile", &[url])?;

    let mut progress = Progress::default();
    let mut loaded = false;
    loop {
        if loaded {
            for args in subs.try_iter().flatten() {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                if let Err(e) = mpv.command("sub-add", &args) {
                    eprintln!("sub-add {}: {e}", args[0]);
                }
            }
        }
        // Wakes now and then to pick up subtitles that arrived meanwhile.
        match mpv.wait_event(0.25) {
            Some(Ok(Event::FileLoaded)) => loaded = true,
            Some(Ok(Event::PropertyChange { change: PropertyData::Double(v), reply_userdata, .. })) => {
                match reply_userdata {
                    0 => progress.position = v,
                    _ => progress.duration = v,
                }
            }
            Some(Ok(Event::ClientMessage(args))) => {
                if let Some(ended) = client_message(&args) {
                    return Ok((progress, Some(ended)));
                }
            }
            Some(Ok(Event::EndFile(_) | Event::Shutdown)) => return Ok((progress, None)),
            Some(Err(e)) => return Err(e),
            _ => {}
        }
    }
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

#[cfg(test)]
mod next_tests {
    use super::*;

    #[test]
    fn the_next_button_ends_playback_asking_for_the_next_episode() {
        assert_eq!(client_message(&["relix-next"]), Some(Ended::NextRequested));
        assert_eq!(client_message(&["something-else"]), None);
        assert_eq!(client_message(&[]), None);
    }
}
