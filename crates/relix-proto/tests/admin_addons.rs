use relix_proto::addon::{Addon, manifest_url, with_admin_addons};

fn addon(id: &str, host: &str) -> Addon {
    let manifest = serde_json::from_value(serde_json::json!({ "id": id, "version": "1.0.0", "name": id })).unwrap();
    Addon { manifest_url: manifest_url(&format!("https://{host}/manifest.json")).unwrap(), manifest }
}

fn ids(list: &[Addon]) -> Vec<(&str, &str)> {
    list.iter().map(|a| (a.manifest.id.as_str(), a.manifest_url.host_str().unwrap())).collect()
}

#[test]
fn admin_addons_follow_the_users_own_in_admin_order() {
    let user = [addon("cinemeta", "user.cinemeta")];
    let admin = [addon("debrid", "admin.debrid"), addon("anime", "admin.anime")];
    assert_eq!(
        ids(&with_admin_addons(&user, &admin, true)),
        [("cinemeta", "user.cinemeta"), ("debrid", "admin.debrid"), ("anime", "admin.anime")]
    );
}

#[test]
fn the_users_own_copy_of_an_addon_wins() {
    // The user configured Torrentio themselves (their own keys); the admin's copy is skipped.
    let user = [addon("torrentio", "user.torrentio")];
    let admin = [addon("torrentio", "admin.torrentio"), addon("anime", "admin.anime")];
    assert_eq!(ids(&with_admin_addons(&user, &admin, true)), [("torrentio", "user.torrentio"), ("anime", "admin.anime")]);
}

#[test]
fn switched_off_means_only_the_users_addons() {
    let user = [addon("cinemeta", "user.cinemeta")];
    let admin = [addon("debrid", "admin.debrid")];
    assert_eq!(ids(&with_admin_addons(&user, &admin, false)), [("cinemeta", "user.cinemeta")]);
}
