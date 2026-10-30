use relix_proto::addon::{CatalogResponse, MetaResponse, StreamSource, StreamsResponse, SubtitlesResponse};
use serde::de::DeserializeOwned;

fn fixture<T: DeserializeOwned>(name: &str) -> T {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn catalog_metas_and_cache_hints() {
    let r: CatalogResponse = fixture("catalog_movie_top");
    assert_eq!(r.metas.len(), 3);
    assert!(r.metas.iter().all(|m| m.id.starts_with("tt") && m.r#type == "movie" && m.poster.is_some()));
    assert_eq!(r.cache.cache_max_age, Some(14400));
    assert_eq!(r.cache.stale_revalidate, Some(3600));
}

#[test]
fn series_meta_with_episodes() {
    let r: MetaResponse = fixture("meta_breaking_bad");
    assert_eq!(r.meta.name, "Breaking Bad");
    assert!(r.meta.background.is_some() && r.meta.logo.is_some());
    let ep = &r.meta.videos[0];
    assert_eq!((ep.id.as_str(), ep.season, ep.episode), ("tt0903747:1:1", Some(1), Some(1)));
    assert_eq!(ep.title(), "Pilot");
}

#[test]
fn torrentio_streams_are_torrents() {
    let r: StreamsResponse = fixture("streams_torrentio");
    assert_eq!(r.streams.len(), 3);
    let s = &r.streams[0];
    assert_eq!(s.name.as_deref(), Some("Torrentio\n4k"));
    assert!(matches!(
        &s.source,
        StreamSource::Torrent { info_hash, file_idx: Some(0), .. } if info_hash == "a584351126afcf50d32be068f3c9a62550d41ecc"
    ));
    assert_eq!(s.behavior_hints.filename.as_deref(), Some("Breaking Bad  S01E01  Pilot.mkv"));
    assert_eq!(r.cache.stale_error, Some(604800));
}

#[test]
fn every_stream_variant_parses() {
    let r: StreamsResponse = fixture("streams_variants");
    let [url, yt, ext, torrent] = &r.streams[..] else { panic!("expected 4 streams") };

    assert!(matches!(&url.source, StreamSource::Url { url } if url == "https://cdn.example.com/movie.mkv"));
    assert!(url.behavior_hints.not_web_ready);
    assert_eq!(url.behavior_hints.binge_group.as_deref(), Some("debrid-1080p"));
    let headers = &url.behavior_hints.proxy_headers.as_ref().unwrap().request;
    assert_eq!(headers.get("Referer").map(String::as_str), Some("https://example.com/"));

    assert!(matches!(&yt.source, StreamSource::YouTube { yt_id } if yt_id == "YoHD9XEInc0"));
    assert!(matches!(&ext.source, StreamSource::External { external_url } if external_url == "https://example.com/watch/1"));
    assert!(matches!(&torrent.source, StreamSource::Torrent { file_idx: None, sources, .. } if sources.len() == 2));
}

#[test]
fn opensubtitles_subtitles() {
    let r: SubtitlesResponse = fixture("subtitles_opensubtitles");
    assert_eq!(r.subtitles.len(), 3);
    assert_eq!(r.subtitles[0].lang, "eng");
    assert!(r.subtitles[0].url.starts_with("https://"));
}

#[test]
fn meta_people_and_rating() {
    let r: MetaResponse = fixture("meta_breaking_bad");
    assert_eq!(r.meta.rating().as_deref(), Some("9.5"));
    assert_eq!(r.meta.people("cast", "Cast"), ["Bryan Cranston", "Aaron Paul", "Anna Gunn"]);
    assert_eq!(r.meta.people("writer", "Writers"), ["Vince Gilligan"]);
    assert!(r.meta.people("director", "Directors").is_empty());
}

#[test]
fn people_come_as_a_list_a_string_or_only_links() {
    let meta = |v: serde_json::Value| -> relix_proto::addon::Meta { serde_json::from_value(v).unwrap() };
    let base = serde_json::json!({ "id": "tt1", "type": "movie", "name": "X" });
    let mut as_string = base.clone();
    as_string["director"] = "Christopher Nolan".into();
    as_string["imdbRating"] = 8.8.into();
    assert_eq!(meta(as_string.clone()).people("director", "Directors"), ["Christopher Nolan"]);
    assert_eq!(meta(as_string).rating().as_deref(), Some("8.8"));

    let mut links_only = base;
    links_only["links"] = serde_json::json!([{ "name": "Christopher Nolan", "category": "Directors", "url": "stremio:///search?search=Nolan" }]);
    assert_eq!(meta(links_only).people("director", "Directors"), ["Christopher Nolan"]);
}
