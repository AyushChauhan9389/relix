mod addons;
mod browse;
mod detail;
mod focus;
mod home;
mod library;
mod player;
mod search;
mod store;

use std::rc::Rc;

use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{Addon, CatalogResponse, Manifest, ResourceRequest, manifest_url};
use tokio::task::JoinSet;

use addons::Addons;
use browse::Browse;
use detail::Detail;
use focus::{Dir, Registry};
use home::{Home, Row};
use search::Search;
use store::Store;

/// Preinstalled addons (PLAN.md: defaults).
const DEFAULT_ADDONS: [&str; 2] =
    ["https://v3-cinemeta.strem.io/manifest.json", "https://opensubtitles-v3.strem.io/manifest.json"];

fn main() {
    // ponytail: phase 0 playback spike, kept for quick manual checks.
    if let Some(url) = std::env::args().skip_while(|a| a != "--play").nth(1) {
        match player::play(&url, &Default::default(), 0.) {
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
    /// Id of the profile every record belongs to (one implicit profile for now).
    pub profile: String,
    /// The installed addons, in order.
    pub addons: State<Vec<Addon>>,
    pub rows: State<Vec<Row>>,
}

impl Ctx {
    /// Re-reads Continue Watching and My List into Home, leaving catalog rows as they are.
    pub fn refresh_local_rows(&self) {
        let mut rows = self.rows;
        let mut next = library::local_rows(&self.store, &self.profile);
        next.extend(rows.peek().iter().filter(|r| !r.id.starts_with("local/")).cloned());
        rows.set(next);
    }

    /// Persists the addon list and rebuilds Home's rows from it.
    pub fn save_addons(&self, addons: Vec<Addon>) {
        let mut this = self.clone();
        if let Err(e) = self.store.put(&self.profile, "addons", "list", &addons) {
            eprintln!("saving addons: {e}");
        }
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
    let addons = ctx.addons.peek().clone();
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
        #[route("/search")]
        Search,
        #[route("/browse/:kind")]
        Browse { kind: String },
}

#[derive(PartialEq)]
struct Shell;

impl Component for Shell {
    fn render(&self) -> impl IntoElement {
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
            .child(home::nav().layer(Layer::Overlay))
    }
}
