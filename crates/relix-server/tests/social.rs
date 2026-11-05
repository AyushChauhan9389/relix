//! Usernames, friends and Watch Together hosts (ADR 0010): the REST side.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relix_server::{Config, Mailer, app};
use serde_json::{Value, json};
use tower::ServiceExt;

const ADMIN: &str = "admin-secret";

fn server() -> Router {
    let dir = std::env::temp_dir().join(format!("relix-social-{}", std::process::id()));
    let config = Config { verify_email: false, mailer: Mailer::Outbox(Arc::new(Mutex::new(vec![]))), admin_token: Some(ADMIN.into()), update_dir: dir };
    app(rusqlite::Connection::open_in_memory().unwrap(), config).unwrap()
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

/// A signed-up account with a username; returns its token.
async fn user(app: &Router, name: &str) -> String {
    let (status, body) = send(app, "POST", "/signup", None, Some(json!({ "email": format!("{name}@x.com"), "password": "right password" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["session"]["token"].as_str().unwrap().to_owned();
    let (status, body) = send(app, "PUT", "/me/username", Some(&token), Some(json!({ "username": name }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    token
}

#[tokio::test]
async fn usernames_are_unique_and_validated() {
    let app = server();
    let (_, body) = send(&app, "POST", "/signup", None, Some(json!({ "email": "a@x.com", "password": "right password" }))).await;
    let token = body["session"]["token"].as_str().unwrap().to_owned();
    let (status, me) = send(&app, "GET", "/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me, json!({ "email": "a@x.com", "username": null, "can_host": false }));

    let set = |name: &str| json!({ "username": name });
    assert_eq!(send(&app, "PUT", "/me/username", Some(&token), Some(set("no spaces"))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(send(&app, "PUT", "/me/username", Some(&token), Some(set("Ayush"))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "GET", "/me", Some(&token), None).await.1["username"], "Ayush");

    let other = user(&app, "sam").await;
    let (status, body) = send(&app, "PUT", "/me/username", Some(&other), Some(set("ayush"))).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("that username is taken")), "taken regardless of case");
    // Renaming frees the old name.
    assert_eq!(send(&app, "PUT", "/me/username", Some(&token), Some(set("ayush2"))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "PUT", "/me/username", Some(&other), Some(set("ayush"))).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn friend_requests_become_friendships() {
    let app = server();
    let (a, b) = (user(&app, "ann").await, user(&app, "bob").await);
    let name = |n: &str| json!({ "username": n });

    assert_eq!(send(&app, "POST", "/friends/request", Some(&a), Some(name("nobody"))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(&app, "POST", "/friends/request", Some(&a), Some(name("ann"))).await.0, StatusCode::BAD_REQUEST, "not yourself");
    assert_eq!(send(&app, "POST", "/friends/request", Some(&a), Some(name("bob"))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "GET", "/friends", Some(&a), None).await.1, json!({ "friends": [], "incoming": [], "outgoing": ["bob"] }));
    assert_eq!(send(&app, "GET", "/friends", Some(&b), None).await.1, json!({ "friends": [], "incoming": ["ann"], "outgoing": [] }));

    assert_eq!(send(&app, "POST", "/friends/accept", Some(&b), Some(name("ann"))).await.0, StatusCode::NO_CONTENT);
    let friends = send(&app, "GET", "/friends", Some(&a), None).await.1;
    assert_eq!(friends["friends"], json!([{ "username": "bob", "online": false, "watching": null }]));
    assert_eq!((friends["incoming"].clone(), friends["outgoing"].clone()), (json!([]), json!([])));

    assert_eq!(send(&app, "DELETE", "/friends/bob", Some(&a), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "GET", "/friends", Some(&b), None).await.1["friends"], json!([]), "removal is mutual");
}

#[tokio::test]
async fn crossed_requests_accept_each_other_and_blocks_stop_requests() {
    let app = server();
    let (a, b) = (user(&app, "ann").await, user(&app, "bob").await);
    let name = |n: &str| json!({ "username": n });
    send(&app, "POST", "/friends/request", Some(&a), Some(name("bob"))).await;
    // Bob asking Ann back is the same as accepting.
    send(&app, "POST", "/friends/request", Some(&b), Some(name("ann"))).await;
    assert_eq!(send(&app, "GET", "/friends", Some(&a), None).await.1["friends"][0]["username"], "bob");

    let c = user(&app, "cat").await;
    assert_eq!(send(&app, "POST", "/friends/block", Some(&c), Some(name("ann"))).await.0, StatusCode::NO_CONTENT);
    let (status, body) = send(&app, "POST", "/friends/request", Some(&a), Some(name("cat"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a blocked user can't tell they're blocked: {body}");
    assert_eq!(send(&app, "GET", "/friends", Some(&c), None).await.1["incoming"], json!([]));
    // Declining clears a request.
    let d = user(&app, "dan").await;
    send(&app, "POST", "/friends/request", Some(&d), Some(name("bob"))).await;
    assert_eq!(send(&app, "POST", "/friends/decline", Some(&b), Some(name("dan"))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "GET", "/friends", Some(&d), None).await.1["outgoing"], json!([]));
}

#[tokio::test]
async fn admin_picks_who_can_host() {
    let app = server();
    let token = user(&app, "ann").await;
    assert_eq!(send(&app, "PUT", "/admin/preview/ann", Some("wrong"), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&app, "PUT", "/admin/preview/nobody", Some(ADMIN), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(&app, "PUT", "/admin/preview/ann", Some(ADMIN), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "GET", "/me", Some(&token), None).await.1["can_host"], true);
    assert_eq!(send(&app, "GET", "/admin/preview", Some(ADMIN), None).await.1, json!(["ann"]));
    assert_eq!(send(&app, "DELETE", "/admin/preview/ann", Some(ADMIN), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "GET", "/me", Some(&token), None).await.1["can_host"], false);
}
