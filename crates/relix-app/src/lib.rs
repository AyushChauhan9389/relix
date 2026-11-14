mod account;
mod addons;
mod browse;
#[cfg(windows)]
mod bundled;
mod detail;
mod discord;
mod drive;
mod filter;
#[macro_use]
mod debug;
mod gamepad;
mod focus;
mod home;
mod instance;
mod library;
mod motion;
mod my_list;
mod player;
mod player_ui;
mod search;
mod settings;
mod social;
mod together;
mod tmdb;
mod store;
mod stremio;
mod sync;
mod update;
mod video;
mod theme;

use std::rc::Rc;
use std::sync::Arc;

use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{Addon, CatalogResponse, Manifest, MetaPreview, ResourceRequest, manifest_url, with_admin_addons};
use tokio::task::JoinSet;

use account::Account;
use addons::Addons;
use browse::{Browse, BrowseCatalog};
use detail::Detail;
use together::Friends;
use focus::{Dir, Registry};
use home::{Home, Row};
use my_list::MyList;
use search::Search;
use settings::Settings;
use store::Store;
use theme::Look;

/// Preinstalled addons (PLAN.md: defaults).
const DEFAULT_ADDONS: [&str; 2] =
    ["https://v3-cinemeta.strem.io/manifest.json", "https://opensubtitles-v3.strem.io/manifest.json"];

/// The desktop app (main.rs).
pub fn main() {
    debug::init();
    log_panics();
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        // Release workflow: `relix-app --bundle <out.exe>` writes the single-file portable build.
        if let Some(out) = std::env::args().skip_while(|a| a != "--bundle").nth(1) {
            let dll = exe.with_file_name("libmpv-2.dll");
            std::process::exit(match bundled::write(std::path::Path::new(&out), &exe, &dll) {
                Ok(()) => 0,
                Err(e) => {
                    crate::log!("bundle: {e}");
                    1
                }
            });
        }
    }

    // ponytail: playback check without the UI:
    // `--play <url> [--subs movie/<imdb id>] [--title T] [--start secs]`, in mpv's own window.
    if let Some(url) = std::env::args().skip_while(|a| a != "--play").nth(1) {
        return play_check(&url);
    }

    // One Relix at a time; a second launch brings this one forward. Off-screen tests run alongside.
    let _lock = match test_offscreen() {
        true => None,
        false => match instance::claim(&data_dir(), std::env::args().any(|a| a == "--after-update")) {
            instance::Claim::Shown => return,
            instance::Claim::Primary(lock, listener) => {
                instance::serve(listener);
                Some(lock)
            }
            instance::Claim::Unsure => None,
        },
    };
    // After the claim, so a relaunch after an update has waited for the old copy to let go of it.
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe()
        && let Some(len) = bundled::appended_len(&exe)
        && let Err(e) = bundled::unpack(&exe, len, &data_dir().join("mpv"))
    {
        crate::log!("unpacking libmpv: {e}");
    }
    update::cleanup();
    run(LaunchConfig::new());
}

/// The app handle Android starts Relix with: its data folder, JVM and activity.
#[cfg(target_os = "android")]
static ANDROID: std::sync::OnceLock<freya::winit::platform::android::activity::AndroidApp> = std::sync::OnceLock::new();

/// Android's entry point: the activity loads Relix as a library and calls this.
/// Android keeps one copy running itself, and the app store updates it.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: freya::winit::platform::android::activity::AndroidApp) {
    use freya::winit::event_loop::EventLoop;
    use freya::winit::platform::android::EventLoopBuilderExtAndroid;
    let _ = ANDROID.set(app.clone());
    debug::init();
    log_panics();
    player::android_init(app.vm_as_ptr());
    let event_loop = EventLoop::<NativeEvent>::with_user_event().with_android_app(app.clone()).build().expect("event loop");
    run(LaunchConfig::new().with_plugin(freya::android::AndroidPlugin::new(app)).with_event_loop(event_loop));
}

/// Back presses from the activity (MainActivity.kt), for the UI to take.
#[cfg(target_os = "android")]
static BACK: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<()>> = std::sync::OnceLock::new();

/// MainActivity's `nativeBack`: Android 13+ gives Back (button or gesture) to the activity, not as a key.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
extern "system" fn Java_tech_carbonkit_relix_MainActivity_nativeBack(_env: *mut std::ffi::c_void, _this: *mut std::ffi::c_void) {
    if let Some(back) = BACK.get() {
        let _ = back.send(());
    }
}

/// Back on Home: Relix goes to the background, as Android apps do, rather than closing.
#[cfg(target_os = "android")]
fn android_to_background() {
    use jni::objects::JObject;
    use jni::{JValue, jni_sig, jni_str};
    let Some(app) = ANDROID.get() else { return };
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    let moved = vm.attach_current_thread(|env| {
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr() as *mut _) };
        env.call_method(&activity, jni_str!("moveTaskToBack"), jni_sig!("(Z)Z"), &[JValue::Bool(true)]).map(drop)
    });
    if let Err(e) = moved {
        crate::log!("back: {e}");
    }
}

