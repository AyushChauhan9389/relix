use relix_proto::addon::{MetaResponse, Stream, StreamSource, Video, seasons, stream_groups};

fn video(id: &str, season: Option<u32>, episode: Option<u32>) -> Video {
    serde_json::from_value(serde_json::json!({ "id": id, "season": season, "episode": episode })).unwrap()
}

#[test]
fn breaking_bad_fixture_groups_into_season_one() {
    let path = format!("{}/tests/fixtures/meta_breaking_bad.json", env!("CARGO_MANIFEST_DIR"));
    let r: MetaResponse = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let s = seasons(&r.meta.videos);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].0, 1);
    assert_eq!(s[0].1.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["tt0903747:1:1", "tt0903747:1:2", "tt0903747:1:3"]);
}

#[test]
fn seasons_sorted_with_specials_last_and_episodes_in_order() {
    let videos = [
        video("x:2:2", Some(2), Some(2)),
        video("x:0:1", Some(0), Some(1)),
        video("x:1:1", Some(1), Some(1)),
        video("x:2:1", Some(2), Some(1)),
    ];
    let s = seasons(&videos);
    let shape: Vec<(u32, Vec<&str>)> = s.iter().map(|(n, eps)| (*n, eps.iter().map(|v| v.id.as_str()).collect())).collect();
    assert_eq!(shape, [(1, vec!["x:1:1"]), (2, vec!["x:2:1", "x:2:2"]), (0, vec!["x:0:1"])]);
}

#[test]
fn movies_have_no_seasons() {
    assert!(seasons(&[video("tt1", None, None)]).is_empty());
}

fn stream(source: serde_json::Value) -> Stream {
    serde_json::from_value(source).unwrap()
}

#[test]
fn stream_groups_follow_addon_order_not_arrival_order() {
    // Responses arrive as (addon index, streams), in any order.
    let arrived = vec![
        (2, vec![stream(serde_json::json!({ "url": "https://c" }))]),
        (0, vec![stream(serde_json::json!({ "url": "https://a" }))]),
    ];
    let groups = stream_groups(arrived);
    assert_eq!(groups.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 2]);
}

#[test]
fn only_url_streams_are_playable_in_the_mvp() {
    let playable = |v| stream(v).playable_url().map(str::to_owned);
    assert_eq!(playable(serde_json::json!({ "url": "https://a/b.mkv" })).as_deref(), Some("https://a/b.mkv"));
    assert_eq!(playable(serde_json::json!({ "infoHash": "abc" })), None);
    assert_eq!(playable(serde_json::json!({ "ytId": "x" })), None);
    assert_eq!(playable(serde_json::json!({ "externalUrl": "https://x" })), None);
    assert!(matches!(stream(serde_json::json!({ "infoHash": "abc" })).source, StreamSource::Torrent { .. }));
}

#[test]
fn torrent_streams_are_dropped_everything_else_kept() {
    let streams = vec![
        stream(serde_json::json!({ "infoHash": "abc", "fileIdx": 0 })),
        stream(serde_json::json!({ "url": "https://debrid/a.mkv" })),
        stream(serde_json::json!({ "externalUrl": "https://x" })),
    ];
    let kept: Vec<_> = relix_proto::addon::without_torrents(streams).into_iter().map(|s| s.source).collect();
    assert!(matches!(kept.as_slice(), [StreamSource::Url { .. }, StreamSource::External { .. }]));
}

fn binge(url: &str, group: Option<&str>) -> Stream {
    stream(serde_json::json!({ "url": url, "behaviorHints": { "bingeGroup": group } }))
}

#[test]
fn next_episode_plays_the_same_release_preferring_the_same_addon() {
    let groups = vec![
        (0, vec![binge("https://a/720", Some("rd-720p")), binge("https://a/1080", Some("rd-1080p"))]),
        (2, vec![binge("https://c/1080", Some("rd-1080p"))]),
    ];
    let pick = |prev_addon, group| relix_proto::addon::pick_binge_stream(&groups, prev_addon, group).map(|(a, s)| (a, s.playable_url().unwrap().to_owned()));
    assert_eq!(pick(2, "rd-1080p"), Some((2, "https://c/1080".into())), "same addon first");
    assert_eq!(pick(1, "rd-1080p"), Some((0, "https://a/1080".into())), "else first in addon order");
    assert_eq!(pick(0, "rd-4k"), None, "no matching release, no auto-play");
}

#[test]
fn torrents_and_ungrouped_streams_never_auto_play() {
    let groups = vec![(0, vec![stream(serde_json::json!({ "infoHash": "x", "behaviorHints": { "bingeGroup": "g" } })), binge("https://a", None)])];
    assert_eq!(relix_proto::addon::pick_binge_stream(&groups, 0, "g").map(|(a, _)| a), None);
}

#[test]
fn preferred_stream_is_the_same_quality_from_the_same_addon_then_that_addon_then_any() {
    let s = |name: &str, url: &str| stream(serde_json::json!({ "name": name, "url": url }));
    let torrent = stream(serde_json::json!({ "name": "4k", "infoHash": "abc" }));
    let groups = vec![
        (0, vec![s("Other\n1080p", "https://o/1080")]),
        (2, vec![torrent, s("Torrentio\n720p", "https://t/720"), s("Torrentio\n4k", "https://t/4k")]),
    ];
    let pick = |addon, name| relix_proto::addon::pick_preferred(&groups, addon, name).map(|(a, s)| (a, s.playable_url().unwrap().to_owned()));

    assert_eq!(pick(Some(2), "Torrentio\n4k"), Some((2, "https://t/4k".into())), "same quality, skipping the torrent");
    assert_eq!(pick(Some(2), "Torrentio\n1080p"), Some((2, "https://t/720".into())), "that addon's first playable");
    assert_eq!(pick(Some(5), "x"), Some((0, "https://o/1080".into())), "preferred addon absent: first playable anywhere");
    assert_eq!(pick(None, ""), Some((0, "https://o/1080".into())));
    assert_eq!(relix_proto::addon::pick_preferred(&[], Some(0), "x").map(|(a, _)| a), None);
}
