//! Import and export addons through a Stremio account (Stremio's account API at api.strem.io).

use relix_proto::addon::{Addon, Manifest, manifest_url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::task::JoinSet;

use crate::{Ctx, get};

const API: &str = "https://api.strem.io/api/";
/// Stremio's "Local Files" addon talks to Stremio's own local server; it's useless in Relix.
const STREMIO_LOCAL: &str = "org.stremio.local";

/// An addon as Stremio stores it in an account. `manifest` stays raw JSON so a round trip
/// through Relix never drops fields Relix doesn't model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub transport_url: String,
    #[serde(default)]
    pub transport_name: String,
    pub manifest: Value,
    #[serde(default)]
    pub flags: Flags,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Flags {
    #[serde(default)]
    pub official: bool,
    /// Stremio's built-ins; removing or changing them breaks Stremio.
    #[serde(default)]
    pub protected: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Summary {
    pub added: usize,
    pub updated: usize,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.added, self.updated) {
            (0, 0) => f.write_str("already up to date"),
            (a, u) => write!(f, "{a} added, {u} updated"),
        }
    }
}

fn id_of(manifest: &Value) -> Option<&str> {
    manifest.get("id")?.as_str()
}

/// Merges Stremio's addons into Relix's: Relix's order stays, an addon Relix already has takes
/// Stremio's copy (its configuration), and the rest are appended in Stremio's order.
pub fn import(mine: &[Addon], theirs: &[Descriptor]) -> (Vec<Addon>, Summary) {
    let mut merged = mine.to_vec();
    let mut summary = Summary::default();
    for d in theirs {
        let (Ok(url), Ok(manifest)) = (manifest_url(&d.transport_url), serde_json::from_value::<Manifest>(d.manifest.clone())) else {
            continue;
        };
        if manifest.id == STREMIO_LOCAL {
            continue;
        }
        match merged.iter_mut().find(|a| a.manifest.id == manifest.id) {
            Some(existing) if existing.manifest_url == url => {}
            Some(existing) => {
                *existing = Addon { manifest_url: url, manifest };
                summary.updated += 1;
            }
            None => {
                merged.push(Addon { manifest_url: url, manifest });
                summary.added += 1;
            }
        }
    }
    (merged, summary)
}

/// Merges Relix's addons (with their original manifest JSON) into Stremio's list: Stremio's
/// protected built-ins and Stremio-only addons stay; Relix's are added or updated.
pub fn export(mut theirs: Vec<Descriptor>, mine: Vec<(Addon, Value)>) -> (Vec<Descriptor>, Summary) {
    let mut summary = Summary::default();
    for (addon, raw) in mine {
        let url = addon.manifest_url.to_string();
        match theirs.iter_mut().find(|d| id_of(&d.manifest) == Some(addon.manifest.id.as_str())) {
            Some(d) if d.flags.protected || d.transport_url == url => {}
            Some(d) => {
                d.transport_url = url;
                d.manifest = raw;
                summary.updated += 1;
            }
            None => {
                theirs.push(Descriptor { transport_url: url, transport_name: String::new(), manifest: raw, flags: Flags::default() });
                summary.added += 1;
            }
        }
    }
    (theirs, summary)
}

/// Calls a Stremio API method; Stremio answers `{"result": ...}` or `{"error": {"message": ...}}`.
async fn call<T: DeserializeOwned>(http: &reqwest::Client, method: &str, body: Value) -> Result<T, String> {
    let reply: Value = http
        .post(format!("{API}{method}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Can't reach Stremio: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Unexpected reply from Stremio: {e}"))?;
    if let Some(msg) = reply.pointer("/error/message").and_then(Value::as_str) {
        return Err(format!("Stremio: {msg}"));
    }
    serde_json::from_value(reply.get("result").cloned().unwrap_or(Value::Null)).map_err(|e| format!("Unexpected reply from Stremio: {e}"))
}

/// Signs in to Stremio and keeps only its session key on this device; the password isn't stored.
pub async fn connect(ctx: &Ctx, email: &str, password: &str) -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct LoginResult {
        auth_key: String,
    }
    let r: LoginResult =
        call(&ctx.http, "login", json!({ "type": "Login", "email": email, "password": password, "facebook": false })).await?;
    let save = |k, v: &str| ctx.store.set_meta(k, Some(v)).map_err(|e| e.to_string());
    save("stremio_auth", &r.auth_key)?;
    save("stremio_email", email)
}

/// The connected Stremio account's email, if any.
pub fn connected(ctx: &Ctx) -> Option<String> {
    ctx.store.meta("stremio_auth").ok().flatten()?;
    ctx.store.meta("stremio_email").ok().flatten()
}

pub async fn disconnect(ctx: &Ctx) {
    if let Ok(Some(auth)) = ctx.store.meta("stremio_auth") {
        // Best effort: the local key is forgotten either way.
        let _ = call::<Value>(&ctx.http, "logout", json!({ "type": "Logout", "authKey": auth })).await;
    }
    for k in ["stremio_auth", "stremio_email"] {
        let _ = ctx.store.set_meta(k, None);
    }
}

fn auth(ctx: &Ctx) -> Result<String, String> {
    ctx.store.meta("stremio_auth").ok().flatten().ok_or_else(|| "Connect your Stremio account first.".to_owned())
}