/// Opens `url` in whatever app views it (the browser, YouTube for trailers).
#[cfg(target_os = "android")]
fn android_view(url: &str) -> Result<(), jni::errors::Error> {
    use jni::objects::JObject;
    use jni::{JValue, jni_sig, jni_str};
    let Some(app) = ANDROID.get() else { return Ok(()) };
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    vm.attach_current_thread(|env| {
        let view = |env: &mut jni::Env| -> Result<(), jni::errors::Error> {
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr() as *mut _) };
            let url = env.new_string(url)?;
            let uri = env
                .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&url)])?
                .l()?;
            let action = env.new_string("android.intent.action.VIEW")?;
            let intent = env.new_object(
                jni_str!("android/content/Intent"),
                jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            env.call_method(&activity, jni_str!("startActivity"), jni_sig!("(Landroid/content/Intent;)V"), &[JValue::Object(&intent)])?;
            Ok(())
        };
        let result = view(env);
        // No app for the link throws in Java; clear it so later JNI calls still work.
        if result.is_err() {
            env.exception_clear();
        }
        result
    })
}

/// The physical pixels Android's status and navigation bars and the camera cutout cover:
/// (top, right, bottom, left). `None` until the window is attached (and on errors).
#[cfg(target_os = "android")]
fn android_insets() -> Option<(i32, i32, i32, i32)> {
    use jni::objects::JObject;
    use jni::{JValue, jni_sig, jni_str};
    let app = ANDROID.get()?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    vm.attach_current_thread(|env| {
        let read = |env: &mut jni::Env| -> Result<(i32, i32, i32, i32), jni::errors::Error> {
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr() as *mut _) };
            let window = env.call_method(&activity, jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?.l()?;
            let decor = env.call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?.l()?;
            // Null before the view is attached: the call below then fails and is retried later.
            let insets = env.call_method(&decor, jni_str!("getRootWindowInsets"), jni_sig!("()Landroid/view/WindowInsets;"), &[])?.l()?;
            let ty = |env: &mut jni::Env, name| -> Result<i32, jni::errors::Error> {
                env.call_static_method(jni_str!("android/view/WindowInsets$Type"), name, jni_sig!("()I"), &[])?.i()
            };
            let types = ty(env, jni_str!("systemBars"))? | ty(env, jni_str!("displayCutout"))?;
            let i = env.call_method(&insets, jni_str!("getInsets"), jni_sig!("(I)Landroid/graphics/Insets;"), &[JValue::Int(types)])?.l()?;
            let mut side = |name| env.get_field(&i, name, jni_sig!("I")).and_then(|v| v.i());
            Ok((side(jni_str!("top"))?, side(jni_str!("right"))?, side(jni_str!("bottom"))?, side(jni_str!("left"))?))
        };
        let result = read(env);
        if result.is_err() {
            env.exception_clear();
        }
        Ok::<_, jni::errors::Error>(result.ok())
    })
    .ok()
    .flatten()
}

/// Opens the window and runs until it closes.
fn run(config: LaunchConfig) {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let _rt = rt.enter();
    // Posters and backdrops stay on the GPU: at Skia's default 256 MiB, scrolling back evicted
    // and re-uploaded them, which showed as stutter.
    let config = config
        .with_gpu_resource_cache_limit(GpuResourceCacheLimit::from_mb(gpu_cache_mb()))
        .with_window(
            WindowConfig::new(app)
                .with_title("Relix")
                .with_icon(LaunchConfig::window_icon(include_bytes!(
                    "../assets/icon.png"
                )))
                .with_size(test_size().0, test_size().1)
                // Video is drawn through OpenGL (video.rs), so Freya renders with it everywhere.
                .with_renderer(RendererPreference::OpenGl)
                .with_window_attributes(|attrs, _| {
                    if test_offscreen() {
                        offscreen(attrs)
                    } else {
                        attrs
                    }
                }),
        );
    // Debug mode: the FPS / frame-time overlay (Ctrl+Shift+P hides it). Debug builds get it from Freya.
    #[cfg(not(debug_assertions))]
    let config = if debug::enabled() { config.with_plugin(freya::metrics::MetricsPlugin::default().with_visible_performance(true)) } else { config };
    launch(config)
}

/// Skia's GPU cache: 512 MiB, or an eighth of the memory on smaller devices (a 1 GB TV shares it
/// with the video decoder; a full cache there gets Relix killed).
fn gpu_cache_mb() -> usize {
    let total_kb = std::fs::read_to_string("/proc/meminfo").ok().and_then(|m| {
        m.lines().find_map(|l| {
            l.strip_prefix("MemTotal:")?
                .trim()
                .strip_suffix("kB")?
                .trim()
                .parse::<usize>()
                .ok()
        })
    });
    total_kb.map_or(512, |kb| (kb / 1024 / 8).clamp(96, 512))
}

/// Release builds have no console (and Android none at all), so a panic would vanish: keep it in crash.log too.
fn log_panics() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let entry = format!("{info}\n{}\n\n", std::backtrace::Backtrace::force_capture());
        // The data folder may be the problem (read-only portable copy): fall back to the temp folder.
        for dir in [data_dir(), std::env::temp_dir().join("relix")] {
            let _ = std::fs::create_dir_all(&dir);
            let written = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")).and_then(|mut f| {
                use std::io::Write;
                f.write_all(entry.as_bytes())
            });
            if written.is_ok() {
                break;
            }
        }
        default_hook(info);
    }));
}

