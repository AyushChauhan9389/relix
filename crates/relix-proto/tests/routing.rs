use relix_proto::addon::{Addon, ResourceRequest, manifest_url, route};

fn addon(name: &str, url: &str) -> Addon {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    Addon {
        manifest_url: manifest_url(url).unwrap(),
        manifest: serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap(),
    }
}

fn addons() -> Vec<Addon> {
    vec![
        addon("cinemeta", "https://v3-cinemeta.strem.io/manifest.json"),
        addon("torrentio", "https://torrentio.strem.fun/providers=yts|eztv/manifest.json"),
        addon("opensubtitles", "https://opensubtitles-v3.strem.io/manifest.json"),
    ]
}

fn urls(req: &ResourceRequest) -> Vec<String> {
    route(&addons(), req).map(|(_, url)| url).collect()
}

#[test]
fn episode_streams_go_to_torrentio_only() {
    let req = ResourceRequest::new("stream", "series", "tt0903747:1:2");
    assert_eq!(urls(&req), ["https://torrentio.strem.fun/providers=yts|eztv/stream/series/tt0903747:1:2.json"]);
}

#[test]
fn meta_goes_to_cinemeta_only() {
    let req = ResourceRequest::new("meta", "movie", "tt1375666");
    assert_eq!(urls(&req), ["https://v3-cinemeta.strem.io/meta/movie/tt1375666.json"]);
}

#[test]
fn id_prefix_must_match() {
    // Cinemeta only knows "tt" ids; Torrentio's stream resource lists "kitsu".
    assert!(urls(&ResourceRequest::new("meta", "anime", "kitsu:1")).is_empty());
    assert_eq!(
        urls(&ResourceRequest::new("stream", "anime", "kitsu:1:3")),
        ["https://torrentio.strem.fun/providers=yts|eztv/stream/anime/kitsu:1:3.json"]
    );
}

#[test]
fn resource_level_types_override_manifest_types() {
    // Torrentio's manifest lists "other", but its stream resource doesn't.
    assert!(urls(&ResourceRequest::new("stream", "other", "tt1")).is_empty());
}

#[test]
fn subtitles_go_to_opensubtitles() {
    let req = ResourceRequest::new("subtitles", "movie", "tt1375666");
    assert_eq!(urls(&req), ["https://opensubtitles-v3.strem.io/subtitles/movie/tt1375666.json"]);
}

#[test]
fn catalog_without_extra() {
    let req = ResourceRequest::new("catalog", "movie", "top");
    assert_eq!(urls(&req), ["https://v3-cinemeta.strem.io/catalog/movie/top.json"]);
}

#[test]
fn catalog_extra_is_encoded_in_path() {
    let req = ResourceRequest::new("catalog", "movie", "top")
        .extra("search", "the office & co")
        .extra("skip", "100");
    assert_eq!(
        urls(&req),
        ["https://v3-cinemeta.strem.io/catalog/movie/top/search=the%20office%20%26%20co&skip=100.json"]
    );
}

#[test]
fn catalog_must_be_declared_by_the_addon() {
    assert!(urls(&ResourceRequest::new("catalog", "movie", "nope")).is_empty());
    assert!(urls(&ResourceRequest::new("catalog", "tv", "top")).is_empty());
}

#[test]
fn subtitles_route_with_the_stream_filename() {
    let addons = addons();
    let urls: Vec<String> = relix_proto::addon::subtitle_requests(&addons, "movie", "tt1375666", Some("Inception 2010.mkv"))
        .into_iter()
        .map(|(i, req)| req.url(&addons[i]))
        .collect();
    assert_eq!(urls, ["https://opensubtitles-v3.strem.io/subtitles/movie/tt1375666/filename=Inception%202010.mkv.json"]);
}

#[test]
fn subtitles_without_a_filename_have_no_extra() {
    let addons = addons();
    let urls: Vec<String> = relix_proto::addon::subtitle_requests(&addons, "series", "tt0903747:1:2", None)
        .into_iter()
        .map(|(i, req)| req.url(&addons[i]))
        .collect();
    assert_eq!(urls, ["https://opensubtitles-v3.strem.io/subtitles/series/tt0903747:1:2.json"]);
}
