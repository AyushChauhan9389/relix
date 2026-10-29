use relix_proto::addon::{base_url, manifest_url};

#[test]
fn stremio_link_becomes_https() {
    let url = manifest_url("stremio://torrentio.strem.fun/providers=yts/manifest.json").unwrap();
    assert_eq!(url.as_str(), "https://torrentio.strem.fun/providers=yts/manifest.json");
}

#[test]
fn https_url_is_kept() {
    let url = manifest_url("  https://v3-cinemeta.strem.io/manifest.json ").unwrap();
    assert_eq!(url.as_str(), "https://v3-cinemeta.strem.io/manifest.json");
}

#[test]
fn rejects_urls_that_are_not_manifests() {
    assert!(manifest_url("https://example.com/").is_err());
    assert!(manifest_url("ftp://example.com/manifest.json").is_err());
    assert!(manifest_url("not a url").is_err());
}

#[test]
fn base_url_strips_manifest_json() {
    let url = manifest_url("https://torrentio.strem.fun/providers=yts|eztv/manifest.json").unwrap();
    assert_eq!(base_url(&url), "https://torrentio.strem.fun/providers=yts|eztv");
}