/// `--play`: plays `url` in mpv's own window, without the UI.
fn play_check(url: &str) {
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(target) = std::env::args().skip_while(|a| a != "--subs").nth(1) {
        std::thread::spawn(move || {
            let url = format!("https://opensubtitles-v3.strem.io/subtitles/{target}.json");
            let rt = tokio::runtime::Runtime::new().unwrap();
            match rt.block_on(get::<relix_proto::addon::SubtitlesResponse>(reqwest::Client::new(), url)) {
                Ok(r) => {
                    println!("{} subtitles", r.subtitles.len());
                    let _ = tx.send(player::sub_add_args(&r.subtitles, "OpenSubtitles"));
                }
                Err(e) => crate::log!("subtitles: {e}"),
            }
        });
    }
    let title = std::env::args().skip_while(|a| a != "--title").nth(1).unwrap_or_default();
    let start = std::env::args().skip_while(|a| a != "--start").nth(1).and_then(|s| s.parse().ok()).unwrap_or(0.);
    let inbox = {
        let (to_player, inbox) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for subs in rx {
                let _ = to_player.send(player::ToPlayer::Subtitles(subs));
            }
        });
        inbox
    };
    match player::play(url, &Default::default(), start, &title, &Default::default(), None, inbox, |_| {}) {
        Ok((p, ended)) => println!("stopped at {:.1}s of {:.1}s ({ended:?})", p.position, p.duration),
        Err(e) => crate::log!("mpv: {e}"),
    }
}

/// `RELIX_TEST_OFFSCREEN=1`: for automated checks on a developer's machine. The window opens
/// off-screen, unfocused and off the taskbar, and playback is muted, so nothing interrupts them.
pub fn test_offscreen() -> bool {
    std::env::var_os("RELIX_TEST_OFFSCREEN").is_some()
}

/// The window's starting size; `RELIX_TEST_SIZE=412x915` tries a phone-sized one on the desktop.
fn test_size() -> (f64, f64) {
    let size = std::env::var("RELIX_TEST_SIZE").ok();
    size.as_deref().and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).unwrap_or((1440., 900.))
}

fn offscreen(attrs: freya::winit::window::WindowAttributes) -> freya::winit::window::WindowAttributes {
    let attrs = attrs.with_position(freya::winit::dpi::PhysicalPosition::new(-20000, -20000)).with_active(false);
    #[cfg(windows)]
    let attrs = {
        use freya::winit::platform::windows::WindowAttributesExtWindows;
        attrs.with_skip_taskbar(true)
    };
    attrs
}

/// Whether `exe` is the single-file portable build (libmpv appended), which keeps its data next to itself.
fn bundled_exe(exe: &std::path::Path) -> bool {
    #[cfg(windows)]
    return bundled::appended_len(exe).is_some();
    #[cfg(not(windows))]
    return {
        let _ = exe;
        false
    };
}

/// Where Relix keeps its database: `data` next to the executable for a portable copy (a file
/// named `portable` sits beside it), otherwise the usual per-user app data folder of each OS.
fn data_dir() -> std::path::PathBuf {
    #[cfg(target_os = "android")]
    if let Some(dir) = ANDROID.get().and_then(|app| app.internal_data_path()) {
        return dir;
    }
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(|e| e.parent().map(std::path::Path::to_path_buf));
    if let Some(dir) = exe_dir.filter(|d| d.join("portable").exists() || exe.as_deref().is_some_and(bundled_exe)) {
        return dir.join("data");
    }
    let env = |k| std::env::var_os(k).map(std::path::PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))
    };
    base.unwrap_or_default().join("relix")
}

/// App-wide data every screen reads.
#[derive(Clone)]
pub struct Ctx {
    pub http: reqwest::Client,
    pub store: Rc<Store>,
    /// Id of the profile every record belongs to (one implicit profile for now).
    pub profile: String,
    /// The user's own installed addons, in order.
    pub addons: State<Vec<Addon>>,
    /// Addons the sync server's admin publishes to signed-in accounts (cached locally).
    pub admin_addons: State<Vec<Addon>>,
    /// The user's switch for admin addons; on unless they turned it off (synced setting).
    pub admin_enabled: State<bool>,
    pub rows: State<Vec<Row>>,
    /// Series with an episode out since the viewer last watched (library::find_new_episodes).
    pub new_episodes: State<std::collections::HashSet<String>>,
    /// While Home's catalogs are being fetched.
    pub rows_loading: State<bool>,
    /// A title Home's Play asked to start; its page plays it once loaded.
    pub autoplay_title: State<Option<String>>,
    /// Wakes the sync loop (sign-in, local changes).
    pub sync: Arc<tokio::sync::Notify>,
    /// Last sync outcome, for the Account screen.
    pub sync_status: State<String>,
    /// The running player, while something plays.
    pub player: State<Option<std::sync::mpsc::Sender<player::ToPlayer>>>,
    /// What's playing (its video and titles), the player's state, and a counter that changes with
    /// each new video frame.
    pub now_playing: State<Option<NowPlaying>>,
    pub player_status: State<player::Status>,
    /// Gain on top of mpv's 100% volume (player_ui::set_volume), 1 when not boosted.
    pub volume_boost: State<f64>,
    pub video_frame: State<u64>,
    /// When the viewer last moved the mouse or pressed a key in the player; its controls show a
    /// while after (player_ui).
    pub player_active: State<std::time::Instant>,
    /// The player's open menu (subtitles, speed), if any.
    pub player_menu: State<Option<player_ui::Menu>>,
    pub fullscreen: State<bool>,
    /// Netflix or Stremio look (synced setting).
    pub look: State<Look>,
    /// Freya's built-in widget colours, following the look.
    pub widgets: State<Theme>,
    /// A downloaded update waiting for a restart.
    pub update: State<Option<update::Ready>>,
    /// Space the system's bars and the camera cutout cover (top, right, bottom, left), Android only.
    pub safe: State<Gaps>,
    /// Friends and Watch Together.
    pub social: social::Social,
}

