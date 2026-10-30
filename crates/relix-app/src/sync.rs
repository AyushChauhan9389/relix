//! Account sign-in and background sync with relix-server (ADR 0004, 0005, 0006).

use std::time::Duration;

use freya::prelude::*;
use relix_proto::addon::Addon;
use relix_proto::sync::{Login, PullResponse, Push, PushRecord, PushResponse, Session, Signup, SignupResponse, Verify};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::store::Store;
use crate::{Ctx, load_rows};

/// The sync server, fixed when building: `RELIX_SERVER=https://api.example.com cargo build --release`.
const SERVER: &str = match option_env!("RELIX_SERVER") {
    Some(url) => url,
    None => "http://localhost:8787",
};
const IDLE_SYNC: Duration = Duration::from_secs(60);
const RECONNECT: Duration = Duration::from_secs(5);

/// The signed-in account, kept in the device's meta table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub token: String,
}

pub fn account(store: &Store) -> Option<Account> {
    store.meta("account").ok().flatten().and_then(|a| serde_json::from_str(&a).ok())
}

async fn call<B: Serialize, T: DeserializeOwned>(
    http: &reqwest::Client,
    url: String,
    token: Option<&str>,
    body: Option<&B>,
) -> Result<T, String> {
    let mut req = match body {
        Some(b) => http.post(url).json(b),
        None => http.get(url),
    };
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.map_err(|e| format!("Can't reach the server: {e}"))?;
    if !res.status().is_success() {
        let status = res.status();
        let msg = res.json::<serde_json::Value>().await.ok().and_then(|v| v["error"].as_str().map(str::to_owned));
        return Err(msg.unwrap_or_else(|| format!("Server error {status}")));
    }
    res.json().await.map_err(|e| format!("Unexpected server reply: {e}"))
}

fn server_url(path: &str) -> String {
    format!("{}{path}", SERVER.trim_end_matches('/'))
}

fn save_session(ctx: &Ctx, email: &str, session: &Session) -> Result<(), String> {
    let acc = Account { email: email.into(), token: session.token.clone() };
    ctx.store
        .set_meta("account", Some(&serde_json::to_string(&acc).unwrap()))
        .and_then(|()| ctx.store.set_meta("last_seq", Some("0")))
        // The first sync takes the account's copy over this device's defaults.
        .and_then(|()| ctx.store.set_meta("first_sync", Some("1")))
        .map_err(|e| e.to_string())?;
    ctx.sync_soon();
    Ok(())
}

/// Creates an account. Returns whether the server emailed a code to confirm first.
pub async fn sign_up(ctx: &Ctx, email: &str, password: &str) -> Result<bool, String> {
    let req = Signup { email: email.into(), password: password.into() };
    let res: SignupResponse = call(&ctx.http, server_url("/signup"), None, Some(&req)).await?;
    match res.session {
        Some(session) => save_session(ctx, email, &session).map(|()| false),
        None => Ok(true),
    }
}

pub async fn verify(ctx: &Ctx, email: &str, code: &str) -> Result<(), String> {
    let req = Verify { email: email.into(), code: code.into() };
    let session: Session = call(&ctx.http, server_url("/verify"), None, Some(&req)).await?;
    save_session(ctx, email, &session)
}

pub async fn sign_in(ctx: &Ctx, email: &str, password: &str) -> Result<(), String> {
    let req = Login { email: email.into(), password: password.into() };
    let session: Session = call(&ctx.http, server_url("/login"), None, Some(&req)).await?;
    save_session(ctx, email, &session)
}

/// Forgets the account on this device; local data stays.
pub fn sign_out(ctx: &Ctx) {
    for key in ["account", "last_seq", "first_sync", "admin_addons"] {
        if let Err(e) = ctx.store.set_meta(key, None) {
            eprintln!("sign out: {e}");
        }
    }
    let mut status = ctx.sync_status;
    status.set(String::new());
    // Admin addons come with the account, so they go with it.
    let mut admin = ctx.admin_addons;
    admin.set(vec![]);
    spawn(load_rows(ctx.clone()));
}

