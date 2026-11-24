//! Offline downloads (Windows and Linux; Android next), a few at a time: a stream saved whole to a folder, resumed
//! after a dropped connection, a restart or an expired debrid link, and played from there. The
//! list lives on this device only (the store's meta table), never synced: files don't travel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use relix_proto::addon::Meta;
use serde::{Deserialize, Serialize};

use crate::Ctx;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Running,
    Paused,
    Failed(String),
    Done,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Download {
    /// The movie's or the episode's id: one download each.
    pub video: String,
    /// The title's details, so its page opens offline.
    pub meta: Meta,
    /// "S1:E2 Cat's in the Bag..."; empty for movies.
    pub episode: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    /// What finds the stream again once its link expires: the addon (manifest id), the stream's
    /// name and its file name.
    pub addon: String,
    pub name: Option<String>,
    pub filename: Option<String>,
    pub binge_group: Option<String>,
    pub file: PathBuf,
    /// Bytes in all (once the server says) and so far.
    pub size: Option<u64>,
    pub done: u64,
    pub status: Status,
}

/// Platforms with downloads; Android needs a background service first, the web a place to keep files.
pub fn supported() -> bool {
    cfg!(not(any(target_os = "emscripten", target_os = "android")))
}

const KEY: &str = "downloads";
const FOLDER_KEY: &str = "download_folder";

pub fn load(store: &crate::store::Store) -> Vec<Download> {
    store.meta(KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(ctx: &Ctx) {
    let json = serde_json::to_string(&*ctx.downloads.peek()).unwrap_or_default();
    if let Err(e) = ctx.store.set_meta(KEY, Some(&json)) {
        crate::log!("saving downloads: {e}");
    }
}

/// Where new downloads go: the folder chosen in Settings, or Videos/Relix in the home folder.
pub fn folder(store: &crate::store::Store) -> PathBuf {
    store.meta(FOLDER_KEY).ok().flatten().filter(|f| !f.trim().is_empty()).map(PathBuf::from).unwrap_or_else(default_folder)
}

pub fn set_folder(store: &crate::store::Store, folder: &str) {
    let folder = folder.trim();
    let value = (!folder.is_empty() && Path::new(folder) != default_folder()).then_some(folder);
    if let Err(e) = store.set_meta(FOLDER_KEY, value) {
        crate::log!("saving download folder: {e}");
    }
}

pub fn default_folder() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    home.join("Videos").join("Relix")
}

/// The finished download of `video`, if its file is still there.
pub fn done(ctx: &Ctx, video: &str) -> Option<Download> {
    ctx.downloads.peek().iter().find(|d| d.video == video && d.status == Status::Done && d.file.exists()).cloned()
}

/// A downloaded title's details, for opening its page offline.
pub fn meta_of(ctx: &Ctx, id: &str) -> Option<Meta> {
    ctx.downloads.peek().iter().find(|d| d.meta.id == id).map(|d| d.meta.clone())
}

/// What a new download needs from the stream the viewer picked.
pub struct Request {
    pub meta: Meta,
    pub video: String,
    pub episode: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub addon: String,
    pub name: Option<String>,
    pub filename: Option<String>,
    pub binge_group: Option<String>,
}

/// Queues `req`'s stream; it starts once the ones before it finish.
pub fn add(ctx: &Ctx, req: Request) {
    let mut list = ctx.downloads;
    let taken: Vec<PathBuf> = list.peek().iter().map(|d| d.file.clone()).collect();
    let file = file_path(&folder(&ctx.store), &req.meta.name, &req.episode, req.filename.as_deref(), &req.url, &taken);
    list.write().retain(|d| d.video != req.video);
    list.write().push(Download {
        video: req.video,
        meta: req.meta,
        episode: req.episode,
        url: req.url,
        headers: req.headers,
        addon: req.addon,
        name: req.name,
        filename: req.filename,
        binge_group: req.binge_group,
        file,
        size: None,
        done: 0,
        status: Status::Queued,
    });
    save(ctx);
    ctx.downloads_wake.notify_one();
}

pub fn pause(ctx: &Ctx, video: &str) {
    stop_if_running(video);
    set_status(ctx, video, Status::Paused);
}

pub fn resume(ctx: &Ctx, video: &str) {
    set_status(ctx, video, Status::Queued);
    ctx.downloads_wake.notify_one();
}

/// Stops it if it's running and deletes what it saved.
pub fn remove(ctx: &Ctx, video: &str) {
    stop_if_running(video);
    let mut list = ctx.downloads;
    let gone: Vec<Download> = list.peek().iter().filter(|d| d.video == video).cloned().collect();
    list.write().retain(|d| d.video != video);
    save(ctx);
    for d in gone {
        // The running transfer lets go of the partial file as it stops; deleting may wait for that.
        let _ = std::fs::remove_file(&d.file);
        let _ = std::fs::remove_file(part_path(&d.file));
    }
}

fn set_status(ctx: &Ctx, video: &str, status: Status) {
    let mut list = ctx.downloads;
    if let Some(d) = list.write().iter_mut().find(|d| d.video == video) {
        d.status = status;
    }
    save(ctx);
}

/// The running downloads' stop switches, by video id.
#[cfg(not(target_os = "emscripten"))]
static RUNNING: std::sync::Mutex<Vec<(String, std::sync::Arc<std::sync::atomic::AtomicBool>)>> = std::sync::Mutex::new(Vec::new());

fn stop_if_running(video: &str) {
    #[cfg(not(target_os = "emscripten"))]
    if let Some((_, stop)) = RUNNING.lock().unwrap().iter().find(|(v, _)| v == video) {
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(target_os = "emscripten")]
    let _ = video;
}

fn part_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    file.with_file_name(name)
}

