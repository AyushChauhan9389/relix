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
pub fn play(url: &str, headers: &HashMap<String, String>, start: f64, subs: Receiver<Vec<[String; 4]>>) -> libmpv2::Result<Progress> {
    let mpv = Mpv::with_initializer(|init| {
        init.set_option("vo", "gpu-next")?;
        init.set_option("hwdec", "auto-safe")?;
        init.set_option("force-window", "immediate")?;
        init.set_option("keep-open", "no")?;
        init.set_option("osc", "yes")?;
        init.set_option("input-default-bindings", "yes")?;
        init.set_option("input-vo-keyboard", "yes")?;
        init.set_option("title", "Relix")?;
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
            Some(Ok(Event::EndFile(_) | Event::Shutdown)) => return Ok(progress),
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
