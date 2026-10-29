//! Playback in mpv's own window (ADR 0002).

use std::collections::HashMap;

use libmpv2::{Format, Mpv, events::Event, events::PropertyData};

/// Where playback stopped, for Continue Watching.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Progress {
    pub position: f64,
    pub duration: f64,
}

/// Plays `url` in a new mpv window and blocks until it closes or the file ends.
/// `headers` are an addon stream's `proxyHeaders.request`.
pub fn play(url: &str, headers: &HashMap<String, String>) -> libmpv2::Result<Progress> {
    let mpv = Mpv::with_initializer(|init| {
        init.set_option("vo", "gpu-next")?;
        init.set_option("hwdec", "auto-safe")?;
        init.set_option("force-window", "immediate")?;
        init.set_option("keep-open", "no")?;
        init.set_option("osc", "yes")?;
        init.set_option("input-default-bindings", "yes")?;
        init.set_option("input-vo-keyboard", "yes")?;
        init.set_option("title", "Relix")?;
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
    loop {
        match mpv.wait_event(-1.) {
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
