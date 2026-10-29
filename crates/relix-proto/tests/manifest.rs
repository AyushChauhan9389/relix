use relix_proto::addon::Manifest;

fn fixture(name: &str) -> Manifest {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn cinemeta_parses() {
    let m = fixture("cinemeta");
    assert_eq!(m.id, "com.linvo.cinemeta");
    assert_eq!(m.catalogs.len(), 8);
}

#[test]
fn home_skips_catalogs_with_required_extra() {
    // "year" requires genre, last-videos/calendar-videos require ids.
    let m = fixture("cinemeta");
    let home: Vec<_> = m.home_catalogs().map(|c| (c.r#type.as_str(), c.id.as_str())).collect();
    assert_eq!(
        home,
        [
            ("movie", "top"),
            ("series", "top"),
            ("movie", "imdbRating"),
            ("series", "imdbRating"),
        ]
    );
}

#[test]
fn search_catalogs_are_the_ones_with_search_extra() {
    let m = fixture("cinemeta");
    let ids: Vec<_> = m.search_catalogs().map(|c| (c.r#type.as_str(), c.id.as_str())).collect();
    assert_eq!(ids, [("movie", "top"), ("series", "top")]);
}

#[test]
fn torrentio_object_resources_and_hints_parse() {
    let m = fixture("torrentio");
    assert!(m.catalogs.is_empty());
    assert!(m.behavior_hints.configurable);
    assert!(!m.behavior_hints.configuration_required);
    assert!(!m.behavior_hints.adult);
}

#[test]
fn opensubtitles_parses() {
    let m = fixture("opensubtitles");
    assert_eq!(m.types, ["movie", "series"]);
}
