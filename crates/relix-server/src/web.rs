//! The web version of Relix (served at /app, uploaded by the release workflow) and the image relay
//! it loads artwork through: the browser only lets a page read images from hosts that allow it, and
//! some art hosts (metahub) don't.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};

use crate::{ApiError, AppState, plain_name, require_admin, store_file};

/// The web app's files: index.html, the wasm, its scripts and icon.
fn web_dir(s: &AppState) -> std::path::PathBuf {
    s.config.update_dir.join("web")
}

pub async fn upload(State(s): State<AppState>, headers: HeaderMap, Path(name): Path<String>, body: Bytes) -> Result<StatusCode, ApiError> {
    require_admin(&s, &headers)?;
    store_file(&web_dir(&s), &name, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// /app: relative links in index.html need the trailing slash.
pub async fn app_root() -> Redirect {
    Redirect::permanent("/app/")
}

pub async fn app_index(s: State<AppState>, headers: HeaderMap) -> Response {
    app_file(s, headers, Path("index.html".into())).await
}

/// A file of the web app. Revalidated on every load (ETag), so a release shows up at once and an
/// unchanged 14 MB wasm isn't downloaded again.
pub async fn app_file(State(s): State<AppState>, headers: HeaderMap, Path(name): Path<String>) -> Response {
    if !plain_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = web_dir(&s).join(&name);
    let Ok(meta) = tokio::fs::metadata(&path).await else { return StatusCode::NOT_FOUND.into_response() };
    let modified = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let etag = format!("\"{}-{modified}\"", meta.len());
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    let Ok(bytes) = tokio::fs::read(&path).await else { return StatusCode::NOT_FOUND.into_response() };
    let kind = match name.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("map") | Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "no-cache"), (header::ETAG, etag.as_str())], bytes).into_response()
}

#[derive(serde::Deserialize)]
pub struct ImageQuery {
    u: String,
}

/// Art hosts addons use (Cinemeta's metahub and the like). The relay isn't an open proxy: only these.
const IMAGE_HOSTS: &[&str] = &[
    "images.metahub.space",
    "episodes.metahub.space",
    "live.metahub.space",
    "image.tmdb.org",
    "m.media-amazon.com",
    "assets.fanart.tv",
    "artworks.thetvdb.com",
    "www.thetvdb.com",
];

/// Largest image passed on.
const MAX_IMAGE: usize = 10 * 1024 * 1024;

pub async fn image(Query(q): Query<ImageQuery>) -> Response {
    let Some(url) = url::Url::parse(&q.u).ok().filter(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| IMAGE_HOSTS.contains(&h))) else {
        return (StatusCode::BAD_REQUEST, "not an image host Relix relays").into_response();
    };
    static CLIENT: std::sync::LazyLock<reqwest::Client> =
        std::sync::LazyLock::new(|| reqwest::Client::builder().timeout(Duration::from_secs(20)).build().expect("HTTP client"));
    let Ok(res) = CLIENT.get(url).send().await else { return StatusCode::BAD_GATEWAY.into_response() };
    let kind = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    if !res.status().is_success() || !kind.starts_with("image/") || res.content_length().is_some_and(|l| l as usize > MAX_IMAGE) {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    match res.bytes().await {
        Ok(bytes) if bytes.len() <= MAX_IMAGE => {
            ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "public, max-age=86400".to_owned())], bytes).into_response()
        }
        _ => StatusCode::BAD_GATEWAY.into_response(),
    }
}
