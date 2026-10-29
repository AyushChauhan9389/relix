mod detail;
mod home;
mod player;

use freya::prelude::*;
use freya::router::*;
use relix_proto::addon::{Addon, CatalogResponse, Manifest, ResourceRequest, manifest_url};
use tokio::task::JoinSet;

use detail::Detail;
use home::{Home, Row};

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

/// App-wide data every screen reads.
#[derive(Clone)]
pub struct Ctx {
    pub http: reqwest::Client,
    pub addons: State<Vec<Addon>>,
    pub rows: State<Vec<Row>>,
}

pub async fn get<T: serde::de::DeserializeOwned>(http: reqwest::Client, url: String) -> reqwest::Result<T> {
    http.get(url).send().await?.error_for_status()?.json().await
}

fn app() -> impl IntoElement {
    let addons = use_state(Vec::<Addon>::new);
    let rows = use_state(Vec::<Row>::new);
    let ctx = use_provide_context(|| Ctx { http: reqwest::Client::new(), addons, rows });

    use_hook(move || {
        spawn(load_home(ctx));
    });

    // `--route /title/movie/tt1375666` opens a screen directly, for development.
    let initial = std::env::args()
        .skip_while(|a| a != "--route")
        .nth(1)
        .and_then(|r| r.parse::<Route>().ok())
        .unwrap_or(Route::Home);
    Router::<Route>::new(move || RouterConfig::default().with_initial_path(initial.clone()))
}

async fn load_home(mut ctx: Ctx) {
    for url in DEFAULT_ADDONS {
        let url = manifest_url(url).unwrap();
        match get::<Manifest>(ctx.http.clone(), url.to_string()).await {
            Ok(manifest) => ctx.addons.write().push(Addon { manifest_url: url, manifest }),
            Err(e) => eprintln!("manifest {url}: {e}"),
        }
    }

    // Rows appear in addon order; each fills in as its catalog answers.
    let addons = ctx.addons.read().clone();
    let mut set = JoinSet::new();
    for addon in &addons {
        for c in addon.manifest.home_catalogs() {
            let name = c.name.clone().unwrap_or_else(|| c.id.clone());
            let kind = if c.r#type == "movie" { "Movies" } else { "Series" };
            let i = ctx.rows.read().len();
            ctx.rows.write().push(Row { title: format!("{name} {kind}"), metas: vec![] });
            let url = ResourceRequest::new("catalog", &c.r#type, &c.id).url(addon);
            let http = ctx.http.clone();
            set.spawn(async move { (i, get::<CatalogResponse>(http, url).await) });
        }
    }
    while let Some(Ok((i, res))) = set.join_next().await {
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
}

#[derive(PartialEq)]
struct Shell;

impl Component for Shell {
    fn render(&self) -> impl IntoElement {
        rect()
            .expanded()
            .background(home::BG)
            .color(Color::WHITE)
            .child(Outlet::<Route>::new())
            .child(home::nav().layer(Layer::Overlay))
    }
}