impl Ctx {
    /// Re-reads Continue Watching and My List into Home, leaving catalog rows as they are.
    pub fn refresh_local_rows(&self) {
        let mut rows = self.rows;
        let mut next = library::local_rows(&self.store, &self.profile);
        next.extend(rows.peek().iter().filter(|r| !r.id.starts_with("local/")).cloned());
        rows.set(next);
    }

    /// Every addon content comes from: the user's own, then the admin's when switched on.
    pub fn all_addons(&self) -> Vec<Addon> {
        with_admin_addons(&self.addons.read(), &self.admin_addons.read(), *self.admin_enabled.read())
    }

    /// Turns the server's admin addons on or off for this user, and syncs the choice.
    pub fn set_admin_enabled(&self, on: bool) {
        if let Err(e) = self.store.put(&self.profile, "settings", "admin_addons_enabled", &on) {
            crate::log!("saving setting: {e}");
        }
        let mut enabled = self.admin_enabled;
        enabled.set(on);
        spawn(load_rows(self.clone()));
        self.sync_soon();
    }

    /// Switches the look and syncs the choice.
    pub fn set_look(&self, look: Look) {
        if let Err(e) = self.store.put(&self.profile, "settings", "look", &look) {
            crate::log!("saving setting: {e}");
        }
        self.show_look(look);
        self.sync_soon();
    }

    /// Applies a look without saving it (e.g. synced from another device).
    pub fn show_look(&self, look: Look) {
        let (mut current, mut widgets) = (self.look, self.widgets);
        if *current.peek() != look {
            theme::set_look(look);
            widgets.set(theme::widgets(look));
            current.set(look);
        }
    }

    /// Makes Relix's window fullscreen, or back.
    pub fn toggle_fullscreen(&self) {
        use freya::winit::window::Fullscreen;
        let mut fullscreen = self.fullscreen;
        // The window decides: it may have left fullscreen through the system (a shortcut, the compositor).
        Platform::get().with_window(Platform::window_id(), move |w| {
            let on = w.fullscreen().is_none();
            w.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
            fullscreen.set(on);
        });
    }

    /// Shows a player full-window; called with the mpv the player hands over.
    pub fn show_video(&self, mpv: std::sync::Arc<libmpv2::Mpv>, ready: std::sync::mpsc::Sender<()>, overlay: player::Overlay) {
        // A player that already stopped (a dead link fails fast) hands over its mpv too late to show.
        // Its destroy waits on mpv's threads, so it goes off the UI thread.
        if self.player.peek().is_none() {
            std::thread::spawn(move || drop(mpv));
            return;
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut frame = self.video_frame;
        spawn_in_window(async move {
            while rx.recv().await.is_some() {
                *frame.write() += 1;
            }
        });
        let video = Rc::new(video::VideoRenderer::new(mpv, ready, move || {
            let _ = tx.send(());
        }));
        let (mut now, mut status) = (self.now_playing, self.player_status);
        status.set(player::Status::default());
        let mut boost = self.volume_boost;
        boost.set(1.);
        let (mut active, mut menu) = (self.player_active, self.player_menu);
        active.set(std::time::Instant::now());
        menu.set(None);
        now.set(Some(NowPlaying {
            video,
            title: overlay.title,
            episode: overlay.episode,
            has_next: overlay.has_next,
            logo: overlay.logo,
            background: overlay.background,
        }));
    }

    /// Esc while playing: leave fullscreen first, then stop.
    pub fn player_escape(&self) {
        crate::log!("player: Esc");
        let (mut fullscreen, player) = (self.fullscreen, self.player.peek().clone());
        Platform::get().with_window(Platform::window_id(), move |w| {
            if w.fullscreen().is_some() {
                w.set_fullscreen(None);
                fullscreen.set(false);
            } else if let Some(p) = player {
                let _ = p.send(player::ToPlayer::Stop);
            }
        });
    }

    /// Asks the sync loop to run soon (no-op when signed out).
    /// The viewer's playback preferences (a synced setting).
    pub fn player_prefs(&self) -> player::Prefs {
        self.store.get(&self.profile, "settings", "player").ok().flatten().unwrap_or_default()
    }

    pub fn save_player_prefs(&self, prefs: &player::Prefs) {
        match self.store.put(&self.profile, "settings", "player", prefs) {
            Ok(()) => self.sync_soon(),
            Err(e) => crate::log!("saving player settings: {e}"),
        }
    }

    pub fn sync_soon(&self) {
        self.sync.notify_one();
        social::wake();
    }

    /// Persists the addon list and rebuilds Home's rows from it.
    pub fn save_addons(&self, addons: Vec<Addon>) {
        let mut this = self.clone();
        if let Err(e) = self.store.put(&self.profile, "addons", "list", &addons) {
            crate::log!("saving addons: {e}");
        }
        self.sync_soon();
        this.addons.set(addons);
        spawn(load_rows(this));
    }
}

/// The one HTTP client for addons, sync and updates.
fn http_client() -> reqwest::Client {
    // No overall timeout: the sync event stream and update downloads run long. The read timeout
    // still ends a stalled one (the server sends keep-alives every 15s).
    let builder = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10)).read_timeout(std::time::Duration::from_secs(30));
    // The system certificate check needs a Java helper on Android; Mozilla's roots ship inside instead.
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|c| reqwest::Certificate::from_der(c).ok()));
    builder.build().expect("HTTP client")
}