async fn stremio_addons(ctx: &Ctx, auth: &str) -> Result<Vec<Descriptor>, String> {
    #[derive(Deserialize)]
    struct Collection {
        addons: Vec<Descriptor>,
    }
    let c: Collection =
        call(&ctx.http, "addonCollectionGet", json!({ "type": "AddonCollectionGet", "authKey": auth, "update": true })).await?;
    Ok(c.addons)
}

/// Brings the Stremio account's addons into Relix (merge; nothing is removed).
pub async fn import_from_stremio(ctx: &Ctx) -> Result<Summary, String> {
    let theirs = stremio_addons(ctx, &auth(ctx)?).await?;
    let (merged, summary) = import(&ctx.addons.peek(), &theirs);
    if summary != Summary::default() {
        ctx.save_addons(merged);
    }
    Ok(summary)
}

/// Sends the user's own addons (not the server admin's) to the Stremio account (merge).
pub async fn export_to_stremio(ctx: &Ctx) -> Result<Summary, String> {
    let auth = auth(ctx)?;
    let theirs = stremio_addons(ctx, &auth).await?;
    // Stremio keeps whole manifests; fetch the originals rather than Relix's trimmed copies.
    let mine = ctx.addons.peek().clone();
    let mut set = JoinSet::new();
    for (i, a) in mine.iter().enumerate() {
        let (http, url) = (ctx.http.clone(), a.manifest_url.to_string());
        set.spawn(async move { (i, get::<Value>(http, url).await) });
    }
    let mut raw: Vec<Option<Value>> = vec![None; mine.len()];
    while let Some(Ok((i, res))) = set.join_next().await {
        match res {
            Ok(v) => raw[i] = Some(v),
            Err(e) => crate::log!("export: manifest of {} unavailable: {e}", mine[i].manifest.name),
        }
    }
    let skipped = raw.iter().filter(|r| r.is_none()).count();
    let pairs = mine.into_iter().zip(raw).filter_map(|(a, r)| Some((a, r?))).collect();
    let (list, summary) = export(theirs, pairs);
    if summary != Summary::default() {
        call::<Value>(&ctx.http, "addonCollectionSet", json!({ "type": "AddonCollectionSet", "authKey": auth, "addons": list })).await?;
    }
    if skipped > 0 {
        return Err(format!("{summary}; {skipped} skipped because their addon didn't answer"));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use relix_proto::addon::manifest_url;

    fn manifest(id: &str) -> serde_json::Value {
        serde_json::json!({ "id": id, "version": "1.0.0", "name": id, "resources": ["stream"], "types": ["movie"] })
    }

    fn relix(id: &str, url: &str) -> Addon {
        Addon { manifest_url: manifest_url(url).unwrap(), manifest: serde_json::from_value(manifest(id)).unwrap() }
    }

    fn stremio(id: &str, url: &str) -> Descriptor {
        Descriptor { transport_url: url.into(), transport_name: String::new(), manifest: manifest(id), flags: Flags::default() }
    }

    fn urls(list: &[Addon]) -> Vec<&str> {
        list.iter().map(|a| a.manifest_url.as_str()).collect()
    }

    #[test]
    fn import_updates_what_relix_has_and_appends_the_rest_in_stremios_order() {
        let mine = [relix("cinemeta", "https://cinemeta.example/manifest.json"), relix("torrentio", "https://torrentio.example/old/manifest.json")];
        let theirs = [
            stremio("kitsu", "https://kitsu.example/manifest.json"),
            stremio("torrentio", "https://torrentio.example/realdebrid=KEY/manifest.json"),
            stremio("opensubs", "https://opensubs.example/manifest.json"),
        ];
        let (merged, summary) = import(&mine, &theirs);
        assert_eq!(
            urls(&merged),
            [
                "https://cinemeta.example/manifest.json",
                "https://torrentio.example/realdebrid=KEY/manifest.json",
                "https://kitsu.example/manifest.json",
                "https://opensubs.example/manifest.json",
            ]
        );
        assert_eq!(summary, Summary { added: 2, updated: 1 });
    }

    #[test]
    fn import_skips_stremios_local_files_addon_and_broken_entries() {
        let theirs = [
            stremio("org.stremio.local", "http://127.0.0.1:11470/local-addon/manifest.json"),
            stremio("broken", "not a link"),
            Descriptor { manifest: serde_json::json!({ "id": "no-name" }), ..stremio("bad-manifest", "https://bad.example/manifest.json") },
            stremio("kitsu", "https://kitsu.example/manifest.json"),
        ];
        let (merged, summary) = import(&[], &theirs);
        assert_eq!(urls(&merged), ["https://kitsu.example/manifest.json"]);
        assert_eq!(summary, Summary { added: 1, updated: 0 });
    }

    #[test]
    fn importing_the_same_list_twice_changes_nothing() {
        let theirs = [stremio("kitsu", "https://kitsu.example/manifest.json")];
        let (once, _) = import(&[], &theirs);
        let (twice, summary) = import(&once, &theirs);
        assert_eq!(urls(&twice), urls(&once));
        assert_eq!(summary, Summary { added: 0, updated: 0 });
    }
}
