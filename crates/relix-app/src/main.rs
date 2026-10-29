mod home;
mod player;

use freya::prelude::*;
use home::{Home, Row};
use relix_proto::addon::{Addon, CatalogResponse, Manifest, ResourceRequest, manifest_url};
use tokio::task::JoinSet;

const CINEMETA: &str = "https://v3-cinemeta.strem.io/manifest.json";

fn main() {
    // ponytail: phase 0 playback spike, replaced by the stream list's Play in phase 1.
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

async fn get<T: serde::de::DeserializeOwned>(http: reqwest::Client, url: String) -> reqwest::Result<T> {
    http.get(url).send().await?.error_for_status()?.json().await
}

fn app() -> impl IntoElement {
    let mut rows = use_state(Vec::<Row>::new);

    use_hook(move || {
        spawn(async move {
            let http = reqwest::Client::new();
            let url = manifest_url(CINEMETA).unwrap();
            let manifest: Manifest = match get(http.clone(), url.to_string()).await {
                Ok(m) => m,
                Err(e) => return eprintln!("manifest: {e}"),
            };
            let addon = Addon { manifest_url: url, manifest };

            // Rows appear in addon order; each fills in as its catalog answers.
            let mut set = JoinSet::new();
            for (i, c) in addon.manifest.home_catalogs().enumerate() {
                let name = c.name.clone().unwrap_or_else(|| c.id.clone());
                let kind = if c.r#type == "movie" { "Movies" } else { "Series" };
                rows.write().push(Row { title: format!("{name} {kind}"), metas: vec![] });
                let url = ResourceRequest::new("catalog", &c.r#type, &c.id).url(&addon);
                let http = http.clone();
                set.spawn(async move { (i, get::<CatalogResponse>(http, url).await) });
            }
            while let Some(Ok((i, res))) = set.join_next().await {
                match res {
                    Ok(r) => rows.write()[i].metas = r.metas,
                    Err(e) => eprintln!("catalog {i}: {e}"),
                }
            }
        });
    });

    Home { rows: rows.read().clone() }
}
