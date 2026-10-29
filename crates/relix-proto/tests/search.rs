use relix_proto::addon::{Addon, manifest_url, search_requests};

fn addon(name: &str, url: &str) -> Addon {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    Addon {
        manifest_url: manifest_url(url).unwrap(),
        manifest: serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap(),
    }
}

#[test]
fn search_hits_every_searchable_catalog_in_addon_order() {
    let addons = [
        addon("torrentio", "https://torrentio.strem.fun/manifest.json"),
        addon("cinemeta", "https://v3-cinemeta.strem.io/manifest.json"),
    ];
    let urls: Vec<String> =
        search_requests(&addons, "the office").into_iter().map(|(a, _, req)| req.url(&addons[a])).collect();
    assert_eq!(
        urls,
        [
            "https://v3-cinemeta.strem.io/catalog/movie/top/search=the%20office.json",
            "https://v3-cinemeta.strem.io/catalog/series/top/search=the%20office.json",
        ]
    );
}

#[test]
fn blank_queries_search_nothing() {
    let addons = [addon("cinemeta", "https://v3-cinemeta.strem.io/manifest.json")];
    assert!(search_requests(&addons, "   ").is_empty());
}
