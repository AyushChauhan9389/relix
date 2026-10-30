use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relix_server::{Config, Mailer, app};
use serde_json::{Value, json};
use tower::ServiceExt;

type Outbox = Arc<Mutex<Vec<(String, String)>>>;

const ADMIN: &str = "admin-secret";

/// A fresh folder per test for update files.
fn update_dir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("relix-updates-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn server(verify_email: bool) -> (Router, Outbox) {
    let outbox = Outbox::default();
    let config = Config { verify_email, mailer: Mailer::Outbox(outbox.clone()), admin_token: Some(ADMIN.into()), update_dir: update_dir() };
    (app(rusqlite::Connection::open_in_memory().unwrap(), config).unwrap(), outbox)
}

async fn call(app: &Router, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    send(app, if body.is_some() { "POST" } else { "GET" }, path, token, body).await
}

async fn send(app: &Router, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().uri(path).method(method);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())).unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn signup_body(email: &str) -> Value {
    json!({ "email": email, "password": "right password" })
}

/// Signs up on a server without email verification and returns the session token.
async fn account(app: &Router, email: &str) -> String {
    let (status, body) = call(app, "/signup", None, Some(signup_body(email))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["session"]["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn signup_verify_and_login() {
    let (app, outbox) = server(true);
    let (status, body) = call(&app, "/signup", None, Some(signup_body("a@x.com"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["session"].is_null(), "no session before the email is verified");

    let login = json!({ "email": "a@x.com", "password": "right password" });
    assert_eq!(call(&app, "/login", None, Some(login.clone())).await.0, StatusCode::FORBIDDEN);

    let code = {
        let mails = outbox.lock().unwrap();
        assert_eq!(mails[0].0, "a@x.com");
        mails[0].1.chars().filter(char::is_ascii_digit).collect::<String>()
    };
    assert_eq!(call(&app, "/verify", None, Some(json!({ "email": "a@x.com", "code": "000000x" }))).await.0, StatusCode::UNAUTHORIZED);
    let (status, session) = call(&app, "/verify", None, Some(json!({ "email": "a@x.com", "code": code }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(session["token"].is_string());

    let (status, session) = call(&app, "/login", None, Some(login)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(session["token"].is_string());
    let bad = json!({ "email": "a@x.com", "password": "wrong password" });
    assert_eq!(call(&app, "/login", None, Some(bad)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/signup", None, Some(signup_body("a@x.com"))).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn unknown_email_and_wrong_password_look_the_same() {
    let (app, _) = server(false);
    account(&app, "a@x.com").await;
    let wrong = call(&app, "/login", None, Some(json!({ "email": "a@x.com", "password": "nope" }))).await;
    let unknown = call(&app, "/login", None, Some(json!({ "email": "nobody@x.com", "password": "nope" }))).await;
    assert_eq!(wrong, unknown);
    assert_eq!(wrong.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn short_passwords_are_refused() {
    let (app, _) = server(false);
    let (status, _) = call(&app, "/signup", None, Some(json!({ "email": "a@x.com", "password": "short" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sync_needs_a_session() {
    let (app, _) = server(false);
    assert_eq!(call(&app, "/sync/pull?since=0", None, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/sync/pull?since=0", Some("made-up"), None).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn push_assigns_increasing_seqs_and_the_last_write_wins() {
    let (app, _) = server(false);
    let t = account(&app, "a@x.com").await;
    let push = |records: Value| json!({ "records": records });

    let (status, r) = call(&app, "/sync/push", Some(&t), Some(push(json!([
        { "kind": "progress", "key": "tt1", "value": "1" },
        { "kind": "library", "key": "tt2", "value": "true" },
    ])))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(r["seqs"], json!([1, 2]));
    let (_, r) = call(&app, "/sync/push", Some(&t), Some(push(json!([{ "kind": "progress", "key": "tt1", "value": "2" }])))).await;
    assert_eq!(r["seqs"], json!([3]));

    let (_, all) = call(&app, "/sync/pull?since=0", Some(&t), None).await;
    assert_eq!(all["records"], json!([
        { "kind": "library", "key": "tt2", "value": "true", "seq": 2 },
        { "kind": "progress", "key": "tt1", "value": "2", "seq": 3 },
    ]));
    let (_, newer) = call(&app, "/sync/pull?since=2", Some(&t), None).await;
    assert_eq!(newer["records"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn accounts_never_see_each_others_records() {
    let (app, _) = server(false);
    let a = account(&app, "a@x.com").await;
    let b = account(&app, "b@x.com").await;
    call(&app, "/sync/push", Some(&a), Some(json!({ "records": [{ "kind": "library", "key": "tt1", "value": "x" }] }))).await;
    let (_, r) = call(&app, "/sync/pull?since=0", Some(&b), None).await;
    assert_eq!(r["records"], json!([]));
}

#[tokio::test]
async fn a_push_notifies_the_accounts_other_devices_over_sse() {
    let (app, _) = server(false);
    let t = account(&app, "a@x.com").await;
    let other = account(&app, "b@x.com").await;

    let req = Request::builder().uri("/sync/events").header("authorization", format!("Bearer {t}")).body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();

    // Another account's push must not show up; this account's must.
    call(&app, "/sync/push", Some(&other), Some(json!({ "records": [{ "kind": "k", "key": "1", "value": "v" }] }))).await;
    call(&app, "/sync/push", Some(&t), Some(json!({ "records": [{ "kind": "k", "key": "1", "value": "v" }] }))).await;

    let frame = tokio::time::timeout(Duration::from_secs(2), body.frame()).await.expect("an event within 2s").unwrap().unwrap();
    let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
    assert!(text.contains("data: 1"), "event carries the new seq: {text:?}");
}

#[tokio::test]
async fn repeated_wrong_logins_are_throttled() {
    let (app, _) = server(false);
    account(&app, "a@x.com").await;
    let bad = json!({ "email": "a@x.com", "password": "nope" });
    for _ in 0..10 {
        assert_eq!(call(&app, "/login", None, Some(bad.clone())).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(call(&app, "/login", None, Some(bad)).await.0, StatusCode::TOO_MANY_REQUESTS);
    let good = json!({ "email": "a@x.com", "password": "right password" });
    assert_eq!(call(&app, "/login", None, Some(good)).await.0, StatusCode::TOO_MANY_REQUESTS, "even the right password waits");
}

fn torrentio() -> Value {
    let path = format!("{}/../relix-proto/tests/fixtures/torrentio.json", env!("CARGO_MANIFEST_DIR"));
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    json!({ "manifest_url": "https://torrentio.strem.fun/manifest.json", "manifest": manifest })
}

#[tokio::test]
async fn admin_endpoints_need_the_admin_token() {
    let (app, _) = server(false);
    let user = account(&app, "a@x.com").await;
    assert_eq!(call(&app, "/admin/addons", None, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/admin/addons", Some(&user), None).await.0, StatusCode::UNAUTHORIZED, "a user session isn't admin");
    assert_eq!(call(&app, "/admin/addons", Some("admin-secreT"), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/admin/addons", Some(ADMIN), Some(torrentio())).await.0, StatusCode::OK);
}

#[tokio::test]
async fn admin_addons_reach_signed_in_users_until_removed() {
    let (app, _) = server(false);
    let user = account(&app, "a@x.com").await;
    let (_, added) = call(&app, "/admin/addons", Some(ADMIN), Some(torrentio())).await;
    let id = added["id"].as_u64().unwrap();

    let (status, list) = call(&app, "/addons/admin", Some(&user), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["manifest"]["id"], "com.stremio.torrentio.addon");
    assert_eq!(call(&app, "/addons/admin", None, None).await.0, StatusCode::UNAUTHORIZED, "signed-out users get nothing");
    let (_, admin_view) = call(&app, "/admin/addons", Some(ADMIN), None).await;
    assert_eq!(admin_view[0]["id"].as_u64(), Some(id));

    assert_eq!(send(&app, "DELETE", &format!("/admin/addons/{id}"), Some(ADMIN), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "/addons/admin", Some(&user), None).await.1, json!([]));
    assert_eq!(send(&app, "DELETE", &format!("/admin/addons/{id}"), Some(ADMIN), None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn adding_an_admin_addon_notifies_every_account() {
    let (app, _) = server(false);
    let mut streams = vec![];
    for email in ["a@x.com", "b@x.com"] {
        let t = account(&app, email).await;
        let req = Request::builder().uri("/sync/events").header("authorization", format!("Bearer {t}")).body(Body::empty()).unwrap();
        streams.push(app.clone().oneshot(req).await.unwrap().into_body());
    }
    call(&app, "/admin/addons", Some(ADMIN), Some(torrentio())).await;
    for mut body in streams {
        let frame = tokio::time::timeout(Duration::from_secs(2), body.frame()).await.expect("an event within 2s").unwrap().unwrap();
        assert!(String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap().contains("data: admin"));
    }
}

#[tokio::test]
async fn serves_the_latest_update_and_its_file() {
    let dir = update_dir();
    let outbox = Outbox::default();
    let config = Config { verify_email: false, mailer: Mailer::Outbox(outbox), admin_token: None, update_dir: dir.clone() };
    let app = app(rusqlite::Connection::open_in_memory().unwrap(), config).unwrap();
    assert_eq!(call(&app, "/update/latest", None, None).await.0, StatusCode::NOT_FOUND, "nothing published yet");

    let latest = json!({ "version": "0.2.0", "file": "Relix-0.2.0.zip", "signature": "sig" });
    std::fs::write(dir.join("latest.json"), latest.to_string()).unwrap();
    std::fs::write(dir.join("Relix-0.2.0.zip"), b"zip bytes").unwrap();
    assert_eq!(call(&app, "/update/latest", None, None).await, (StatusCode::OK, latest));

    let req = Request::builder().uri("/update/files/Relix-0.2.0.zip").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(&res.into_body().collect().await.unwrap().to_bytes()[..], b"zip bytes");

    assert_eq!(call(&app, "/update/files/missing.zip", None, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "/update/files/..%2Flatest.json", None, None).await.0, StatusCode::NOT_FOUND, "no paths out of the folder");
}

async fn put(app: &Router, path: &str, token: Option<&str>, body: &'static [u8]) -> StatusCode {
    let mut req = Request::builder().uri(path).method("PUT");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    app.clone().oneshot(req.body(Body::from(body)).unwrap()).await.unwrap().status()
}

#[tokio::test]
async fn admins_publish_updates_per_os() {
    let (app, _) = server(false);
    assert_eq!(put(&app, "/admin/update/latest-linux.json", None, b"{}").await, StatusCode::UNAUTHORIZED);
    assert_eq!(put(&app, "/admin/update/Relix-linux.tar.gz", Some(ADMIN), b"archive").await, StatusCode::NO_CONTENT);
    let latest = json!({ "version": "0.2.0", "file": "Relix-linux.tar.gz", "signature": "sig" });
    let bytes: &'static [u8] = Box::leak(latest.to_string().into_bytes().into_boxed_slice());
    assert_eq!(put(&app, "/admin/update/latest-linux.json", Some(ADMIN), bytes).await, StatusCode::NO_CONTENT);

    assert_eq!(call(&app, "/update/latest?os=linux", None, None).await, (StatusCode::OK, latest));
    assert_eq!(call(&app, "/update/latest", None, None).await.0, StatusCode::NOT_FOUND, "no Windows (default) build yet");
    assert_eq!(call(&app, "/update/latest?os=..%2Flatest", None, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(put(&app, "/admin/update/..%2Fescape", Some(ADMIN), b"x").await, StatusCode::BAD_REQUEST);

    let req = Request::builder().uri("/update/files/Relix-linux.tar.gz").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(&res.into_body().collect().await.unwrap().to_bytes()[..], b"archive");
}

#[tokio::test]
async fn logout_ends_this_session_and_logout_all_ends_every_one() {
    let (app, _) = server(false);
    let a = account(&app, "a@x.com").await;
    let login = json!({ "email": "a@x.com", "password": "right password" });
    let b = call(&app, "/login", None, Some(login.clone())).await.1["token"].as_str().unwrap().to_owned();
    let c = call(&app, "/login", None, Some(login)).await.1["token"].as_str().unwrap().to_owned();

    assert_eq!(call(&app, "/logout", Some(&a), Some(json!({}))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "/sync/pull?since=0", Some(&a), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/sync/pull?since=0", Some(&b), None).await.0, StatusCode::OK, "other devices stay signed in");

    assert_eq!(call(&app, "/logout/all", Some(&b), Some(json!({}))).await.0, StatusCode::NO_CONTENT);
    for t in [&b, &c] {
        assert_eq!(call(&app, "/sync/pull?since=0", Some(t), None).await.0, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn forgotten_passwords_are_reset_with_an_emailed_code() {
    let (app, outbox) = server(false);
    let old = account(&app, "a@x.com").await;

    assert_eq!(call(&app, "/password/forgot", None, Some(json!({ "email": "nobody@x.com" }))).await.0, StatusCode::NO_CONTENT);
    assert!(outbox.lock().unwrap().is_empty(), "unknown emails get nothing, and the reply doesn't say so");

    assert_eq!(call(&app, "/password/forgot", None, Some(json!({ "email": " A@x.com " }))).await.0, StatusCode::NO_CONTENT);
    let (to, text) = outbox.lock().unwrap().pop().unwrap();
    assert_eq!(to, "a@x.com");
    let code: String = text.chars().filter(char::is_ascii_digit).collect();

    let reset = |code: &str, password: &str| json!({ "email": "a@x.com", "code": code, "password": password });
    assert_eq!(call(&app, "/password/reset", None, Some(reset("000000x", "new password"))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/password/reset", None, Some(reset(&code, "short"))).await.0, StatusCode::BAD_REQUEST);
    let (status, body) = call(&app, "/password/reset", None, Some(reset(&code, "new password"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["token"].is_string(), "reset signs this device in");

    assert_eq!(call(&app, "/sync/pull?since=0", Some(&old), None).await.0, StatusCode::UNAUTHORIZED, "old sessions end");
    assert_eq!(call(&app, "/password/reset", None, Some(reset(&code, "again password"))).await.0, StatusCode::UNAUTHORIZED, "codes work once");
    let login = |password: &str| json!({ "email": "a@x.com", "password": password });
    assert_eq!(call(&app, "/login", None, Some(login("right password"))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/login", None, Some(login("new password"))).await.0, StatusCode::OK);
}

#[tokio::test]
async fn deleting_an_account_needs_its_password_and_removes_everything() {
    let (app, _) = server(false);
    let token = account(&app, "a@x.com").await;
    let push = json!({ "records": [{ "kind": "library", "key": "tt1", "value": "{}" }] });
    assert_eq!(call(&app, "/sync/push", Some(&token), Some(push)).await.0, StatusCode::OK);

    let delete = |password: &str| json!({ "password": password });
    assert_eq!(send(&app, "DELETE", "/account", Some(&token), Some(delete("wrong one"))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&app, "DELETE", "/account", Some(&token), Some(delete("right password"))).await.0, StatusCode::NO_CONTENT);

    assert_eq!(call(&app, "/sync/pull?since=0", Some(&token), None).await.0, StatusCode::UNAUTHORIZED);
    let fresh = account(&app, "a@x.com").await;
    let (_, body) = call(&app, "/sync/pull?since=0", Some(&fresh), None).await;
    assert_eq!(body["records"], json!([]), "a new account with the same email starts empty");
}

#[tokio::test]
async fn signups_are_limited_per_address() {
    let (app, _) = server(false);
    let signup = |n: u32| {
        Request::builder()
            .uri("/signup")
            .method("POST")
            .header("content-type", "application/json")
            .header("x-forwarded-for", "203.0.113.7")
            .body(Body::from(signup_body(&format!("u{n}@x.com")).to_string()))
            .unwrap()
    };
    for n in 0..5 {
        assert_eq!(app.clone().oneshot(signup(n)).await.unwrap().status(), StatusCode::OK);
    }
    assert_eq!(app.clone().oneshot(signup(5)).await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
}

#[test]
fn opens_a_database_from_before_session_expiry() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE sessions (token_hash TEXT PRIMARY KEY, account INTEGER NOT NULL); INSERT INTO sessions VALUES ('h', 1);").unwrap();
    let config = Config { verify_email: false, mailer: Mailer::Log, admin_token: None, update_dir: update_dir() };
    assert!(app(db, config).is_ok());
}
