// Release builds are a GUI app on Windows: no console window next to Relix.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod addons;
mod browse;
mod detail;
mod focus;
mod home;
mod library;
mod motion;
mod my_list;
mod player;
mod search;
mod store;
mod stremio;
mod sync;

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{Addon, CatalogResponse, Manifest, ResourceRequest, manifest_url, with_admin_addons};
use tokio::task::JoinSet;

use account::Account;
use addons::Addons;
use browse::Browse;
use detail::Detail;
use focus::{Dir, Registry};
use home::{Home, Row};
use my_list::MyList;
use search::Search;
use store::Store;

/// Preinstalled addons (PLAN.md: defaults).
const DEFAULT_ADDONS: [&str; 2] =
    ["https://v3-cinemeta.strem.io/manifest.json", "https://opensubtitles-v3.strem.io/manifest.json"];

fn main() {
    // ponytail: playback check without the UI:
    // `--play <url> [--subs movie/<imdb id>] [--title T] [--episode E] [--has-next] [--start secs]`.
    if let Some(url) = std::env::args().skip_while(|a| a != "--play").nth(1) {
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
                    Err(e) => eprintln!("subtitles: {e}"),
                }
            });
        }
        let overlay = player::Overlay {
            title: std::env::args().skip_while(|a| a != "--title").nth(1).unwrap_or_default(),
            episode: std::env::args().skip_while(|a| a != "--episode").nth(1).unwrap_or_default(),
            has_next: std::env::args().any(|a| a == "--has-next"),
        };
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
        match player::play(&url, &Default::default(), start, &overlay, None, inbox, |_| {}) {
            Ok((p, ended)) => println!("stopped at {:.1}s of {:.1}s ({ended:?})", p.position, p.duration),
            Err(e) => eprintln!("mpv: {e}"),
        }
        return;
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let _rt = rt.enter();
    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(app)
                .with_title("Relix")
                .with_size(1440., 900.)
                .with_window_handle(|window| {
                    let _ = NATIVE_WINDOW.set(native_id(window));
                })
                .with_window_attributes(|attrs, _| if test_offscreen() { offscreen(attrs) } else { attrs }),
        ),
    )
}

/// `RELIX_TEST_OFFSCREEN=1`: for automated checks on a developer's machine. The window opens
/// off-screen, unfocused and off the taskbar, and playback is muted, so nothing interrupts them.
pub fn test_offscreen() -> bool {
    std::env::var_os("RELIX_TEST_OFFSCREEN").is_some()
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

/// Relix's own window as mpv's `wid`, so video plays inside it. `None` where mpv can't embed.
static NATIVE_WINDOW: OnceLock<Option<i64>> = OnceLock::new();

pub fn native_window() -> Option<i64> {
    NATIVE_WINDOW.get().copied().flatten()
}

fn native_id(window: &freya::winit::window::Window) -> Option<i64> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as i64),
        RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr() as i64),
        RawWindowHandle::Xlib(h) => Some(h.window as i64),
        RawWindowHandle::Xcb(h) => Some(h.window.get() as i64),
        // Wayland has no foreign-window embedding; mpv opens its own window there.
        _ => None,
    }
}

/// Where Relix keeps its database: `data` next to the executable for a portable copy (a file
/// named `portable` sits beside it), otherwise the usual per-user app data folder of each OS.
fn data_dir() -> std::path::PathBuf {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(std::path::Path::to_path_buf));
    if let Some(dir) = exe_dir.filter(|d| d.join("portable").exists()) {
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
    /// Wakes the sync loop (sign-in, local changes).
    pub sync: Arc<tokio::sync::Notify>,
    /// Last sync outcome, for the Account screen.
    pub sync_status: State<String>,
    /// The running player, while something plays.
    pub player: State<Option<std::sync::mpsc::Sender<player::ToPlayer>>>,
    pub fullscreen: State<bool>,
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
            eprintln!("saving setting: {e}");
        }
        let mut enabled = self.admin_enabled;
        enabled.set(on);
        spawn(load_rows(self.clone()));
        self.sync_soon();
    }

    /// Makes Relix's window fullscreen, or back.
    pub fn toggle_fullscreen(&self) {
        use freya::winit::window::Fullscreen;
        let mut fullscreen = self.fullscreen;
        let on = !*fullscreen.peek();
        fullscreen.set(on);
        Platform::get().with_window(Platform::window_id(), move |w| {
            w.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
        });
    }

    /// Esc while playing: leave fullscreen first, then stop.
    pub fn player_escape(&self) {
        if *self.fullscreen.peek() {
            self.toggle_fullscreen();
        } else if let Some(p) = self.player.peek().as_ref() {
            let _ = p.send(player::ToPlayer::Stop);
        }
    }

    /// Asks the sync loop to run soon (no-op when signed out).
    pub fn sync_soon(&self) {
        self.sync.notify_one();
    }

    /// Persists the addon list and rebuilds Home's rows from it.
    pub fn save_addons(&self, addons: Vec<Addon>) {
        let mut this = self.clone();
        if let Err(e) = self.store.put(&self.profile, "addons", "list", &addons) {
            eprintln!("saving addons: {e}");
        }
        self.sync_soon();
        this.addons.set(addons);
        spawn(load_rows(this));
    }
}

pub async fn get<T: serde::de::DeserializeOwned>(http: reqwest::Client, url: String) -> reqwest::Result<T> {
    http.get(url).send().await?.error_for_status()?.json().await
}

