//! Google Drive for Watch Together (ADR 0010). A party's video is a file in the host's Drive,
//! shared only with the Google accounts of the people who join (never "anyone with the link"):
//! each member signs in to Google, the server passes their address to the host, and the host's
//! Relix adds them as a viewer. Everyone streams it through a relay on 127.0.0.1 that adds their
//! own fresh token. Afterwards the host can move the file to Drive's trash.
//!
//! Google sign-in is the desktop flow: the browser comes back to a one-off port on 127.0.0.1
//! (PKCE), and the refresh token stays in this device's store. The OAuth client comes in when
//! building: `RELIX_GOOGLE_CLIENT_ID` and `RELIX_GOOGLE_CLIENT_SECRET` (repository secrets).
//! Without them, Drive parties are off. While the Google Cloud project is in testing mode, only
//! its listed test users can sign in.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::Ctx;

const CLIENT_ID: Option<&str> = option_env!("RELIX_GOOGLE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("RELIX_GOOGLE_CLIENT_SECRET");
/// Sharing and trashing files Relix didn't create need the full Drive scope (drive.file only
/// covers its own); members use the same one so there's a single sign-in.
const SCOPE: &str = "https://www.googleapis.com/auth/drive";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://www.googleapis.com/drive/v3";
/// Where the refresh token lives in the store's meta table.
const META: &str = "google_refresh_token";

/// This device's Google session: plain data, so the relay can refresh it off the UI thread.
struct Session {
    refresh: String,
    access: String,
    until: Instant,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

pub fn enabled() -> bool {
    // Unset repository secrets arrive as empty strings, not missing.
    [CLIENT_ID, CLIENT_SECRET].iter().all(|v| v.is_some_and(|v| !v.is_empty())) && !cfg!(target_os = "android")
}

/// The Drive file id in a link: a share link (`/file/d/<id>/view`, `open?id=`) or a download one.
pub fn file_id(url: &str) -> Option<String> {
    let id = if let Some(rest) = url.split("/file/d/").nth(1) {
        rest.split(['/', '?', '#']).next()?
    } else {
        let is_drive = url.contains("drive.google.com") || url.contains("drive.usercontent.google.com") || url.contains("docs.google.com");
        let query = url.split_once('?')?.1;
        query.split('&').find_map(|kv| kv.strip_prefix("id=")).filter(|_| is_drive)?
    };
    let valid = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| id.to_owned())
}

/// The link a party keeps for a Drive file.
pub fn party_url(file_id: &str) -> String {
    format!("https://drive.google.com/file/d/{file_id}/view")
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

fn client() -> Result<(&'static str, &'static str), String> {
    match (CLIENT_ID, CLIENT_SECRET) {
        (Some(id), Some(secret)) if enabled() => Ok((id, secret)),
        _ => Err("This build of Relix can't sign in to Google, so it can't play Drive parties.".into()),
    }
}

fn keep(refresh: String, tokens: &Tokens) {
    // A minute early, so a token never runs out mid-request.
    let until = Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(60));
    *SESSION.lock().unwrap() = Some(Session { refresh, access: tokens.access_token.clone(), until });
}

/// A valid access token from the session in memory, refreshed when due; never signs in.
async fn cached_access(http: &reqwest::Client) -> Result<String, String> {
    let refresh = match SESSION.lock().unwrap().as_ref() {
        Some(s) if s.until > Instant::now() => return Ok(s.access.clone()),
        Some(s) => s.refresh.clone(),
        None => return Err("not signed in to Google".into()),
    };
    let (id, secret) = client()?;
    let tokens = token_request(http, &[("client_id", id), ("client_secret", secret), ("refresh_token", &refresh), ("grant_type", "refresh_token")]).await?;
    keep(refresh, &tokens);
    Ok(tokens.access_token)
}

/// An access token, signing in to Google in the browser if this device never has (or the
/// sign-in expired: testing-mode ones last 7 days).
pub async fn access_token(ctx: &Ctx) -> Result<String, String> {
    if SESSION.lock().unwrap().is_none()
        && let Some(refresh) = ctx.store.meta(META).ok().flatten()
    {
        *SESSION.lock().unwrap() = Some(Session { refresh, access: String::new(), until: Instant::now() });
    }
    match cached_access(&ctx.http).await {
        Ok(token) => Ok(token),
        Err(_) => sign_in(ctx).await,
    }
}

async fn api(ctx: &Ctx, req: reqwest::RequestBuilder) -> Result<reqwest::Response, String> {
    let token = access_token(ctx).await?;
    req.bearer_auth(token).timeout(Duration::from_secs(20)).send().await.map_err(|e| format!("Couldn't reach Google Drive: {}", e.without_url()))
}

async fn json<T: serde::de::DeserializeOwned>(res: reqwest::Response) -> Result<T, String> {
    res.error_for_status().map_err(|e| format!("Google Drive: {}", e.without_url()))?.json().await.map_err(|e| e.without_url().to_string())
}

/// The Google address this device signed in with.
pub async fn email(ctx: &Ctx) -> Result<String, String> {
    #[derive(Deserialize)]
    struct About {
        user: User,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct User {
        email_address: String,
    }
    let about: About = json(api(ctx, ctx.http.get(format!("{API}/about?fields=user(emailAddress)"))).await?).await?;
    Ok(about.user.email_address)
}

/// Host: the file is a video in this account's Drive that it may share.
pub async fn check_hostable(ctx: &Ctx, file_id: &str) -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct File {
        mime_type: String,
        capabilities: Caps,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Caps {
        can_share: bool,
    }
    let url = format!("{API}/files/{file_id}?fields=mimeType,capabilities(canShare)&supportsAllDrives=true");
    let res = api(ctx, ctx.http.get(url)).await?;
    if res.status().as_u16() == 404 {
        return Err("That file isn't in the Google Drive you signed in with.".into());
    }
    let file: File = json(res).await?;
    if !file.mime_type.starts_with("video/") {
        return Err(format!("That Drive file isn't a video ({}).", file.mime_type));
    }
    if !file.capabilities.can_share {
        return Err("Your Google account can't share that file: put it in your own Drive first.".into());
    }
    Ok(())
}

/// Host: lets one Google account view the file, without Google emailing them.
pub async fn share(ctx: &Ctx, file_id: &str, email: &str) -> Result<(), String> {
    let url = format!("{API}/files/{file_id}/permissions?sendNotificationEmail=false&supportsAllDrives=true");
    let body = serde_json::json!({ "type": "user", "role": "reader", "emailAddress": email });
    let res = api(ctx, ctx.http.post(url).json(&body)).await?;
    match res.status() {
        s if s.is_success() => Ok(()),
        s => Err(format!("Google Drive refused to share it ({s})")),
    }
}

/// Member: whether this account can read the file yet (the host shares it when they join).
pub async fn readable(ctx: &Ctx, file_id: &str) -> bool {
    let url = format!("{API}/files/{file_id}?fields=id&supportsAllDrives=true");
    api(ctx, ctx.http.get(url)).await.is_ok_and(|r| r.status().is_success())
}

/// Moves the file to the host's Drive trash.
pub async fn trash(ctx: &Ctx, file_id: &str) -> Result<(), String> {
    let url = format!("{API}/files/{file_id}?supportsAllDrives=true");
    let res = api(ctx, ctx.http.patch(url).json(&serde_json::json!({ "trashed": true }))).await?;
    match res.status().as_u16() {
        200 => Ok(()),
        404 => Err("Drive has no such file for this Google account (already deleted, or someone else's).".into()),
        403 => Err("This Google account can't delete that file: only its owner can.".into()),
        code => Err(format!("Google Drive refused ({code}).")),
    }
}

/// Serves the Drive file on 127.0.0.1 for mpv, adding a fresh token to each request (tokens last
/// an hour; a film is longer). Returns the URL to play and a handle that stops it.
pub async fn relay(ctx: &Ctx, file_id: &str) -> Result<(String, tokio::task::AbortHandle), String> {
    access_token(ctx).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (http, media) = (ctx.http.clone(), format!("{API}/files/{file_id}?alt=media&supportsAllDrives=true"));
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let (http, media) = (http.clone(), media.clone());
            tokio::spawn(async move {
                if let Err(e) = forward(socket, &http, &media).await {
                    crate::log!("drive relay: {e}");
                }
            });
        }
    });
    Ok((format!("http://127.0.0.1:{port}/video"), task.abort_handle()))
}

