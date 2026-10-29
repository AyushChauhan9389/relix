mod addons;
mod detail;
mod focus;
mod home;
mod player;
mod profiles;
mod store;

use std::rc::Rc;

use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{Addon, CatalogResponse, Manifest, ResourceRequest, manifest_url};
use tokio::task::JoinSet;

use addons::Addons;
use detail::Detail;
use focus::{Dir, Registry};
use home::{Home, Row};
use profiles::Profiles;
use store::{Profile, Store};

/// Preinstalled addons (PLAN.md: defaults).
const DEFAULT_ADDONS: [&str; 2] =
    ["https://v3-cinemeta.strem.io/manifest.json", "https://opensubtitles-v3.strem.io/manifest.json"];

fn main() {
    // ponytail: phase 0 playback spike, kept for quick manual checks.
    if let Some(url) = std::env::args().skip_while(|a| a != "--play").nth(1) {
        match player::play(&url, &Default::default()) {
            Ok(p) => println!("stopped at {:.1}s of {:.1}s", p.position, p.duration),
            Err(e) => eprintln!("mpv: {e}"),
        }
        return;
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let _rt = rt.enter();
    launch(LaunchConfig::new().with_window(WindowConfig::new(app).with_title("Relix").with_size(1440., 900.)))
}

/// Where Relix keeps its database: the usual per-user app data folder of each OS.
fn data_dir() -> std::path::PathBuf {
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
    pub profile: State<Profile>,
    /// The current profile's addon list, in order.
    pub addons: State<Vec<Addon>>,
    pub rows: State<Vec<Row>>,
}

impl Ctx {
    /// The addons the current profile may use (kids filtering applied).
    pub fn visible_addons(&self) -> Vec<Addon> {
        self.profile.read().visible_addons(&self.addons.read()).into_iter().cloned().collect()
    }

    /// Persists the addon list and rebuilds Home's rows from it.
    /// For a kids profile, whatever the parent saves here is what the kid is allowed.
    pub fn save_addons(&self, addons: Vec<Addon>) {
        let mut this = self.clone();
        let mut profile = self.profile.peek().clone();
        if profile.kids {
            profile.allowed_addons = addons.iter().map(|a| a.manifest_url.to_string()).collect();
            if let Err(e) = self.store.save_profile(&profile) {
                eprintln!("saving profile: {e}");
            }
            this.profile.set(profile.clone());
        }
        if let Err(e) = self.store.put(&profile.id, "addons", "list", &addons) {
            eprintln!("saving addons: {e}");
        }
        this.addons.set(addons);
        spawn(load_rows(this));
    }

    /// Makes `profile` current: its addons, then its Home rows.
    pub fn switch_profile(&self, profile: Profile) {
        let mut this = self.clone();
        let saved = self.store.get::<Vec<Addon>>(&profile.id, "addons", "list").ok().flatten();
        this.profile.set(profile);
        this.rows.set(vec![]);
        this.addons.set(saved.clone().unwrap_or_default());
        spawn(async move {
            match saved {
                Some(_) => load_rows(this).await,
                None => install_defaults(&this).await,
            }
        });
    }
}

pub async fn get<T: serde::de::DeserializeOwned>(http: reqwest::Client, url: String) -> reqwest::Result<T> {
    http.get(url).send().await?.error_for_status()?.json().await
}

fn app() -> impl IntoElement {
    use_init_theme(dark_theme);
    let store = use_hook(|| Rc::new(Store::open(&data_dir().join("relix.db")).expect("open database")));
    let profiles = use_hook(|| store.profiles().unwrap_or_default());
    let profile = use_state(|| {
        let first = profiles.first().cloned();
        first.unwrap_or_else(|| store.create_profile("Me", false).expect("create profile"))
    });
    let saved = store.get::<Vec<Addon>>(&profile.peek().id, "addons", "list").ok().flatten();
    let addons = use_state(|| saved.clone().unwrap_or_default());
    let rows = use_state(Vec::<Row>::new);
    let ctx = use_provide_context(|| Ctx { http: reqwest::Client::new(), store: store.clone(), profile, addons, rows });

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
        .and_then(|r| r.parse::<Route>().ok())
        // With several people on this device, ask who's watching first.
        .unwrap_or(if profiles.len() > 1 { Route::Profiles } else { Route::Home });
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
    let addons = ctx.visible_addons();
    let mut rows = vec![];
    let mut set = JoinSet::new();
    for addon in &addons {
        for c in addon.manifest.home_catalogs() {
            let name = c.name.clone().unwrap_or_else(|| c.id.clone());
            let kind = match c.r#type.as_str() {
                "movie" => "Movies".to_owned(),
                "series" => "Series".to_owned(),
                other => other.to_owned(),
            };
            let i = rows.len();
            rows.push(Row { title: format!("{name} {kind}"), metas: vec![] });
            let url = ResourceRequest::new("catalog", &c.r#type, &c.id).url(addon);
            let http = ctx.http.clone();
            set.spawn(async move { (i, get::<CatalogResponse>(http, url).await) });
        }
    }
    let titles: Vec<String> = rows.iter().map(|r| r.title.clone()).collect();
    ctx.rows.set(rows);
    while let Some(Ok((i, res))) = set.join_next().await {
        // A newer load may have replaced the rows meanwhile; only fill our own.
        if ctx.rows.peek().get(i).map(|r| &r.title) != titles.get(i) {
            return;
        }
        match res {
            Ok(r) => ctx.rows.write()[i].metas = r.metas,
            Err(e) => eprintln!("catalog {i}: {e}"),
        }
    }
}

#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Shell)]
        #[route("/")]
        Home,
        #[route("/title/:kind/:id")]
        Detail { kind: String, id: String },
        #[route("/addons")]
        Addons,
        #[route("/profiles")]
        Profiles,
}

#[derive(PartialEq)]
struct Shell;

impl Component for Shell {
    fn render(&self) -> impl IntoElement {
        let profile = use_consume::<Ctx>().profile.read().clone();
        let picking = RouterContext::get().current::<Route>() == Route::Profiles;
        let registry = use_provide_context(Registry::default);
        rect()
            .expanded()
            .on_global_key_down(move |e: Event<KeyboardEventData>| {
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
            .child(Outlet::<Route>::new())
            .maybe(!picking, |el| el.child(home::nav(&profile).layer(Layer::Overlay)))
    }
}
