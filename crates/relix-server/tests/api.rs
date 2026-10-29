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

fn server(verify_email: bool) -> (Router, Outbox) {
    let outbox = Outbox::default();
    let config = Config { verify_email, mailer: Mailer::Outbox(outbox.clone()) };
    (app(rusqlite::Connection::open_in_memory().unwrap(), config).unwrap(), outbox)
}

async fn call(app: &Router, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().uri(path).method(if body.is_some() { "POST" } else { "GET" });
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
    json!({ "email": email, "salt": "c2FsdHNhbHRzYWx0c2FsdA==", "login_hash": "right-hash",
            "wrapped_key": "wk", "recovery_wrapped_key": "rk" })
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

    let login = json!({ "email": "a@x.com", "login_hash": "right-hash" });
    assert_eq!(call(&app, "/login", None, Some(login.clone())).await.0, StatusCode::FORBIDDEN);

    let code = {
        let mails = outbox.lock().unwrap();
        assert_eq!(mails[0].0, "a@x.com");
        mails[0].1.chars().filter(char::is_ascii_digit).collect::<String>()
    };
    assert_eq!(call(&app, "/verify", None, Some(json!({ "email": "a@x.com", "code": "000000x" }))).await.0, StatusCode::UNAUTHORIZED);
    let (status, session) = call(&app, "/verify", None, Some(json!({ "email": "a@x.com", "code": code }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["wrapped_key"], "wk");

    let (status, session) = call(&app, "/login", None, Some(login)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["recovery_wrapped_key"], "rk");
    let bad = json!({ "email": "a@x.com", "login_hash": "wrong-hash" });
    assert_eq!(call(&app, "/login", None, Some(bad)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "/signup", None, Some(signup_body("a@x.com"))).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn prelogin_hides_which_emails_exist() {
    let (app, _) = server(false);
    account(&app, "a@x.com").await;
    let salt = |email: &'static str| {
        let app = app.clone();
        async move { call(&app, "/prelogin", None, Some(json!({ "email": email }))).await.1["salt"].clone() }
    };
    assert_eq!(salt("a@x.com").await, "c2FsdHNhbHRzYWx0c2FsdA==");
    let fake = salt("nobody@x.com").await;
    assert!(fake.is_string() && fake != "c2FsdHNhbHRzYWx0c2FsdA==");
    assert_eq!(fake, salt("nobody@x.com").await, "same answer every time, so it looks real");
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
    let bad = json!({ "email": "a@x.com", "login_hash": "nope" });
    for _ in 0..10 {
        assert_eq!(call(&app, "/login", None, Some(bad.clone())).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(call(&app, "/login", None, Some(bad)).await.0, StatusCode::TOO_MANY_REQUESTS);
    let good = json!({ "email": "a@x.com", "login_hash": "right-hash" });
    assert_eq!(call(&app, "/login", None, Some(good)).await.0, StatusCode::TOO_MANY_REQUESTS, "even the right password waits");
}