/// One request from mpv: the same byte range from Drive, streamed back.
async fn forward(mut socket: tokio::net::TcpStream, http: &reqwest::Client, media: &str) -> Result<(), String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = socket.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 || head.len() > 16384 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
    }
    let range = range_header(&String::from_utf8_lossy(&head));
    let mut req = http.get(media).bearer_auth(cached_access(http).await?);
    if let Some(range) = range {
        req = req.header("range", range);
    }
    let mut res = req.send().await.map_err(|e| e.without_url().to_string())?;
    let mut reply = format!("HTTP/1.1 {}\r\nConnection: close\r\n", res.status());
    for name in ["content-type", "content-length", "content-range", "accept-ranges"] {
        if let Some(v) = res.headers().get(name).and_then(|v| v.to_str().ok()) {
            reply.push_str(&format!("{name}: {v}\r\n"));
        }
    }
    reply.push_str("\r\n");
    socket.write_all(reply.as_bytes()).await.map_err(|e| e.to_string())?;
    while let Some(chunk) = res.chunk().await.map_err(|e| e.without_url().to_string())? {
        // mpv closing the connection (a seek) ends this request.
        if socket.write_all(&chunk).await.is_err() {
            break;
        }
    }
    Ok(())
}

fn range_header(head: &str) -> Option<String> {
    head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("range")).map(|(_, v)| v.trim().to_owned()))
}

