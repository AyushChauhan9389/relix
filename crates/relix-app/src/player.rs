//! Playback with mpv; Relix draws the video (video.rs) and the controls (player_ui.rs). ADR 0002.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

#[cfg(not(target_os = "emscripten"))]
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
    /// "Switch to a smaller version": the download speed it measured while stalled, in bytes/s (0 unknown).
    TooSlow(f64),
}

/// What the app sends a running player.
pub enum ToPlayer {
    /// `sub-add` arguments for one addon's subtitles.
    Subtitles(Vec<[String; 4]>),
    Stop,
    Next,
    /// "Switch to a smaller version".
    Smaller,
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
    /// The stream's link: its subtitle delay is remembered under it.
    pub stream: String,
    /// The episode's video id ("tt0903747:1:2") for shared intro times; empty to not share.
    pub video_id: String,
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
    /// Stalled waiting for the network, or seeking: the controls show a spinner.
    pub buffering: bool,
    pub seeking: bool,
    /// Intros (start, end) from the file's chapters, for the Skip Intro button.
    pub intros: Vec<(f64, f64)>,
    /// Where the end credits start, from the chapters: Next Episode shows from there, and
    /// stopping past it counts as finished.
    pub credits: Option<f64>,
    /// Stalled 3 times since it started: the controls offer a smaller version.
    pub slow: bool,
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
            buffering: false,
            seeking: false,
            intros: vec![],
            credits: None,
            slow: false,
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

#[cfg(not(target_os = "emscripten"))]
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

/// Hands Android's JVM to FFmpeg, which needs it for MediaCodec (hardware decoding) and mpv's
/// AudioTrack output. Without it video decodes in software.
#[cfg(target_os = "android")]
pub fn android_init(vm: *mut std::ffi::c_void) {
    use std::ffi::{c_char, c_void};
    unsafe extern "C" {
        fn dlopen(file: *const c_char, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    type SetVm = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
    const RTLD_NOW: i32 = 2;
    // mpv-android's libmpv ships FFmpeg as its own libraries; other builds link it into libmpv.
    for lib in [c"libavcodec.so", c"libmpv.so"] {
        unsafe {
            let handle = dlopen(lib.as_ptr(), RTLD_NOW);
            let set = if handle.is_null() { handle } else { dlsym(handle, c"av_jni_set_java_vm".as_ptr()) };
            if !set.is_null() {
                std::mem::transmute::<*mut c_void, SetVm>(set)(vm, std::ptr::null_mut());
                return;
            }
        }
    }
    crate::log!("player: av_jni_set_java_vm not found, decoding in software");
}

/// Mozilla's certificate roots (the ones Relix's own requests use on Android) as a PEM file for mpv.
#[cfg(target_os = "android")]
fn android_roots() -> std::io::Result<std::path::PathBuf> {
    use base64::Engine;
    let path = crate::data_dir().join("roots.pem");
    let mut pem = String::new();
    for cert in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        pem.push_str("-----BEGIN CERTIFICATE-----\n");
        // 64 to a line: PEM readers refuse long ones.
        for line in base64::engine::general_purpose::STANDARD.encode(cert).as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(line).unwrap_or_default());
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
    }
    // Written each time: an update may bring new roots, and it's a fifth of a megabyte.
    std::fs::write(&path, pem)?;
    Ok(path)
}

#[cfg(not(target_os = "emscripten"))]
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
    // The single-file portable build may still be unpacking libmpv (bundled.rs).
    #[cfg(windows)]
    crate::bundled::wait();
    let mpv = Arc::new(Mpv::with_initializer(|init| {
        // Android: MediaCodec, copied back so it draws through the same GL path (video.rs).
        init.set_option(
            "hwdec",
            if cfg!(target_os = "android") {
                "mediacodec-copy"
            } else {
                "auto-safe"
            },
        )?;
        // Relix never plays through youtube-dl; left on, mpv hands every link that fails to open
        // (debrid key included) to a yt-dlp it looks for on the device.
        if let Err(e) = init.set_option("ytdl", "no") {
            crate::log!("mpv option ytdl: {e}");
        }
        // Android has no certificate file mpv's downloader can read, so every https stream failed
        // its check: it gets the roots that ship inside Relix.
        #[cfg(target_os = "android")]
        match android_roots() {
            Ok(path) => {
                if let Err(e) = init.set_option("tls-ca-file", path.to_string_lossy().as_ref()) {
                    crate::log!("mpv option tls-ca-file: {e}");
                }
            }
            Err(e) => crate::log!("player: certificate roots: {e}"),
        }
        // mpv's default read-ahead (~200 MB) is a fifth of a small TV's memory.
        if cfg!(target_os = "android") {
            init.set_option("demuxer-max-bytes", "64MiB")?;
            init.set_option("demuxer-max-back-bytes", "16MiB")?;
        }
        if let Some(lang) = &prefs.audio_lang {
            init.set_option("alang", lang.as_str())?;
        }
        match prefs.sub_lang.as_deref() {
            Some("off") => init.set_option("sid", "no")?,
            Some(lang) => init.set_option("slang", lang)?,
            None => {}
        }
        init.set_option("sub-scale", prefs.sub_scale)?;
        // Looks only: older mpv (0.37, Ubuntu 24.04) lacks `sub-border-style`, which mustn't stop playback.
        for (name, value) in [("sub-border-style", prefs.border_style()), ("sub-back-color", "#B0000000")] {
            if let Err(e) = init.set_option(name, value) {
                crate::log!("mpv option {name}: {e}");
            }
        }
        // A stream slower than the film: wait for 4 s of it (mpv: 1 s), not a stall every few seconds.
        init.set_option("cache-pause-wait", 4)?;
        init.set_option("keep-open", "no")?;
        if let Ok(log) = std::env::var("RELIX_MPV_LOG") {
            init.set_option("log-file", log.as_str())?;
        } else if crate::debug::enabled() {
            init.set_option("log-file", crate::debug::mpv_log_path().to_string_lossy().as_ref())?;
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
        // One at a time: mpv splits a whole list on ',', which breaks values with commas in them.
        for (k, v) in headers {
            init.set_option("http-header-fields-append", format!("{k}: {v}").as_str())?;
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
    crate::log!("player: playback ended ({:?})", played.as_ref().map(|(_, ended)| ended));
    // Unload the stream now (stops the network reads and sound) and return at once; the core itself
    // is destroyed by its last holder, off the UI thread (video.rs).
    let _ = mpv.command("stop", &[]);
    played
}

#[cfg(not(target_os = "emscripten"))]
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
        ("paused-for-cache", Format::Flag),
        ("seeking", Format::Flag),
        ("frame-drop-count", Format::Int64),
        ("chapter-list/count", Format::Int64),
        ("hwdec-current", Format::String),
        ("cache-speed", Format::Int64),
    ]
    .into_iter()
    .enumerate()
    {
        mpv.observe_property(name, format, id as u64)?;
    }
    let opened = std::time::Instant::now();
    crate::log!("player: opening a stream from {}", crate::debug::host(url));
    mpv.command("loadfile", &[url])?;

    let mut status = Status::default();
    // The position changes every frame; the controls get it a few times a second.
    let mut sent_position = f64::MIN;
    // Subtitles wait here until the file has loaded.
    let mut pending: Vec<[String; 4]> = vec![];
    // The first addon subtitle in the preferred language is selected, unless one already is.
    let mut want_sub = prefs.sub_lang.clone().filter(|l| l != "off");
    let mut drops = DropWatch::new();
    let mut retried = false;
    // Stalls after the first seconds, and the download speed sampled while stalled (bytes/s).
    let (mut stalls, mut speed_sum, mut speed_n) = (0, 0., 0);
    const SUB_PACE: std::time::Duration = std::time::Duration::from_millis(500);
    let mut sub_pace = std::time::Instant::now() - SUB_PACE;
    loop {
        for msg in inbox.try_iter() {
            match msg {
                ToPlayer::Subtitles(subs) => pending.extend(subs),
                ToPlayer::Stop => return Ok((progress(&status), Some(Ended::Stopped))),
                ToPlayer::Next => return Ok((progress(&status), Some(Ended::NextRequested))),
                ToPlayer::Smaller => {
                    let speed = if speed_n > 0 { speed_sum / speed_n as f64 } else { 0. };
                    crate::log!("player: switching to a smaller version ({:.1} Mbit/s measured)", speed * 8. / 1e6);
                    return Ok((progress(&status), Some(Ended::TooSlow(speed))));
                }
            }
        }
        if status.loaded {
            // A few at a time: a hundred at once gets the subtitle host answering 429 to most of
            // them, and they crowd the video's own download. The preferred language goes first.
            if let Some(lang) = &want_sub {
                pending.sort_by_key(|a| &a[3] != lang);
            }
            let batch = if sub_pace.elapsed() >= SUB_PACE { pending.len().min(2) } else { 0 };
            if batch > 0 {
                sub_pace = std::time::Instant::now();
            }
            for mut args in pending.drain(..batch) {
                if status.sid.is_none() && want_sub.as_deref() == Some(args[3].as_str()) {
                    args[1] = "select".into();
                    want_sub = None;
                }
                // `async`: mpv downloads it on its own thread. A plain `sub-add` downloads inside playback,
                // freezing the video, and with a hundred subtitles this loop then can't see Stop for minutes.
                let args: Vec<&str> = std::iter::once("sub-add").chain(args.iter().map(String::as_str)).collect();
                if let Err(e) = mpv.command("async", &args) {
                    crate::log!("sub-add {}: {e}", crate::debug::host(args[1]));
                }
            }
        }
        // Wakes often enough that Stop and Next feel instant, and picks up subtitles meanwhile.
        let before = status.clone();
        match mpv.wait_event(0.05) {
            Some(Ok(Event::FileLoaded)) => {
                status.loaded = true;
                crate::log!("player: stream loaded in {} ms", opened.elapsed().as_millis());
            }
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
                // Logged on changes only: mpv also reports each property's starting value.
                (9, PropertyData::Flag(v)) => {
                    if v != status.buffering {
                        crate::log!("player: {}", if v { "buffering" } else { "buffered" });
                    }
                    if v && !status.buffering && status.loaded && !status.seeking && opened.elapsed().as_secs() > 10 {
                        stalls += 1;
                        status.slow = stalls >= 3;
                    }
                    status.buffering = v;
                }
                (10, PropertyData::Flag(v)) => {
                    if v != status.seeking {
                        crate::log!("player: {}", if v { "seeking" } else { "seeked" });
                    }
                    status.seeking = v;
                }
                // Set once the decoder opens (after the seek, on a resume): which one mpv picked.
                (13, PropertyData::Str(v)) => {
                    crate::log!("player: decoding {}", if v.is_empty() || v == "no" { "on the CPU".to_owned() } else { format!("on the GPU ({v})") });
                }
                (14, PropertyData::Int64(n)) if status.buffering && n > 0 => {
                    speed_sum += n as f64;
                    speed_n += 1;
                }
                (12, PropertyData::Int64(n)) => (status.intros, status.credits) = chapter_marks(mpv, n, status.duration),
                (11, PropertyData::Int64(n)) => {
                    // Seeks and stalls skip frames on purpose; only steady playback counts.
                    let steady = status.loaded && !status.seeking && !status.buffering && !status.paused && crate::video::drawing();
                    if drops.lighten(n, steady, std::time::Instant::now()) {
                        crate::log!("player: dropping frames ({n} so far); switching to lighter rendering");
                        if let Err(e) = mpv.command("apply-profile", &["fast"]) {
                            crate::log!("player: lighter rendering: {e}");
                        }
                    }
                }
                _ => {}
            },
            // A playlist link expanding into its entries; playback goes on with them.
            Some(Ok(Event::EndFile(libmpv2::mpv_end_file_reason::Redirect))) => {}
            Some(Ok(Event::EndFile(_) | Event::Shutdown)) => return Ok((progress(&status), None)),
            // libmpv2 reports an errored end of file this way. Once playing, keep the progress.
            Some(Err(_)) if status.loaded => return Ok((progress(&status), Some(Ended::Failed))),
            // Debrid links often refuse the first request while the file is made ready: one more try.
            Some(Err(libmpv2::Error::Raw(-13))) if !retried => {
                retried = true;
                crate::log!("player: the stream didn't open; trying once more");
                std::thread::sleep(std::time::Duration::from_secs(1));
                mpv.command("loadfile", &[url])?;
            }
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

/// Frames mpv drops during steady playback: past 12 in 10 seconds the GPU can't keep up with
/// mpv's high-quality scaling, so playback switches to its lighter "fast" profile, once.
struct DropWatch {
    /// The count and time the current 10-second window started at.
    from: (i64, std::time::Instant),
    lightened: bool,
}

impl DropWatch {
    fn new() -> Self {
        Self { from: (0, std::time::Instant::now()), lightened: false }
    }

    /// Takes mpv's running drop count; true when it's time to switch.
    fn lighten(&mut self, count: i64, steady: bool, now: std::time::Instant) -> bool {
        let window = std::time::Duration::from_secs(10);
        if self.lightened || !steady || now.duration_since(self.from.1) > window {
            self.from = (count, now);
            return false;
        }
        if count - self.from.0 >= 12 {
            self.lightened = true;
            return true;
        }
        false
    }
}

#[cfg(not(target_os = "emscripten"))]
/// The intros and where the credits start, from the file's chapters.
fn chapter_marks(mpv: &Mpv, count: i64, duration: f64) -> (Vec<(f64, f64)>, Option<f64>) {
    let chapters: Vec<(String, f64)> = (0..count)
        .map(|i| {
            let title = mpv.get_property::<String>(&format!("chapter-list/{i}/title")).unwrap_or_default();
            (title, mpv.get_property::<f64>(&format!("chapter-list/{i}/time")).unwrap_or(0.))
        })
        .collect();
    // The last chapter ends with the file; duration may not be known yet, so ask mpv.
    let duration = if duration > 0. { duration } else { mpv.get_property::<f64>("duration").unwrap_or(f64::MAX) };
    (intro_ranges(&chapters, duration), credits_start(&chapters, duration))
}

/// The first chapter named as end credits ("Ending", "ED", "ED2", "Credits", "Outro") in the
/// second half: an episode's credits, not a cold open's.
fn credits_start(chapters: &[(String, f64)], duration: f64) -> Option<f64> {
    let is_credits = |title: &str| {
        let t = title.trim().to_lowercase();
        let ed = t.strip_prefix("ed").is_some_and(|rest| rest.trim_start_matches([' ', '-']).chars().all(|c| c.is_ascii_digit()));
        !t.contains("opening") && (ed || t.starts_with("ending") || t.contains("credits") || t.contains("outro"))
    };
    chapters.iter().find(|(title, start)| *start >= duration / 2. && is_credits(title)).map(|(_, start)| *start)
}

fn intro_ranges(chapters: &[(String, f64)], duration: f64) -> Vec<(f64, f64)> {
    let is_intro = |title: &str| {
        let t = title.trim().to_lowercase();
        let op = t.strip_prefix("op").is_some_and(|rest| rest.trim_start_matches([' ', '-']).chars().all(|c| c.is_ascii_digit()));
        op || t.contains("intro") || t.starts_with("opening")
    };
    chapters
        .iter()
        .enumerate()
        .filter(|(_, (title, _))| is_intro(title))
        .filter_map(|(i, (_, start))| {
            let end = chapters.get(i + 1).map_or(duration, |c| c.1);
            (end > *start).then_some((*start, end))
        })
        .collect()
}

fn progress(s: &Status) -> Progress {
    // Into the credits counts as watched, however long they run.
    let in_credits = s.credits.is_some_and(|c| s.position >= c);
    Progress { position: if in_credits { s.duration } else { s.position }, duration: s.duration }
}

#[cfg(not(target_os = "emscripten"))]
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
    fn intro_chapters_become_skippable_ranges() {
        let ch = |t: &str, s: f64| (t.to_owned(), s);
        let list = [ch("Prologue", 0.), ch("Opening", 92.), ch("Part A", 182.), ch("ED", 1300.)];
        assert_eq!(intro_ranges(&list, 1420.), vec![(92., 182.)]);
        for t in ["Intro", "OP", "op1", "OP 2", "Opening Credits", "Intro Start"] {
            assert_eq!(intro_ranges(&[ch(t, 0.), ch("Episode", 90.)], 1400.), vec![(0., 90.)], "{t}");
        }
        for t in ["Operation", "Chapter 1", "Ending", "Open Season"] {
            assert!(intro_ranges(&[ch(t, 0.), ch("x", 90.)], 1400.).is_empty(), "{t}");
        }
    }

    #[test]
    fn credit_chapters_in_the_second_half() {
        let ch = |t: &str, s: f64| (t.to_owned(), s);
        let list = [ch("Opening", 0.), ch("Part A", 90.), ch("Ending", 1290.), ch("Preview", 1380.)];
        assert_eq!(credits_start(&list, 1420.), Some(1290.));
        for t in ["ED", "ed1", "End Credits", "Outro", "Credits"] {
            assert_eq!(credits_start(&[ch("A", 0.), ch(t, 1300.)], 1420.), Some(1300.), "{t}");
        }
        assert_eq!(credits_start(&[ch("Opening Credits", 800.)], 1420.), None);
        assert_eq!(credits_start(&[ch("Credits", 30.), ch("Edward", 1300.)], 1420.), None, "early, or not credits");
        let s = Status { position: 1300., duration: 1420., credits: Some(1290.), ..Status::default() };
        assert_eq!(progress(&s).position, 1420., "stopped in the credits: finished");
    }

    #[test]
    fn steady_dropping_lightens_rendering_once() {
        use std::time::{Duration, Instant};
        let t = Instant::now();
        let mut w = DropWatch::new();
        w.from = (0, t);
        assert!(!w.lighten(5, true, t + Duration::from_secs(1)));
        assert!(w.lighten(12, true, t + Duration::from_secs(5)), "12 in 5 seconds");
        assert!(!w.lighten(40, true, t + Duration::from_secs(6)), "only once");
        // A seek's skipped frames and slow trickles don't count.
        let mut w = DropWatch::new();
        w.from = (0, t);
        assert!(!w.lighten(30, false, t + Duration::from_secs(1)), "seeking");
        assert!(!w.lighten(35, true, t + Duration::from_secs(2)));
        assert!(!w.lighten(41, true, t + Duration::from_secs(13)), "6 more, but a new window");
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
