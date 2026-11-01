//! Debug mode: Settings > Debug mode, or `--debug`. While on, everything Relix logs also goes to
//! `debug.log` in its data folder with timestamps (release builds have no console on Windows),
//! mpv writes `mpv.log` there, and the performance overlay (Ctrl+Shift+P) is available.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static ON: AtomicBool = AtomicBool::new(false);
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
static START: Mutex<Option<Instant>> = Mutex::new(None);

fn marker() -> PathBuf {
    crate::data_dir().join("debug")
}

pub fn log_path() -> PathBuf {
    crate::data_dir().join("debug.log")
}

pub fn mpv_log_path() -> PathBuf {
    crate::data_dir().join("mpv.log")
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turns debug mode on at startup when it was left on, or `--debug` was passed.
pub fn init() {
    *START.lock().unwrap() = Some(Instant::now());
    if std::env::args().any(|a| a == "--debug") || marker().exists() {
        set(true);
        crate::log!(
            "Relix {} on {} {} ({})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
        );
    }
}

/// Switches debug mode now and for later launches.
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
    let mut file = FILE.lock().unwrap();
    if on {
        let _ = std::fs::create_dir_all(crate::data_dir());
        let _ = std::fs::write(marker(), b"");
        *file = std::fs::OpenOptions::new().create(true).append(true).open(log_path()).ok();
    } else {
        let _ = std::fs::remove_file(marker());
        *file = None;
    }
}

/// Writes a line to stderr and, in debug mode, to `debug.log` (the `log!` macro).
pub fn write(args: std::fmt::Arguments) {
    eprintln!("{args}");
    if !enabled() {
        return;
    }
    let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let (h, m, s, ms) = (wall.as_secs() / 3600 % 24, wall.as_secs() / 60 % 60, wall.as_secs() % 60, wall.subsec_millis());
    let since = START.lock().unwrap().map_or(0., |t| t.elapsed().as_secs_f64());
    if let Some(f) = FILE.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{h:02}:{m:02}:{s:02}.{ms:03} UTC  +{since:8.3}s  {args}");
    }
}

/// A URL reduced to its host: stream and addon URLs can carry debrid keys, which logs mustn't.
pub fn host(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| "?".into())
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::debug::write(format_args!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_keeps_only_the_site() {
        assert_eq!(super::host("https://torrentio.strem.fun/realdebrid=SECRETKEY/stream/x.json"), "torrentio.strem.fun");
        assert_eq!(super::host("not a url"), "?");
    }
}