/// A JSON request that gives up after 20s, so a stalled addon can't hold a screen forever.
pub async fn get<T: serde::de::DeserializeOwned>(http: reqwest::Client, url: String) -> reqwest::Result<T> {
    // Errors drop the URL: addon URLs can carry debrid keys, and errors end up in logs.
    let res = http.get(url).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| e.without_url())?;
    res.error_for_status().map_err(|e| e.without_url())?.json().await.map_err(|e| e.without_url())
}

fn app() -> impl IntoElement {
    let store = use_hook(|| Rc::new(Store::open(&data_dir().join("relix.db")).expect("open database")));
    let profile = use_hook(|| {
        let first = store.profiles().ok().and_then(|p| p.into_iter().next());
        first.unwrap_or_else(|| store.create_profile("Me").expect("create profile")).id
    });
    let look = use_hook(|| {
        let look = store.get::<Look>(&profile, "settings", "look").ok().flatten().unwrap_or(Look::Netflix);
        theme::set_look(look);
        look
    });
    let widgets = use_init_theme(|| theme::widgets(look));
    let look = use_state(|| look);
    let saved = store.get::<Vec<Addon>>(&profile, "addons", "list").ok().flatten();
    let addons = use_state(|| saved.clone().unwrap_or_default());
    let rows = use_state(Vec::<Row>::new);
    let new_episodes = use_state(std::collections::HashSet::new);
    let rows_loading = use_state(|| true);
    let autoplay_title = use_state(|| None);
    let sync_status = use_state(String::new);
    let player = use_state(|| None);
    let now_playing = use_state(|| None);
    let player_status = use_state(player::Status::default);
    let volume_boost = use_state(|| 1.);
    let video_frame = use_state(|| 0);
    let player_active = use_state(std::time::Instant::now);
    let player_menu = use_state(|| None);
    let fullscreen = use_state(|| false);
    let admin_addons = use_state(|| {
        let cached = store.meta("admin_addons").ok().flatten();
        cached.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default()
    });
    let admin_enabled =
        use_state(|| store.get::<bool>(&profile, "settings", "admin_addons_enabled").ok().flatten().unwrap_or(true));
    let social = social::Social::new();
    let ctx = use_provide_context(|| Ctx {
        // No overall timeout: the sync event stream and update downloads run long. The read timeout
        // still ends a stalled one (the server sends keep-alives every 15s).
        http: http_client(),
        store: store.clone(),
        profile,
        addons,
        admin_addons,
        admin_enabled,
        rows,
        new_episodes,
        rows_loading,
        autoplay_title,
        sync: Arc::default(),
        sync_status,
        player,
        now_playing,
        player_status,
        volume_boost,
        video_frame,
        player_active,
        player_menu,
        fullscreen,
        look,
        widgets,
        update: use_state(|| None),
        safe: use_state(Gaps::default),
        social,
    });

    use_hook({
        let ctx = ctx.clone();
        move || {
            spawn(update::check(ctx.clone()));
            spawn(library::find_new_episodes(ctx.clone()));
            spawn(social::run(ctx.clone()));
            spawn(sync::run(ctx));
        }
    });
    use_hook(move || {
        spawn(async move {
            match saved {
                Some(_) => load_rows(ctx).await,
                None => install_defaults(&ctx).await,
            }
        });
    });

    // `--route /title/movie/tt1375666` opens a screen directly, for development.
    let initial = std::env::args()
        .skip_while(|a| a != "--route")
        .nth(1)
        .and_then(|r| r.parse::<Route>().inspect_err(|e| crate::log!("--route {r}: {e}")).ok())
        .unwrap_or(Route::Home);
    Router::<Route>::new(move || RouterConfig::default().with_initial_path(initial.clone()))
}

async fn install_defaults(ctx: &Ctx) {
    let mut addons = vec![];
    for url in DEFAULT_ADDONS {
        let url = manifest_url(url).unwrap();
        match get::<Manifest>(ctx.http.clone(), url.to_string()).await {
            Ok(manifest) => addons.push(Addon { manifest_url: url, manifest }),
            Err(e) => crate::log!("manifest {url}: {e}"),
        }
    }
    ctx.save_addons(addons);
}

