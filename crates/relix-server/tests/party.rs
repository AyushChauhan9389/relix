//! Watch Together over the live WebSocket (ADR 0010), against a real running server.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use relix_proto::social::{PartyTitle, ToClient, ToServer};
use relix_server::{Config, Mailer, app};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const ADMIN: &str = "admin-secret";

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A server on a random local port; returns its base URL.
async fn server() -> String {
    let dir = std::env::temp_dir().join(format!("relix-party-{}", std::process::id()));
    let config = Config { verify_email: false, mailer: Mailer::Outbox(Arc::new(Mutex::new(vec![]))), admin_token: Some(ADMIN.into()), update_dir: dir };
    let router = app(rusqlite::Connection::open_in_memory().unwrap(), config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("127.0.0.1:{}", addr.port())
}

async fn rest(base: &str, method: &str, path: &str, token: &str, body: Option<Value>) -> (u16, Value) {
    let http = reqwest::Client::new();
    let url = format!("http://{base}{path}");
    let req = match method {
        "POST" => http.post(url),
        "PUT" => http.put(url),
        _ => http.get(url),
    };
    let req = req.bearer_auth(token);
    let res = match body {
        Some(b) => req.json(&b).send().await.unwrap(),
        None => req.send().await.unwrap(),
    };
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

/// A signed-up account with a username; returns its token.
async fn user(base: &str, name: &str) -> String {
    let http = reqwest::Client::new();
    let body: Value = http
        .post(format!("http://{base}/signup"))
        .json(&json!({ "email": format!("{name}@x.com"), "password": "right password" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = body["session"]["token"].as_str().unwrap().to_owned();
    assert_eq!(rest(base, "PUT", "/me/username", &token, Some(json!({ "username": name }))).await.0, 204);
    token
}

async fn connect(base: &str, token: &str) -> Ws {
    let mut req = format!("ws://{base}/ws").into_client_request().unwrap();
    req.headers_mut().insert("authorization", format!("Bearer {token}").parse().unwrap());
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

async fn send(ws: &mut Ws, msg: ToServer) {
    ws.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await.unwrap();
}

/// The next message matching `want`, skipping others; fails after a few seconds.
async fn expect(ws: &mut Ws, want: impl Fn(&ToClient) -> bool) -> ToClient {
    let wait = async {
        loop {
            let Some(Ok(Message::Text(text))) = ws.next().await else { panic!("connection closed") };
            let msg: ToClient = serde_json::from_str(&text).unwrap();
            if want(&msg) {
                return msg;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), wait).await.expect("no matching message")
}

fn title() -> PartyTitle {
    PartyTitle { id: "tt1".into(), kind: "movie".into(), name: "Test Movie".into(), poster: None, episode: String::new() }
}

/// Host and viewer in one party; returns (host ws, viewer ws, code).
async fn party(base: &str) -> (Ws, Ws, String) {
    let host = user(base, "host").await;
    assert_eq!(rest(base, "PUT", "/admin/preview/host", ADMIN, None).await.0, 204);
    let viewer = user(base, "viewer").await;
    let (mut h, mut v) = (connect(base, &host).await, connect(base, &viewer).await);
    send(&mut h, ToServer::Create { title: title(), url: "https://example.com/v.mkv".into() }).await;
    let ToClient::Party(view) = expect(&mut h, |m| matches!(m, ToClient::Party(_))).await else { unreachable!() };
    assert_eq!((view.host.as_str(), view.members.len()), ("host", 1));
    send(&mut v, ToServer::Join { code: view.code.to_lowercase() }).await;
    let ToClient::Party(joined) = expect(&mut v, |m| matches!(m, ToClient::Party(_))).await else { unreachable!() };
    assert_eq!(joined.members.iter().map(|m| m.username.as_str()).collect::<Vec<_>>(), ["host", "viewer"]);
    expect(&mut v, |m| matches!(m, ToClient::Sync { playing: false, .. })).await;
    (h, v, view.code)
}

#[tokio::test]
async fn hosting_needs_the_preview() {
    let base = server().await;
    let token = user(&base, "ann").await;
    let mut ws = connect(&base, &token).await;
    send(&mut ws, ToServer::Create { title: title(), url: "https://example.com/v.mkv".into() }).await;
    let ToClient::Error { message } = expect(&mut ws, |m| matches!(m, ToClient::Error { .. })).await else { unreachable!() };
    assert!(message.contains("preview"), "{message}");
}

#[tokio::test]
async fn host_controls_and_everyone_follows() {
    let base = server().await;
    let (mut h, mut v, _) = party(&base).await;
    send(&mut h, ToServer::Control { playing: true, position: 10. }).await;
    let ToClient::Sync { playing, position, by } = expect(&mut v, |m| matches!(m, ToClient::Sync { by: Some(_), .. })).await else { unreachable!() };
    assert!(playing && (10. ..11.).contains(&position));
    assert_eq!(by.as_deref(), Some("host"));
    expect(&mut v, |m| matches!(m, ToClient::Notice { text } if text == "host resumed")).await;

    // Only the host, until control is opened.
    send(&mut v, ToServer::Control { playing: false, position: 20. }).await;
    expect(&mut v, |m| matches!(m, ToClient::Error { .. })).await;
    send(&mut h, ToServer::OpenControl { open: true }).await;
    expect(&mut v, |m| matches!(m, ToClient::Party(p) if p.open_control)).await;
    send(&mut v, ToServer::Control { playing: false, position: 20. }).await;
    let ToClient::Sync { playing, position, .. } = expect(&mut h, |m| matches!(m, ToClient::Sync { by: Some(b), .. } if b == "viewer")).await else { unreachable!() };
    assert_eq!((playing, position), (false, 20.));
}

#[tokio::test]
async fn everyone_waits_for_a_buffering_member() {
    let base = server().await;
    let (mut h, mut v, _) = party(&base).await;
    send(&mut h, ToServer::Control { playing: true, position: 0. }).await;
    expect(&mut v, |m| matches!(m, ToClient::Sync { playing: true, .. })).await;

    send(&mut v, ToServer::Buffering { buffering: true }).await;
    expect(&mut h, |m| matches!(m, ToClient::Sync { playing: false, by: None, .. })).await;
    expect(&mut h, |m| matches!(m, ToClient::Notice { text } if text.contains("Waiting for viewer"))).await;
    send(&mut v, ToServer::Buffering { buffering: false }).await;
    expect(&mut h, |m| matches!(m, ToClient::Sync { playing: true, .. })).await;
}

#[tokio::test]
async fn chat_reaches_everyone_and_the_host_role_passes_on() {
    let base = server().await;
    let (mut h, mut v, _) = party(&base).await;
    send(&mut v, ToServer::Chat { text: "  hi!  ".into() }).await;
    let ToClient::Chat { from, text } = expect(&mut h, |m| matches!(m, ToClient::Chat { .. })).await else { unreachable!() };
    assert_eq!((from.as_str(), text.as_str()), ("viewer", "hi!"));

    send(&mut h, ToServer::Leave).await;
    expect(&mut h, |m| matches!(m, ToClient::Left { .. })).await;
    expect(&mut v, |m| matches!(m, ToClient::Party(p) if p.host == "viewer" && p.members.len() == 1)).await;
}

#[tokio::test]
async fn kicked_members_cant_come_back() {
    let base = server().await;
    let (mut h, mut v, code) = party(&base).await;
    send(&mut h, ToServer::Kick { username: "viewer".into() }).await;
    expect(&mut v, |m| matches!(m, ToClient::Left { reason } if reason.contains("removed"))).await;
    send(&mut v, ToServer::Join { code }).await;
    expect(&mut v, |m| matches!(m, ToClient::Error { message } if message.contains("removed"))).await;
}

#[tokio::test]
async fn invites_go_to_friends_and_presence_shows_online() {
    let base = server().await;
    let (mut h, _v, code) = party(&base).await;
    let friend = user(&base, "pal").await;
    let host_token = {
        // Sign in again as the host for the REST calls (the WebSocket already holds a session).
        let http = reqwest::Client::new();
        let body: Value = http
            .post(format!("http://{base}/login"))
            .json(&json!({ "email": "host@x.com", "password": "right password" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["token"].as_str().unwrap().to_owned()
    };
    send(&mut h, ToServer::Invite { username: "pal".into() }).await;
    expect(&mut h, |m| matches!(m, ToClient::Error { message } if message.contains("isn't your friend"))).await;

    assert_eq!(rest(&base, "POST", "/friends/request", &host_token, Some(json!({ "username": "pal" }))).await.0, 204);
    assert_eq!(rest(&base, "POST", "/friends/accept", &friend, Some(json!({ "username": "host" }))).await.0, 204);
    let mut p = connect(&base, &friend).await;
    expect(&mut h, |m| matches!(m, ToClient::Presence { username, online: true, .. } if username == "pal")).await;
    send(&mut h, ToServer::Invite { username: "pal".into() }).await;
    let ToClient::Invited { from, code: invited, title } = expect(&mut p, |m| matches!(m, ToClient::Invited { .. })).await else { unreachable!() };
    assert_eq!((from.as_str(), invited, title.name.as_str()), ("host", code, "Test Movie"));

    send(&mut p, ToServer::Presence { watching: Some("Test Movie".into()), hidden: false }).await;
    expect(&mut h, |m| matches!(m, ToClient::Presence { watching: Some(w), .. } if w == "Test Movie")).await;
    let (_, friends) = rest(&base, "GET", "/friends", &host_token, None).await;
    assert_eq!(friends["friends"], json!([{ "username": "pal", "online": true, "watching": "Test Movie" }]));
}
