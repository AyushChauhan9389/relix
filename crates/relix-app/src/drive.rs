//! Deleting a party's video from the host's Google Drive afterwards (ADR 0010), by hand: the file
//! goes to Drive's trash. Google sign-in is the desktop flow: the browser comes back to a one-off
//! port on 127.0.0.1 (PKCE), and the refresh token stays in this device's store.
//!
//! The OAuth client comes in when building: `RELIX_GOOGLE_CLIENT_ID` and
//! `RELIX_GOOGLE_CLIENT_SECRET` (a desktop client's secret isn't secret to Google, but it stays
//! out of the public repo). Without them the feature is hidden.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::Ctx;

const CLIENT_ID: Option<&str> = option_env!("RELIX_GOOGLE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("RELIX_GOOGLE_CLIENT_SECRET");
/// Trashing a file Relix didn't create needs the full Drive scope (drive.file only covers its own).
const SCOPE: &str = "https://www.googleapis.com/auth/drive";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Where the refresh token lives in the store's meta table.
const META: &str = "google_refresh_token";

pub fn enabled() -> bool {
    // Unset repository secrets arrive as empty strings, not missing.
    [CLIENT_ID, CLIENT_SECRET]
        .iter()
        .all(|v| v.is_some_and(|v| !v.is_empty()))
        && !cfg!(target_os = "android")
}

/// The Drive file id in a party URL (the download link `direct_video_url` made, or a share link).
pub fn file_id(url: &str) -> Option<String> {
    let id = if let Some(rest) = url.split("/file/d/").nth(1) {
        rest.split(['/', '?', '#']).next()?
    } else {
        let is_drive = url.contains("drive.google.com")
            || url.contains("drive.usercontent.google.com")
            || url.contains("docs.google.com");
        let query = url.split_once('?')?.1;
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix("id="))
            .filter(|_| is_drive)?
    };
    let valid = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| id.to_owned())
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
}

/// Moves the file to the host's Drive trash, signing in to Google first if needed.
pub async fn trash(ctx: &Ctx, file_id: &str) -> Result<(), String> {
    let saved = ctx.store.meta(META).ok().flatten();
    let access = match saved {
        Some(refresh) => match refresh_access(ctx, &refresh).await {
            Ok(token) => token,
            // Revoked or expired (testing-mode tokens last 7 days): sign in again.
            Err(_) => sign_in(ctx).await?,
        },
        None => sign_in(ctx).await?,
    };
    let res = ctx
        .http
        .patch(format!(
            "https://www.googleapis.com/drive/v3/files/{file_id}?supportsAllDrives=true"
        ))
        .bearer_auth(access)
        .json(&serde_json::json!({ "trashed": true }))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach Google Drive: {}", e.without_url()))?;
    match res.status().as_u16() {
        200 => Ok(()),
        404 => Err(
            "Drive has no such file for this Google account (already deleted, or someone else's)."
                .into(),
        ),
        403 => Err("This Google account can't delete that file: only its owner can.".into()),
        code => Err(format!("Google Drive refused ({code}).")),
    }
}

async fn refresh_access(ctx: &Ctx, refresh: &str) -> Result<String, String> {
    let (id, secret) = client()?;
    let form = [
        ("client_id", id),
        ("client_secret", secret),
        ("refresh_token", refresh),
        ("grant_type", "refresh_token"),
    ];
    let tokens: Tokens = token_request(ctx, &form).await?;
    Ok(tokens.access_token)
}

fn client() -> Result<(&'static str, &'static str), String> {
    match (CLIENT_ID, CLIENT_SECRET) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => Err("This build of Relix has no Google sign-in.".into()),
    }
}

async fn token_request(ctx: &Ctx, form: &[(&str, &str)]) -> Result<Tokens, String> {
    // reqwest here is built without its `form` feature: encode the body the same way by hand.
    let mut encoded = reqwest::Url::parse("http://x/").expect("valid URL");
    encoded.query_pairs_mut().extend_pairs(form);
    let body = encoded.query().unwrap_or_default().to_owned();
    let res = ctx
        .http
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let redirect = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().map_err(|e| e.to_string())?.port()
    );
    let (verifier, state) = (random()?, random()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url =
        reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("valid URL");
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
        .map_err(|_| {
            "Google sign-in timed out: try again and finish it in the browser.".to_owned()
        })??;
    let form = [
        ("client_id", id),
        ("client_secret", secret),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let tokens = token_request(ctx, &form).await?;
    if let Some(refresh) = &tokens.refresh_token
        && let Err(e) = ctx.store.set_meta(META, Some(refresh))
    {
        crate::log!("saving Google sign-in: {e}");
    }
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
            let _ = socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        };
        let message = match &result {
            Ok(_) => "Relix can delete the file now. You can close this tab.",
            Err(_) => {
                "Google sign-in didn't finish. You can close this tab and try again in Relix."
            }
        };
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>Relix</title><p style=\"font:18px sans-serif;margin:3em\">{message}</p>"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        return result;
    }
}

/// The code from `GET /?state=..&code=..`, an error for a refusal, or `None` for other requests.
fn parse_callback(request: &str, state: &str) -> Option<Result<String, String>> {
    let target = request
        .lines()
        .next()?
        .strip_prefix("GET ")?
        .split(' ')
        .next()?;
    let query = target.strip_prefix("/?")?;
    let params: Vec<(String, String)> = reqwest::Url::parse(&format!("http://x/?{query}"))
        .ok()?
        .query_pairs()
        .into_owned()
        .collect();
    let get = |k: &str| {
        params
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
    };
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
        assert_eq!(
            file_id(
                "https://drive.usercontent.google.com/download?id=1AbC-d_9&export=download&confirm=t"
            ),
            id
        );
        assert_eq!(
            file_id("https://drive.google.com/file/d/1AbC-d_9/view?usp=sharing"),
            id
        );
        assert_eq!(file_id("https://drive.google.com/open?id=1AbC-d_9"), id);
        assert_eq!(
            file_id("https://example.com/video?id=1AbC-d_9"),
            None,
            "not Drive"
        );
        assert_eq!(file_id("https://drive.google.com/file/d/a%2Fb/view"), None);
    }

    #[test]
    fn reads_the_browser_coming_back() {
        let ok = "GET /?state=s1&code=4%2F0Ab&scope=x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(parse_callback(ok, "s1"), Some(Ok("4/0Ab".into())));
        assert!(matches!(parse_callback(ok, "other"), Some(Err(_))));
        assert!(
            matches!(parse_callback("GET /?state=s1&error=access_denied HTTP/1.1\r\n", "s1"), Some(Err(e)) if e.contains("cancelled"))
        );
        assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1\r\n", "s1"), None);
    }
}
