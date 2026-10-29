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

fn cinemeta() -> Addon {
    addon("cinemeta", "https://v3-cinemeta.strem.io/manifest.json")
}

#[test]
fn discover_with_genre_and_paging() {
    let a = cinemeta();
    let top = a.manifest.catalogs.iter().find(|c| c.r#type == "movie" && c.id == "top").unwrap();
    assert!(top.genres().contains(&"Action".to_owned()));
    assert_eq!(top.discover_request(Some("Action"), 100).url(&a), "https://v3-cinemeta.strem.io/catalog/movie/top/genre=Action&skip=100.json");
    assert_eq!(top.discover_request(None, 0).url(&a), "https://v3-cinemeta.strem.io/catalog/movie/top.json");
}

#[test]
fn required_genre_defaults_to_the_first_option() {
    let a = cinemeta();
    let new = a.manifest.catalogs.iter().find(|c| c.r#type == "movie" && c.id == "year").unwrap();
    let first = new.genres()[0].clone();
    assert_eq!(new.discover_request(None, 0).url(&a), format!("https://v3-cinemeta.strem.io/catalog/movie/year/genre={first}.json"));
}

#[test]
fn discover_lists_catalogs_of_a_type_that_need_no_other_input() {
    let addons = [cinemeta()];
    let ids: Vec<&str> = relix_proto::addon::discover_catalogs(&addons, "series").map(|(_, c)| c.id.as_str()).collect();
    assert_eq!(ids, ["top", "year", "imdbRating"], "last-videos/calendar-videos need ids, not browsable");
}