fn app() -> impl IntoElement {
    use_init_theme(dark_theme);
    let store = use_hook(|| Rc::new(Store::open(&data_dir().join("relix.db")).expect("open database")));
    let profile = use_hook(|| {
        let first = store.profiles().ok().and_then(|p| p.into_iter().next());
        first.unwrap_or_else(|| store.create_profile("Me").expect("create profile")).id
    });
    let saved = store.get::<Vec<Addon>>(&profile, "addons", "list").ok().flatten();
    let addons = use_state(|| saved.clone().unwrap_or_default());
    let rows = use_state(Vec::<Row>::new);
    let sync_status = use_state(String::new);
    let player = use_state(|| None);
    let fullscreen = use_state(|| false);
    let admin_addons = use_state(|| {
        let cached = store.meta("admin_addons").ok().flatten();
        cached.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default()
    });
    let admin_enabled =
        use_state(|| store.get::<bool>(&profile, "settings", "admin_addons_enabled").ok().flatten().unwrap_or(true));
    let ctx = use_provide_context(|| Ctx {
        http: reqwest::Client::new(),
        store: store.clone(),
        profile,
        addons,
        admin_addons,
        admin_enabled,
        rows,
        sync: Arc::default(),
        sync_status,
        player,
        fullscreen,
    });

    use_hook({
        let ctx = ctx.clone();
        move || {
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
        .and_then(|r| r.parse::<Route>().inspect_err(|e| eprintln!("--route {r}: {e}")).ok())
        .unwrap_or(Route::Home);
    Router::<Route>::new(move || RouterConfig::default().with_initial_path(initial.clone()))
}

async fn install_defaults(ctx: &Ctx) {
    let mut addons = vec![];
    for url in DEFAULT_ADDONS {
        let url = manifest_url(url).unwrap();
        match get::<Manifest>(ctx.http.clone(), url.to_string()).await {
            Ok(manifest) => addons.push(Addon { manifest_url: url, manifest }),
            Err(e) => eprintln!("manifest {url}: {e}"),
        }
    }
    ctx.save_addons(addons);
}

/// Rebuilds Home's rows: one per home catalog, in addon order, each filled as its catalog answers.
pub async fn load_rows(mut ctx: Ctx) {
    let addons = ctx.all_addons();
    let mut rows = library::local_rows(&ctx.store, &ctx.profile);
    let mut set = JoinSet::new();
    for addon in &addons {
        for c in addon.manifest.home_catalogs() {
            let name = c.name.clone().unwrap_or_else(|| c.id.clone());
            let kind = match c.r#type.as_str() {
                "movie" => "Movies".to_owned(),
                "series" => "Series".to_owned(),
                other => other.to_owned(),
            };
            let id = format!("{}/{}/{}", addon.manifest.id, c.r#type, c.id);
            rows.push(Row { id: id.clone(), title: format!("{name} {kind}"), metas: vec![], progress: vec![] });
            let url = ResourceRequest::new("catalog", &c.r#type, &c.id).url(addon);
            let http = ctx.http.clone();
            set.spawn(async move { (id, get::<CatalogResponse>(http, url).await) });
        }
    }
    ctx.rows.set(rows);
    while let Some(Ok((id, res))) = set.join_next().await {
        // Rows may have been rebuilt meanwhile, so find ours by id.
        let Some(i) = ctx.rows.peek().iter().position(|r| r.id == id) else { continue };
        match res {
            Ok(r) => ctx.rows.write()[i].metas = r.metas,
            Err(e) => eprintln!("catalog {id}: {e}"),
        }
    }
}

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
        #[route("/account")]
        Account,
        #[route("/my-list")]
        MyList,
}

#[derive(PartialEq)]
struct Shell;

impl Component for Shell {
    fn render(&self) -> impl IntoElement {
        let registry = use_provide_context(Registry::default);
        let ctx = use_consume::<Ctx>();
        let playing = ctx.player.read().is_some();
        rect()
            .expanded()
            .on_global_key_down(move |e: Event<KeyboardEventData>| {
                // While video plays, Relix's keys only stop it (mpv's controls take the rest).
                if ctx.player.peek().is_some() {
                    if e.key == Key::Named(NamedKey::Escape) {
                        ctx.player_escape();
                    }
                    return;
                }
                let dir = match e.key {
                    Key::Named(NamedKey::ArrowUp) => Dir::Up,
                    Key::Named(NamedKey::ArrowDown) => Dir::Down,
                    Key::Named(NamedKey::ArrowLeft) => Dir::Left,
                    Key::Named(NamedKey::ArrowRight) => Dir::Right,
                    Key::Named(NamedKey::Escape) => {
                        let router = RouterContext::get();
                        if router.can_go_back() {
                            router.go_back();
                        }
                        return;
                    }
                    _ => return,
                };
                registry.navigate(dir);
            })
            .background(home::BG)
            .color(Color::WHITE)
            // Every screen fades and rises in; the key replays it on each navigation.
            .child(motion::FadeIn::page(Outlet::<Route>::new()).key(RouterContext::get().current::<Route>().to_string()))
            .maybe(!playing, |el| {
                el.child(home::nav(home::active_link(&RouterContext::get().current::<Route>())).layer(Layer::Overlay))
            })
            // mpv draws over this; black keeps the page from flashing through while it starts and stops.
            .maybe(playing, |el| el.child(home::layer().background((0, 0, 0)).layer(Layer::Overlay)))
    }
}