/// "Title - S1:E2 Episode.mkv" in `folder`: characters file systems refuse become spaces, and a
/// number is added when the name is taken.
fn file_path(folder: &Path, title: &str, episode: &str, filename: Option<&str>, url: &str, taken: &[PathBuf]) -> PathBuf {
    const VIDEO: [&str; 8] = ["mkv", "mp4", "m4v", "webm", "avi", "mov", "ts", "m2ts"];
    let ext_of = |s: &str| s.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).filter(|e| VIDEO.contains(&e.as_str()));
    let url_path = url::Url::parse(url).ok().map(|u| u.path().to_owned()).unwrap_or_default();
    let ext = filename.and_then(ext_of).or_else(|| ext_of(&url_path)).unwrap_or_else(|| "mkv".into());
    let name = if episode.is_empty() { title.to_owned() } else { format!("{title} - {episode}") };
    let clean: String = name.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = clean.trim_end_matches('.');
    let clean: String = if clean.is_empty() { "Video".into() } else { clean.chars().take(150).collect() };
    (1..)
        .map(|n| folder.join(if n == 1 { format!("{clean}.{ext}") } else { format!("{clean} ({n}).{ext}") }))
        .find(|p| !p.exists() && !part_path(p).exists() && !taken.contains(p))
        .unwrap()
}

/// "1.2 GB", "340 MB".
pub fn size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1. { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// The most downloads at once, however fast the connection.
const MAX_AT_ONCE: usize = 4;

/// How many downloads to run at once, decided again every few seconds from the total speed:
/// one more is tried, and kept only if the total rose by a fifth (the connection had room);
/// otherwise the count goes back and stays (`settled`) until the queue empties.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pace {
    limit: usize,
    /// The speed before the count was last raised (bytes/s), while that raise is on trial.
    trial: Option<f64>,
    settled: bool,
}

impl Pace {
    const START: Pace = Pace { limit: 2, trial: None, settled: false };

    fn next(self, speed: f64) -> Pace {
        match self.trial {
            Some(before) if speed > before * 1.2 => Pace { trial: None, ..self },
            Some(_) => Pace { limit: self.limit - 1, trial: None, settled: true },
            None if !self.settled && self.limit < MAX_AT_ONCE => Pace { limit: self.limit + 1, trial: Some(speed), ..self },
            None => self,
        }
    }
}