/// Pulls and applies the account's changes, then pushes this device's.
async fn sync_now(ctx: &Ctx, acc: &Account) -> Result<(), String> {
    let first = ctx.store.meta("first_sync").ok().flatten().is_some();
    let since = ctx.store.last_seq().unwrap_or(0);

    let pulled: PullResponse =
        call::<(), _>(&ctx.http, server_url(&format!("/sync/pull?since={since}")), Some(&acc.token), None).await?;
    let changed = ctx.store.apply_pulled(&ctx.profile, &pulled.records, first).map_err(|e| e.to_string())?;
    if changed.iter().any(|k| k == "settings") {
        let mut enabled = ctx.admin_enabled;
        enabled.set(ctx.store.get(&ctx.profile, "settings", "admin_addons_enabled").ok().flatten().unwrap_or(true));
        ctx.show_look(ctx.store.get(&ctx.profile, "settings", "look").ok().flatten().unwrap_or(crate::theme::Look::Netflix));
    }
    let admin: Vec<Addon> = call::<(), _>(&ctx.http, server_url("/addons/admin"), Some(&acc.token), None).await?;
    let admin_changed = *ctx.admin_addons.peek() != admin;
    if admin_changed {
        ctx.store.set_meta("admin_addons", Some(&serde_json::to_string(&admin).unwrap())).map_err(|e| e.to_string())?;
        let mut cached = ctx.admin_addons;
        cached.set(admin);
    }
    if first {
        ctx.store.set_meta("first_sync", None).map_err(|e| e.to_string())?;
    }
    if changed.iter().any(|k| k == "addons") {
        let mut addons = ctx.addons;
        addons.set(ctx.store.get(&ctx.profile, "addons", "list").ok().flatten().unwrap_or_default());
        spawn(load_rows(ctx.clone()));
    } else if admin_changed || changed.iter().any(|k| k == "settings") {
        spawn(load_rows(ctx.clone()));
    } else if changed.iter().any(|k| ["progress", "library", "watched"].contains(&k.as_str())) {
        ctx.refresh_local_rows();
    }

    let sent: Vec<_> = ctx.store.dirty().map_err(|e| e.to_string())?.into_iter().filter(|r| r.profile == ctx.profile).collect();
    if !sent.is_empty() {
        let records = sent.iter().map(|r| PushRecord { kind: r.kind.clone(), key: r.key.clone(), value: r.value.clone() }).collect();
        let res: PushResponse = call(&ctx.http, server_url("/sync/push"), Some(&acc.token), Some(&Push { records })).await?;
        ctx.store.mark_pushed(&sent, &res.seqs).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Waits for the next `data:` event on the SSE stream; drops the stream when it ends or fails.
async fn next_event(events: &mut Option<reqwest::Response>) {
    let Some(res) = events.as_mut() else { return std::future::pending().await };
    loop {
        match res.chunk().await {
            // Keep-alive comments carry no data line.
            Ok(Some(chunk)) if chunk.windows(5).any(|w| w == b"data:") => return,
            Ok(Some(_)) => {}
            _ => {
                *events = None;
                tokio::time::sleep(RECONNECT).await;
                return;
            }
        }
    }
}

/// Runs for the app's lifetime: syncs when signed in, then waits for a server event,
/// a local change, or the idle interval.
pub async fn run(ctx: Ctx) {
    let mut events: Option<reqwest::Response> = None;
    loop {
        let Some(acc) = account(&ctx.store) else {
            events = None;
            ctx.sync.notified().await;
            continue;
        };
        let mut status = ctx.sync_status;
        match sync_now(&ctx, &acc).await {
            Ok(()) => status.set("Synced".into()),
            Err(e) => status.set(e),
        }
        if events.is_none() {
            let res = ctx.http.get(server_url("/sync/events")).bearer_auth(&acc.token).send().await;
            events = res.ok().filter(|r| r.status().is_success());
        }
        tokio::select! {
            () = next_event(&mut events) => {}
            () = ctx.sync.notified() => {}
            () = tokio::time::sleep(IDLE_SYNC) => {}
        }
    }
}
