use relix_server::cli::{AdminCommand, parse_admin};

fn parse(args: &[&str]) -> Result<AdminCommand, String> {
    parse_admin(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
}

#[test]
fn add_takes_a_manifest_link_and_normalizes_stremio_links() {
    match parse(&["add", "stremio://torrentio.strem.fun/realdebrid=KEY/manifest.json"]).unwrap() {
        AdminCommand::Add(url) => assert_eq!(url.as_str(), "https://torrentio.strem.fun/realdebrid=KEY/manifest.json"),
        other => panic!("expected add, got {other:?}"),
    }
    assert!(parse(&["add", "https://example.com/not-a-manifest"]).is_err());
    assert!(parse(&["add"]).is_err());
}

#[test]
fn list_and_remove() {
    assert_eq!(parse(&["list"]).unwrap(), AdminCommand::List);
    assert_eq!(parse(&["remove", "7"]).unwrap(), AdminCommand::Remove(7));
    assert!(parse(&["remove", "seven"]).is_err());
}

#[test]
fn anything_else_explains_usage() {
    let err = parse(&["frobnicate"]).unwrap_err();
    assert!(err.contains("add <manifest-url>") && err.contains("list") && err.contains("remove <id>"));
}