/// Runs the queue for as long as Relix is open, a few downloads at a time ([`Pace`]).
#[cfg(not(target_os = "emscripten"))]
pub async fn run(ctx: Ctx) {
    use freya::prelude::spawn;
    let mut list = ctx.downloads;
    // Running when Relix closed: they pick up where they stopped.
    for d in list.write().iter_mut().filter(|d| d.status == Status::Running) {
        d.status = Status::Queued;
    }
    let mut pace = Pace::START;
    let total = |list: &Vec<Download>| list.iter().filter(|d| d.status == Status::Running).map(|d| d.done).sum::<u64>();
    loop {
        let (running, queued) = {
            let list = list.peek();
            (list.iter().filter(|d| d.status == Status::Running).count(), list.iter().find(|d| d.status == Status::Queued).cloned())
        };
        if let (Some(d), true) = (queued.clone(), running < pace.limit) {
            set_status(&ctx, &d.video, Status::Running);
            let ctx = ctx.clone();
            spawn(async move {
                let outcome = transfer(&ctx, &d).await;
                RUNNING.lock().unwrap().retain(|(v, _)| *v != d.video);
                // Paused or removed meanwhile: that already says what it is now.
                if ctx.downloads.peek().iter().any(|x| x.video == d.video && x.status == Status::Running) {
                    set_status(&ctx, &d.video, outcome);
                }
                ctx.downloads_wake.notify_one();
            });
            continue;
        }
        if running == 0 {
            pace = Pace::START;
            ctx.downloads_wake.notified().await;
            continue;
        }
        // Full, or nothing waiting: until one ends or is added, or 8 seconds of speed are in.
        let (before, started) = (total(&list.peek()), std::time::Instant::now());
        let woken = crate::net::timeout(std::time::Duration::from_secs(8), ctx.downloads_wake.notified()).await.is_ok();
        // Only a full, undisturbed stretch with more waiting says whether another would help.
        if !woken && queued.is_some() && running == pace.limit {
            let speed = total(&list.peek()).saturating_sub(before) as f64 / started.elapsed().as_secs_f64();
            let next = pace.next(speed);
            if next != pace {
                crate::log!("download: {:.1} Mbit/s with {} at once; now {}", speed * 8. / 1e6, pace.limit, next.limit);
            }
            pace = next;
        }
    }
}

#[cfg(not(target_os = "emscripten"))]
enum Fail {
    Stopped,
    /// The server turned the link down (4xx): usually an expired debrid link.
    Refused(u16),
    Network(String),
    Disk(String),
}

/// Downloads `d` to its end, trying again after network trouble and finding the stream again
/// when its link has expired. Returns what it ended as.
#[cfg(not(target_os = "emscripten"))]
async fn transfer(ctx: &Ctx, d: &Download) -> Status {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    let stop = Arc::new(AtomicBool::new(false));
    RUNNING.lock().unwrap().push((d.video.clone(), stop.clone()));
    if let Some(dir) = d.file.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return Status::Failed(format!("Couldn't create {}: {e}", dir.display()));
    }
    let part = part_path(&d.file);
    let (mut url, mut headers) = (d.url.clone(), d.headers.clone());
    let (mut tries, mut found_again) = (0, false);
    loop {
        let from = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let job = tokio::spawn(fetch(url.clone(), headers.clone(), part.clone(), from, stop.clone(), tx));
        let mut saved_at = std::time::Instant::now();
        let mut moved = false;
        while let Some((done, size)) = rx.recv().await {
            moved |= done > from;
            let mut list = ctx.downloads;
            if let Some(x) = list.write().iter_mut().find(|x| x.video == d.video) {
                x.done = done;
                x.size = size.or(x.size);
            }
            if saved_at.elapsed().as_secs() >= 5 {
                saved_at = std::time::Instant::now();
                save(ctx);
            }
        }
        let result = job.await.unwrap_or_else(|e| Err(Fail::Network(e.to_string())));
        if moved {
            tries = 0;
        }
        match result {
            Ok(()) => {
                return match std::fs::rename(&part, &d.file) {
                    Ok(()) => {
                        crate::log!("download: finished {}", d.file.display());
                        Status::Done
                    }
                    Err(e) => Status::Failed(format!("Couldn't finish the file: {e}")),
                };
            }
            Err(Fail::Stopped) => return Status::Paused,
            Err(Fail::Disk(e)) => return Status::Failed(format!("Couldn't write the file: {e}")),
            Err(Fail::Refused(code)) if !found_again => {
                found_again = true;
                crate::log!("download: the link was refused ({code}); asking the addon again");
                match find_again(ctx, d).await {
                    Some((u, h)) => {
                        (url, headers) = (u.clone(), h.clone());
                        let mut list = ctx.downloads;
                        if let Some(x) = list.write().iter_mut().find(|x| x.video == d.video) {
                            (x.url, x.headers) = (u, h);
                        }
                        save(ctx);
                    }
                    None => return Status::Failed("The link expired and the addon no longer lists this stream. Download another one.".into()),
                }
            }
            Err(Fail::Refused(code)) => return Status::Failed(format!("The server refused the download ({code}). Download another stream.")),
            Err(Fail::Network(e)) => {
                tries += 1;
                crate::log!("download: {e} (try {tries})");
                if tries > 5 {
                    return Status::Failed(format!("The connection kept failing: {e}. Resume to try again."));
                }
                for _ in 0..tries * 5 {
                    crate::net::sleep(std::time::Duration::from_secs(1)).await;
                    if stop.load(Relaxed) {
                        return Status::Paused;
                    }
                }
            }
        }
    }
}

