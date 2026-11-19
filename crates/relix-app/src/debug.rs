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
            crate::update::version(),
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
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| "?".into())
}

/// The stream last played, for [`speed_test`].
#[cfg(not(any(target_os = "emscripten", target_os = "android")))]
pub static LAST_STREAM: std::sync::Mutex<Option<(String, std::collections::HashMap<String, String>)>> = std::sync::Mutex::new(None);

/// How fast the last played stream downloads over 1 connection, then over 4 at once (each from a
/// different part of the file): says whether downloading in parallel would help this stream.
/// Each round stops after 6 seconds or 48 MB, whichever comes first.
#[cfg(not(any(target_os = "emscripten", target_os = "android")))]
pub async fn speed_test() -> String {
    let Some((url, headers)) = LAST_STREAM.lock().unwrap().clone() else {
        return "Play something first, then come back: this tests the stream you last played.".into();
    };
    if !url.starts_with("http") {
        return "The last stream isn't an http one.".into();
    }
    // Total size, from a one-byte request.
    let size = match fetch(&url, &headers, 0, Some(0)).await {
        Ok(r) => r.headers().get("content-range").and_then(|v| v.to_str().ok()?.rsplit('/').next()?.parse::<u64>().ok()),
        Err(e) => return format!("The stream didn't answer: {e}"),
    };
    let Some(size) = size else {
        return "This stream's server doesn't allow fetching parts of the file, so parallel downloading can't work on it.".into();
    };
    let (one, four) = (mbit(&url, &headers, size, 1).await, mbit(&url, &headers, size, 4).await);
    let text = format!("1 connection: {one:.1} Mbit/s. 4 connections: {four:.1} Mbit/s ({}).", host(&url));
    crate::log!("speed test: {text}");
    text
}

/// One ranged GET on a connection of its own (a fresh client, HTTP/1.1 like mpv's).
#[cfg(not(any(target_os = "emscripten", target_os = "android")))]
async fn fetch(url: &str, headers: &std::collections::HashMap<String, String>, from: u64, to: Option<u64>) -> Result<reqwest::Response, String> {
    let client = reqwest::Client::builder().http1_only().connect_timeout(std::time::Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let mut req = client.get(url).header("Range", format!("bytes={from}-{}", to.map(|t| t.to_string()).unwrap_or_default()));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    // Without the URL: it can hold a debrid key.
    req.send().await.and_then(|r| r.error_for_status()).map_err(|e| e.without_url().to_string())
}

/// Download speed over `n` connections at once, each starting at a different part of the file.
#[cfg(not(any(target_os = "emscripten", target_os = "android")))]
async fn mbit(url: &str, headers: &std::collections::HashMap<String, String>, size: u64, n: u64) -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    const CAP: u64 = 48 << 20;
    let got = AtomicU64::new(0);
    let started = std::time::Instant::now();
    let all = futures_util::future::join_all((0..n).map(|i| {
        let got = &got;
        async move {
            let Ok(mut r) = fetch(url, headers, size / n * i, None).await else { return };
            while let Ok(Some(chunk)) = r.chunk().await {
                if got.fetch_add(chunk.len() as u64, Relaxed) > CAP {
                    return;
                }
            }
        }
    }));
    let _ = crate::net::timeout(std::time::Duration::from_secs(6), all).await;
    got.load(Relaxed) as f64 * 8. / 1e6 / started.elapsed().as_secs_f64()
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