async fn token_request(http: &reqwest::Client, form: &[(&str, &str)]) -> Result<Tokens, String> {
    // reqwest here is built without its `form` feature: encode the body the same way by hand.
    let mut encoded = reqwest::Url::parse("http://x/").expect("valid URL");
    encoded.query_pairs_mut().extend_pairs(form);
    let body = encoded.query().unwrap_or_default().to_owned();
    let res = http
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !res.status().is_success() {
        return Err(format!("Google sign-in failed ({})", res.status()));
    }
    res.json().await.map_err(|e| e.without_url().to_string())
}

fn random() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Google's desktop sign-in: opens the browser, waits (up to 3 minutes) for it to come back to
/// 127.0.0.1, and keeps the refresh token. Returns an access token.
async fn sign_in(ctx: &Ctx) -> Result<String, String> {
    let (id, secret) = client()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port());
    let (verifier, state) = (random()?, random()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state);
    crate::addons::open_in_browser(url.as_str());

    let code = tokio::time::timeout(Duration::from_secs(180), wait_for_code(&listener, &state))
        .await
        .map_err(|_| "Google sign-in timed out: try again and finish it in the browser.".to_owned())??;
    let form = [
        ("client_id", id),
        ("client_secret", secret),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let tokens = token_request(&ctx.http, &form).await?;
    let refresh = tokens.refresh_token.clone().ok_or("Google didn't give Relix a lasting sign-in; try again")?;
    if let Err(e) = ctx.store.set_meta(META, Some(&refresh)) {
        crate::log!("saving Google sign-in: {e}");
    }
    keep(refresh, &tokens);
    Ok(tokens.access_token)
}

/// Answers the browser's return visit and takes the code from it; other visits (a favicon) wait on.
async fn wait_for_code(listener: &tokio::net::TcpListener, state: &str) -> Result<String, String> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let Some(result) = parse_callback(&request, state) else {
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        };
        let message = match &result {
            Ok(_) => "Relix is signed in to Google Drive. You can close this tab.",
            Err(_) => "Google sign-in didn't finish. You can close this tab and try again in Relix.",
        };
        let body = format!("<!doctype html><meta charset=utf-8><title>Relix</title><p style=\"font:18px sans-serif;margin:3em\">{message}</p>");
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let _ = socket.write_all(response.as_bytes()).await;
        return result;
    }
}

/// The code from `GET /?state=..&code=..`, an error for a refusal, or `None` for other requests.
fn parse_callback(request: &str, state: &str) -> Option<Result<String, String>> {
    let target = request.lines().next()?.strip_prefix("GET ")?.split(' ').next()?;
    let query = target.strip_prefix("/?")?;
    let params: Vec<(String, String)> = reqwest::Url::parse(&format!("http://x/?{query}")).ok()?.query_pairs().into_owned().collect();
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    if get("state").as_deref() != Some(state) {
        return Some(Err("Google sign-in didn't match (state)".into()));
    }
    Some(match (get("code"), get("error")) {
        (Some(code), _) => Ok(code),
        (None, Some(e)) if e == "access_denied" => Err("Google sign-in was cancelled.".into()),
        (None, e) => Err(format!("Google sign-in failed: {}", e.unwrap_or_default())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_drive_file_id() {
        let id = Some("1AbC-d_9".to_owned());
        assert_eq!(file_id("https://drive.usercontent.google.com/download?id=1AbC-d_9&export=download&confirm=t"), id);
        assert_eq!(file_id("https://drive.google.com/file/d/1AbC-d_9/view?usp=sharing"), id);
        assert_eq!(file_id("https://drive.google.com/open?id=1AbC-d_9"), id);
        assert_eq!(file_id(&party_url("1AbC-d_9")), id);
        assert_eq!(file_id("https://example.com/video?id=1AbC-d_9"), None, "not Drive");
        assert_eq!(file_id("https://drive.google.com/file/d/a%2Fb/view"), None);
    }

    #[test]
    fn reads_the_browser_coming_back() {
        let ok = "GET /?state=s1&code=4%2F0Ab&scope=x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(parse_callback(ok, "s1"), Some(Ok("4/0Ab".into())));
        assert!(matches!(parse_callback(ok, "other"), Some(Err(_))));
        assert!(matches!(parse_callback("GET /?state=s1&error=access_denied HTTP/1.1\r\n", "s1"), Some(Err(e)) if e.contains("cancelled")));
        assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1\r\n", "s1"), None);
    }

    #[test]
    fn the_relay_passes_mpv_range_on() {
        assert_eq!(range_header("GET /video HTTP/1.1\r\nHost: x\r\nRange: bytes=100-\r\n\r\n").as_deref(), Some("bytes=100-"));
        assert_eq!(range_header("GET /video HTTP/1.1\r\nrange:bytes=0-9\r\n\r\n").as_deref(), Some("bytes=0-9"));
        assert_eq!(range_header("GET /video HTTP/1.1\r\n\r\n"), None);
    }
}