/// The same stream from the same addon, with a fresh link.
#[cfg(not(target_os = "emscripten"))]
async fn find_again(ctx: &Ctx, d: &Download) -> Option<(String, HashMap<String, String>)> {
    use relix_proto::addon::{ResourceRequest, StreamsResponse};
    let addon = ctx.all_addons().into_iter().find(|a| a.manifest.id == d.addon)?;
    let req = ResourceRequest::new("stream", &d.meta.r#type, &d.video);
    let found = crate::get::<StreamsResponse>(ctx.http.clone(), req.url(&addon)).await.ok()?;
    let same = |s: &&relix_proto::addon::Stream| match &d.filename {
        Some(f) => s.behavior_hints.filename.as_ref() == Some(f),
        None => s.name.is_some() && s.name == d.name,
    };
    let s = found.streams.iter().find(same)?;
    Some((s.playable_url()?.to_owned(), s.behavior_hints.proxy_headers.clone().map(|h| h.request).unwrap_or_default()))
}

/// One request from byte `from` onwards into `part`, reporting (bytes so far, total) as it goes.
#[cfg(not(target_os = "emscripten"))]
async fn fetch(
    url: String,
    headers: HashMap<String, String>,
    part: PathBuf,
    from: u64,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress: tokio::sync::mpsc::UnboundedSender<(u64, Option<u64>)>,
) -> Result<(), Fail> {
    use std::sync::atomic::Ordering::Relaxed;
    use tokio::io::AsyncWriteExt;
    let mut req = crate::http_client_native().get(&url);
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    if from > 0 {
        req = req.header("range", format!("bytes={from}-"));
    }
    // Errors never carry the URL: it can hold a debrid key.
    let mut res = req.send().await.map_err(|e| Fail::Network(e.without_url().to_string()))?;
    let status = res.status().as_u16();
    // Asked for the rest of a file that's already whole.
    if status == 416 && from > 0 {
        return Ok(());
    }
    if (400..500).contains(&status) {
        return Err(Fail::Refused(status));
    }
    if status >= 500 {
        return Err(Fail::Network(format!("server error {status}")));
    }
    // 206: the rest, as asked; 200: the whole file (a server that can't resume starts over).
    let resumed = status == 206;
    let total = if resumed {
        res.headers().get("content-range").and_then(|v| v.to_str().ok()?.rsplit('/').next()?.parse().ok())
    } else {
        res.content_length()
    };
    let mut done = if resumed { from } else { 0 };
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .await
        .map_err(|e| Fail::Disk(e.to_string()))?;
    let mut file = tokio::io::BufWriter::with_capacity(1 << 20, file);
    let _ = progress.send((done, total));
    let mut sent = std::time::Instant::now();
    loop {
        if stop.load(Relaxed) {
            let _ = file.flush().await;
            return Err(Fail::Stopped);
        }
        let chunk = match res.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) => {
                let _ = file.flush().await;
                return Err(Fail::Network(e.without_url().to_string()));
            }
        };
        file.write_all(&chunk).await.map_err(|e| Fail::Disk(e.to_string()))?;
        done += chunk.len() as u64;
        if sent.elapsed().as_millis() >= 300 {
            sent = std::time::Instant::now();
            let _ = progress.send((done, total));
        }
    }
    file.flush().await.map_err(|e| Fail::Disk(e.to_string()))?;
    let _ = progress.send((done, total));
    if total.is_some_and(|t| done < t) {
        return Err(Fail::Network("the connection ended early".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_clean_and_never_collide() {
        let dir = std::env::temp_dir().join("relix-download-names");
        let _ = std::fs::remove_dir_all(&dir);
        let name = |title, ep, hint: Option<&str>, url, taken: &[PathBuf]| file_path(&dir, title, ep, hint, url, taken).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name("Breaking Bad", "S1:E2 Cat's in the Bag...", Some("BB.S01E02.1080p.mkv"), "https://x/y", &[]), "Breaking Bad - S1 E2 Cat's in the Bag.mkv");
        assert_eq!(name("Up", "", None, "https://x/dl/abc/Up.2009.MP4?token=1", &[]), "Up.mp4");
        // Neither the hint nor the link says: Matroska.
        assert_eq!(name("Up", "", Some("Up 2009"), "https://x/stream", &[]), "Up.mkv");
        let taken = [dir.join("Up.mkv")];
        assert_eq!(name("Up", "", None, "https://x/stream", &taken), "Up (2).mkv");
    }

    /// A one-file HTTP server: honours Range unless `ranges` is off, and cuts the first answer
    /// after `cut` bytes when set. Returns its address.
    fn serve(body: Vec<u8>, ranges: bool, cut: Option<usize>) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}/film.mkv", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut cut = cut;
            for conn in listener.incoming() {
                let mut conn = conn.unwrap();
                let mut from = 0;
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(r) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = r.trim().trim_end_matches('-').parse().unwrap();
                    }
                }
                let from = if ranges { from } else { 0 };
                if from >= body.len() && from > 0 {
                    let _ = conn.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\ncontent-length: 0\r\n\r\n");
                    continue;
                }
                let rest = &body[from..];
                let head = if from > 0 {
                    format!("HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {from}-{}/{}\r\n\r\n", rest.len(), body.len() - 1, body.len())
                } else {
                    format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", rest.len())
                };
                let _ = conn.write_all(head.as_bytes());
                let send = cut.take().map_or(rest.len(), |c| c.min(rest.len()));
                let _ = conn.write_all(&rest[..send]);
            }
        });
        addr
    }

    fn get(url: &str, part: &Path, from: u64) -> Result<(), Fail> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        rt.block_on(fetch(url.into(), HashMap::new(), part.into(), from, Default::default(), tx))
    }

    #[test]
    fn a_cut_download_resumes_where_it_stopped() {
        let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let dir = std::env::temp_dir().join(format!("relix-dl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let part = dir.join("film.mkv.part");
        let _ = std::fs::remove_file(&part);
        let url = serve(body.clone(), true, Some(1_000_000));
        // The first answer stops a third of the way in: kept, and reported as a network failure.
        assert!(matches!(get(&url, &part, 0), Err(Fail::Network(_))));
        let kept = std::fs::metadata(&part).unwrap().len();
        assert_eq!(kept, 1_000_000);
        // The next request asks for the rest and appends it.
        assert!(get(&url, &part, kept).is_ok());
        assert_eq!(std::fs::read(&part).unwrap(), body);
        // Already whole: the server's 416 counts as done.
        assert!(get(&url, &part, body.len() as u64).is_ok());
    }

    #[test]
    fn a_server_without_ranges_starts_over() {
        let body: Vec<u8> = (0..500_000u32).map(|i| (i % 13) as u8).collect();
        let dir = std::env::temp_dir().join(format!("relix-dl-norange-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let part = dir.join("film.mkv.part");
        std::fs::write(&part, vec![9u8; 1234]).unwrap();
        let url = serve(body.clone(), false, None);
        assert!(get(&url, &part, 1234).is_ok());
        assert_eq!(std::fs::read(&part).unwrap(), body);
    }

    #[test]
    fn more_at_once_only_while_the_total_speed_rises() {
        let start = Pace::START;
        // Two give 2 MB/s: a third is tried. It brings 3 MB/s: kept, and a fourth is tried.
        let three = start.next(2e6);
        assert_eq!((three.limit, three.trial), (3, Some(2e6)));
        let kept = three.next(3e6);
        assert_eq!((kept.limit, kept.trial, kept.settled), (3, None, false));
        let four = kept.next(3e6);
        assert_eq!(four.limit, 4);
        // The fourth adds nothing (the line is full): back to three, for good.
        let back = four.next(3.1e6);
        assert_eq!((back.limit, back.settled), (3, true));
        assert_eq!(back.next(3e6), back);
        // Never past the most allowed.
        let top = Pace { limit: MAX_AT_ONCE, trial: None, settled: false };
        assert_eq!(top.next(9e9), top);
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size_text(4_520_000_000), "4.5 GB");
        assert_eq!(size_text(340_000_000), "340 MB");
    }
}