/// Rebuilds Home's rows: one per home catalog, in addon order, each filled as its catalog answers.
pub async fn load_rows(mut ctx: Ctx) {
    // Runs overlap (addons change, sync, Try again); only the newest one touches the rows.
    static LATEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let run = LATEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let current = || LATEST.load(std::sync::atomic::Ordering::Relaxed) == run;
    ctx.rows_loading.set(true);
    let addons = ctx.all_addons();
    let mut rows = library::local_rows(&ctx.store, &ctx.profile);
    let mut set = JoinSet::new();
    // Recommendations from what this profile watched, then TMDB's trending titles, then every
    // addon's home catalogs. Titles already watched or saved are left out of recommendations.
    if tmdb::enabled() {
        let (seeds, seen) = library::taste_seeds(&ctx.store, &ctx.profile);
        let seen = std::sync::Arc::new(seen);
        let unseen = |seen: &std::collections::HashSet<String>, metas: Vec<MetaPreview>| metas.into_iter().filter(|m| !seen.contains(&m.id)).collect::<Vec<_>>();
        for (kind, title) in [("movie", "For You · Movies"), ("series", "For You · Series")] {
            // shortcut: the 8 most recent titles of a type make the taste; weigh older ones in if it feels narrow.
            let picked: Vec<tmdb::Seed> = seeds.iter().filter(|(s, _)| s.kind == kind).take(8).map(|(s, _)| tmdb::Seed { imdb: s.imdb.clone(), kind: s.kind.clone(), weight: s.weight }).collect();
            if picked.len() < 2 {
                continue;
            }
            let id = format!("tmdb/for-you/{kind}");
            rows.push(Row { id: id.clone(), title: title.into(), metas: vec![], progress: vec![], loading: true, next: None });
            let (http, seen) = (ctx.http.clone(), seen.clone());
            set.spawn(async move { (id, tmdb::for_you(&http, kind, &picked).await.map(|m| unseen(&seen, m))) });
        }
        for (seed, name) in seeds.into_iter().take(2) {
            let id = format!("tmdb/because/{}", seed.imdb);
            rows.push(Row { id: id.clone(), title: format!("Because you watched {name}"), metas: vec![], progress: vec![], loading: true, next: None });
            let (http, seen) = (ctx.http.clone(), seen.clone());
            set.spawn(async move { (id, tmdb::because(&http, &seed).await.map(|m| unseen(&seen, m))) });
        }
        for (kind, title) in [("movie", "Trending Movies"), ("series", "Trending Series")] {
            let id = format!("tmdb/trending/{kind}");
            rows.push(Row { id: id.clone(), title: title.into(), metas: vec![], progress: vec![], loading: true, next: None });
            let http = ctx.http.clone();
            set.spawn(async move { (id, tmdb::trending(&http, kind).await) });
        }
    }
    for addon in &addons {
        for c in addon.manifest.home_catalogs() {
            let name = c.name.clone().unwrap_or_else(|| c.id.clone());
            let kind = match c.r#type.as_str() {
                "movie" => "Movies".to_owned(),
                "series" => "Series".to_owned(),
                other => other.to_owned(),
            };
            let id = format!("{}/{}/{}", addon.manifest.id, c.r#type, c.id);
            let req = ResourceRequest::new("catalog", &c.r#type, &c.id);
            let next = c.supports("skip").then(|| (addon.clone(), req.clone()));
            rows.push(Row { id: id.clone(), title: format!("{name} {kind}"), metas: vec![], progress: vec![], loading: true, next });
            let url = req.url(addon);
            let http = ctx.http.clone();
            set.spawn(async move { (id, get::<CatalogResponse>(http, url).await.map(|r| r.metas).map_err(|e| e.to_string())) });
        }
    }
    ctx.rows.set(rows);
    while let Some(joined) = set.join_next().await {
        if !current() {
            return;
        }
        let Ok((id, res)) = joined else { continue };
        // Rows may have been rebuilt meanwhile, so find ours by id.
        let Some(i) = ctx.rows.peek().iter().position(|r| r.id == id) else { continue };
        match res {
            Ok(metas) if !metas.is_empty() => {
                let mut rows = ctx.rows.write();
                rows[i].metas = metas;
                rows[i].loading = false;
            }
            // Failed or empty: the row goes rather than sit blank.
            Ok(_) => drop(ctx.rows.write().remove(i)),
            Err(e) => {
                crate::log!("catalog {id}: {e}");
                ctx.rows.write().remove(i);
            }
        }
    }
    ctx.rows_loading.set(false);
}

// Rust 1.100 flags unreachable code inside Freya's `Routable` derive, not code of ours; the allow
// has to sit on a module, since the derive's impls are siblings of the enum.
pub use routes::Route;
#[allow(unreachable_code)]
mod routes {
    use super::*;

    #[derive(Routable, Clone, PartialEq, Debug)]
    #[rustfmt::skip]
    pub enum Route {
        #[layout(Shell)]
            #[route("/")]
            Home,
            #[route("/title/:kind/:id")]
            Detail { kind: String, id: String },
            #[route("/addons")]
            Addons,
            #[route("/search")]
            Search,
            #[route("/browse/:kind")]
            Browse { kind: String },
            #[route("/browse/:kind/:catalog")]
            BrowseCatalog { kind: String, catalog: usize },
            #[route("/account")]
            Account,
            #[route("/my-list")]
            MyList,
            #[route("/settings")]
            Settings,
            #[route("/friends")]
            Friends,
    }
}

/// "Relix x.y.z is ready · Restart", bottom-right, once an update has downloaded.
fn update_banner(ready: &update::Ready) -> Rect {
    let restart = {
        let ready = ready.clone();
        move |()| update::apply_in_background(ready.clone())
    };
    rect()
        .position(Position::new_absolute().right(24.).bottom(24.))
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(16.)
        .padding(Gaps::new(12., 12., 12., 20.))
        .corner_radius(8.)
        .background(theme::theme().surface)
        .child(label().text(format!("Relix {} is ready", ready.version)).font_size(15.))
        .child(focus::Focusable::new(home::button("Restart", theme::theme().primary.0, theme::theme().primary.1), restart))
}

#[derive(PartialEq)]
struct Shell;

/// What a key does: F11 fullscreen, arrows move focus, Esc backs out; while video plays they go
/// to the player. Gamepads (gamepad.rs) send the same keys.
fn handle_key(ctx: &Ctx, registry: &Registry, key: &Key) {
    // Android's Back button (phones and TV remotes) does what Esc does.
    let key = if *key == Key::Named(NamedKey::BrowserBack) { &Key::Named(NamedKey::Escape) } else { key };
    // F11 toggles fullscreen anywhere, like Stremio.
    if *key == Key::Named(NamedKey::F11) {
        ctx.toggle_fullscreen();
        return;
    }
    // Typing in the party's chat: keys are the text field's; Esc closes the panel.
    if ctx.player.peek().is_some() && *ctx.social.chat_open.peek() && registry.typing() {
        if *key == Key::Named(NamedKey::Escape) {
            let mut open = ctx.social.chat_open;
            open.set(false);
        }
        return;
    }
    // While video plays, keys go to the player; Esc it doesn't use (to close its controls) stops it.
    if ctx.player.peek().is_some() {
        if !player_ui::key(ctx, registry, key) && *key == Key::Named(NamedKey::Escape) {
            ctx.player_escape();
        }
        return;
    }
    let dir = match *key {
        Key::Named(NamedKey::ArrowUp) => Dir::Up,
        Key::Named(NamedKey::ArrowDown) => Dir::Down,
        Key::Named(NamedKey::ArrowLeft) => Dir::Left,
        Key::Named(NamedKey::ArrowRight) => Dir::Right,
        // Esc leaves fullscreen first, then goes back.
        Key::Named(NamedKey::Escape) if *ctx.fullscreen.peek() => {
            ctx.toggle_fullscreen();
            return;
        }
        Key::Named(NamedKey::Escape) => {
            let router = RouterContext::get();
            if router.can_go_back() {
                router.go_back();
            } else {
                #[cfg(target_os = "android")]
                android_to_background();
            }
            return;
        }
        _ => return,
    };
    registry.navigate(dir, false);
}

impl Component for Shell {
    fn render(&self) -> impl IntoElement {
        let registry = use_provide_context(Registry::default);
        let ctx = use_consume::<Ctx>();
        // The window's size: narrow windows tighten the nav, phones get their own layout.
        let size = *Platform::get().current_window().root_size.read();
        let phone = theme::is_phone_size(size.width, size.height);
        theme::set_phone(phone);
        let safe = *ctx.safe.read();
        let playing = ctx.player.read().is_some();
        let update = ctx.update.read().clone();
        let look = *ctx.look.read();
        // Phones have bottom tabs in both looks.
        let sidebar = theme::theme().sidebar && !phone;
        let route = RouterContext::get().current::<Route>().to_string();
        let keys = ctx.clone();
        // Discord "Now watching" follows what plays (only when the build has a Discord app id).
        let presence = use_hook(discord::start);
        // Another launch asked for this window: bring it forward (Wayland only flashes it).
        use_hook(|| {
            let Some(mut rx) = instance::SHOW.lock().unwrap().take() else { return };
            spawn(async move {
                while rx.recv().await.is_some() {
                    Platform::get().with_window(Platform::window_id(), |w| {
                        w.set_minimized(false);
                        w.focus_window();
                        w.request_user_attention(Some(freya::winit::window::UserAttentionType::Informational));
                    });
                }
            });
        });
        use_side_effect({
            let ctx = ctx.clone();
            move || {
                let watching = ctx.now_playing.read().clone().map(|np| discord::Watching {
                    title: np.title,
                    episode: np.episode,
                    image: np.logo.or(np.background),
                });
                if let Some(tx) = &presence {
                    let _ = tx.send(watching);
                }
                // Friends see it too (social::send_presence follows the privacy settings).
                social::send_presence(&ctx);
            }
        });
        // Android's Back arrives from the activity (MainActivity.kt); it does what Esc does.
        #[cfg(target_os = "android")]
        use_hook({
            let (ctx, registry) = (ctx.clone(), registry.clone());
            move || {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let _ = BACK.set(tx);
                spawn(async move {
                    while rx.recv().await.is_some() {
                        handle_key(&ctx, &registry, &Key::Named(NamedKey::BrowserBack));
                    }
                });
            }
        });
        // Android tells a NativeActivity nothing when its bars move (rotation, a cutout), so this checks.
        #[cfg(target_os = "android")]
        use_hook({
            let mut safe = ctx.safe;
            move || {
                let window = Platform::get().current_window();
                spawn(async move {
                    loop {
                        if let Some((t, r, b, l)) = android_insets() {
                            let scale = *window.scale_factor.peek() as f32;
                            let gaps = Gaps::new(t as f32 / scale, r as f32 / scale, b as f32 / scale, l as f32 / scale);
                            if *safe.peek() != gaps {
                                safe.set(gaps);
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                });
            }
        });
        use_hook({
            let (ctx, registry) = (ctx.clone(), registry.clone());
            move || {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                std::thread::spawn(move || gamepad::run(tx));
                spawn(async move {
                    while let Some(pad) = rx.recv().await {
                        match pad {
                            gamepad::Pad::Key(key) => handle_key(&ctx, &registry, &key),
                            gamepad::Pad::Press
                                if ctx.player.peek().is_some() && !registry.player_focused() =>
                            {
                                handle_key(&ctx, &registry, &Key::Character(" ".into()))
                            }
                            gamepad::Pad::Press => drop(registry.press()),
                        }
                    }
                });
            }
        });
        let active = home::active_link(&RouterContext::get().current::<Route>());
        // Every screen fades and rises in; the key replays it on each navigation and remounts
        // pages when the look or layout changes, so they redraw in the new colours and sizes.
        // Turning a phone remounts too: pages size some things (the hero) by the window's height.
        let sideways = phone && size.width > size.height;
        let page = motion::FadeIn::page(Outlet::<Route>::new()).key(format!("{route}-{look:?}-{phone}-{sideways}"));
        let body = if phone {
            // Inside the system bars: the page with its top bar over it, then the tabs.
            rect()
                .expanded()
                .padding(safe)
                .content(Content::Flex)
                .maybe(playing, |el| el.height(Size::px(0.)).overflow(Overflow::Clip))
                .child(
                    rect()
                        .width(Size::fill())
                        .height(Size::flex(1.))
                        .child(page)
                        .maybe(!playing, |el| el.child(home::phone_top_bar(active).layer(Layer::OverlayLevel(2)))),
                )
                .maybe(!playing, |el| el.child(home::tab_bar(active)))
        } else {
            rect()
                .expanded()
                .padding(Gaps::new(0., 0., 0., if sidebar { home::SIDEBAR_W } else { 0. }))
                // Under the player the page stays alive (it runs playback) but is clipped away, so
                // Freya skips painting it on every video frame.
                .maybe(playing, |el| el.height(Size::px(0.)).overflow(Overflow::Clip))
                .child(page)
                // Stremio's search bar tops every page but Search, which has its own field.
                .maybe(sidebar && !playing && route != Route::Search.to_string(), |el| {
                    el.child(home::top_bar(&ctx).layer(Layer::OverlayLevel(2)))
                })
        };
        rect()
            .expanded()
            .on_global_key_down({
                let registry = registry.clone();
                move |e: Event<KeyboardEventData>| handle_key(&keys, &registry, &e.key)
            })
            .background(theme::theme().bg)
            .color(Color::WHITE)
            .child(body)
            .maybe(!playing && !phone, |el| {
                el.child(if sidebar { home::sidebar(active) } else { home::nav(active, &ctx, size.width < 1000.) }.layer(Layer::OverlayLevel(2)))
            })
            .maybe_child(
                update
                    .filter(|_| !playing)
                    .map(|ready| update_banner(&ready).layer(Layer::OverlayLevel(2))),
            )
            .maybe_child(
                together::invite_toast(&ctx)
                    .filter(|_| !playing)
                    .map(|t| t.layer(Layer::OverlayLevel(2))),
            )
            .maybe_child(
                (!playing && ctx.social.party.read().is_none() && drive::enabled())
                    .then(|| ctx.social.hosted_drive_file.read().clone())
                    .flatten()
                    .map(|(id, title)| {
                        rect()
                            .position(Position::new_absolute().right(24.).bottom(100.))
                            .layer(Layer::OverlayLevel(2))
                            .child(together::DeleteFromDrive { file_id: id, title })
                    }),
            )
            // mpv draws over this; black keeps the page from flashing through while it starts and stops.
            .maybe(playing, |el| el.child(rect().position(Position::new_absolute().top(0.).left(0.)).width(Size::percent(100.)).height(Size::percent(100.)).layer(Layer::OverlayLevel(3)).child(player_ui::Player)))
    }
}

/// The player on screen: its video and what the controls show.
#[derive(Clone)]
pub struct NowPlaying {
    pub video: Rc<video::VideoRenderer>,
    pub title: String,
    pub episode: String,
    pub has_next: bool,
    pub logo: Option<String>,
    pub background: Option<String>,
}

impl PartialEq for NowPlaying {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.video, &other.video)
    }
}
