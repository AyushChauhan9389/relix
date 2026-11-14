//! Friends and Watch Together on the app side (ADR 0010): the live connection to the Relix server,
//! the state the screens read, and the player that follows a party.

use std::time::{Duration, Instant};

use freya::prelude::*;
use futures_util::{SinkExt, StreamExt};
use relix_proto::social::{Friends, Me, PartyTitle, PartyView, ToClient, ToServer, Username};
use reqwest::Method;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::sync::{account, call, send, server_url};
use crate::{Ctx, player};

/// A line in the party's chat panel: a message, or a notice when `from` is empty.
#[derive(Clone, PartialEq)]
pub struct Line {
    pub from: String,
    pub text: String,
}

#[derive(Clone, PartialEq)]
pub struct Invite {
    pub from: String,
    pub code: String,
    pub title: PartyTitle,
}

/// Wakes the live loop when it waits to connect (sign-in, a new username).
static WAKE: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);

pub fn wake() {
    WAKE.notify_one();
}

/// Everything social the screens show; part of [`Ctx`].
#[derive(Clone, Copy)]
pub struct Social {
    pub me: State<Option<Me>>,
    pub friends: State<Friends>,
    pub party: State<Option<PartyView>>,
    pub lines: State<Vec<Line>>,
    pub invites: State<Vec<Invite>>,
    /// The last thing the server refused or a request failed with.
    pub error: State<Option<String>>,
    out: State<Option<UnboundedSender<ToServer>>>,
    /// The player on screen is the party's (its controls go through the server).
    pub party_player: State<bool>,
    /// The chat panel beside the party's video.
    pub chat_open: State<bool>,
    /// The last party this account hosted from a Google Drive file: (file id, title), until the
    /// host deletes the file or keeps it (drive.rs).
    pub hosted_drive_file: State<Option<(String, String)>>,
    /// The party to rejoin once the connection is back (it dropped mid-party).
    rejoin: State<Option<String>>,
    /// A position to apply once the party's video has loaded.
    pending: State<Option<(bool, f64)>>,
}

impl Social {
    pub fn new() -> Self {
        Social {
            me: use_state(|| None),
            friends: use_state(Friends::default),
            party: use_state(|| None),
            lines: use_state(Vec::new),
            invites: use_state(Vec::new),
            error: use_state(|| None),
            out: use_state(|| None),
            party_player: use_state(|| false),
            chat_open: use_state(|| true),
            hosted_drive_file: use_state(|| None),
            rejoin: use_state(|| None),
            pending: use_state(|| None),
        }
    }

    pub fn connected(&self) -> bool {
        self.out.read().is_some()
    }

    pub fn in_party_player(&self) -> bool {
        *self.party_player.peek()
    }
}

/// Sends a message over the live connection (dropped when offline).
pub fn send_live(ctx: &Ctx, msg: ToServer) {
    if let Some(tx) = ctx.social.out.peek().as_ref() {
        let _ = tx.send(msg);
    }
}

fn token(ctx: &Ctx) -> Result<String, String> {
    account(&ctx.store).map(|a| a.token).ok_or_else(|| "Sign in first".to_owned())
}

/// Fetches who this account is and its friends.
pub async fn refresh(ctx: &Ctx) {
    let Ok(token) = token(ctx) else {
        let (mut me, mut friends) = (ctx.social.me, ctx.social.friends);
        me.set(None);
        friends.set(Friends::default());
        return;
    };
    if let Ok(me) = call::<(), Me>(&ctx.http, server_url("/me"), Some(&token), None).await {
        let mut state = ctx.social.me;
        state.set(Some(me));
    }
    if let Ok(friends) = call::<(), Friends>(&ctx.http, server_url("/friends"), Some(&token), None).await {
        let mut state = ctx.social.friends;
        state.set(friends);
    }
}

pub async fn set_username(ctx: &Ctx, name: &str) -> Result<(), String> {
    let body = Username { username: name.trim().to_owned() };
    send(&ctx.http, Method::PUT, server_url("/me/username"), Some(&token(ctx)?), Some(&body)).await?;
    refresh(ctx).await;
    // The live connection needs a username; with one it can start.
    wake();
    Ok(())
}

/// A friend action: "request", "accept", "decline", "block", or "remove".
pub async fn friend(ctx: &Ctx, action: &str, name: &str) -> Result<(), String> {
    let token = token(ctx)?;
    let body = Username { username: name.trim().to_owned() };
    match action {
        "remove" => send(&ctx.http, Method::DELETE, server_url(&format!("/friends/{}", body.username)), Some(&token), None::<&()>).await?,
        other => send(&ctx.http, Method::POST, server_url(&format!("/friends/{other}")), Some(&token), Some(&body)).await?,
    };
    refresh(ctx).await;
    Ok(())
}

fn ws_url() -> String {
    let http = server_url("/ws");
    match http.strip_prefix("https://") {
        Some(rest) => format!("wss://{rest}"),
        None => http.replacen("http://", "ws://", 1),
    }
}

/// Keeps the live connection up while signed in with a username; reconnects after drops.
pub async fn run(ctx: Ctx) {
    loop {
        refresh(&ctx).await;
        let has_name = ctx.social.me.peek().as_ref().is_some_and(|m| m.username.is_some());
        let token = token(&ctx);
        if let (true, Ok(token)) = (has_name, token) {
            match connect(&ctx, &token).await {
                Ok(()) => crate::log!("live: disconnected"),
                Err(e) => crate::log!("live: {e}"),
            }
            on_disconnect(&ctx);
        }
        // Sign-in, a new username, or just a pause before trying again.
        let _ = tokio::time::timeout(Duration::from_secs(5), WAKE.notified()).await;
    }
}

fn on_disconnect(ctx: &Ctx) {
    let (mut out, mut party, mut rejoin) = (ctx.social.out, ctx.social.party, ctx.social.rejoin);
    out.set(None);
    let Some(code) = party.peek().as_ref().map(|p| p.code.clone()) else {
        return;
    };
    // Signed out: the party is over for this device. Otherwise keep watching and rejoin on reconnect.
    if account(&ctx.store).is_none() {
        party.set(None);
        stop_party_player(ctx);
    } else {
        push(ctx, "", "Connection lost; reconnecting…".into());
        rejoin.set(Some(code));
    }
}

async fn connect(ctx: &Ctx, token: &str) -> Result<(), String> {
    let mut req = ws_url().into_client_request().map_err(|e| e.to_string())?;
    req.headers_mut().insert("authorization", format!("Bearer {token}").parse().map_err(|_| "bad token")?);
    let (socket, _) = tokio_tungstenite::connect_async(req).await.map_err(|e| e.to_string())?;
    crate::log!("live: connected");
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = unbounded_channel::<ToServer>();
    let mut out = ctx.social.out;
    out.set(Some(tx));
    send_presence(ctx);
    // Back after a drop mid-party: the server answers with the party (or an error if it ended).
    if let Some(code) = ctx.social.rejoin.peek().clone() {
        send_live(ctx, ToServer::Join { code });
    }
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { return Ok(()) };
                let text = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
                sink.send(Message::Text(text.into())).await.map_err(|e| e.to_string())?;
            }
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(msg) = serde_json::from_str::<ToClient>(&text) {
                        handle(ctx, msg);
                    }
                }
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(e)) => return Err(e.to_string()),
                _ => {}
            },
            // Keeps the connection from looking idle to proxies; signing out ends it.
            _ = tokio::time::sleep(Duration::from_secs(25)) => {
                if account(&ctx.store).is_none() {
                    return Ok(());
                }
                sink.send(Message::Ping(Vec::new().into())).await.map_err(|e| e.to_string())?;
            }
        }
    }
}

fn push(ctx: &Ctx, from: &str, text: String) {
    let mut lines = ctx.social.lines;
    let mut l = lines.write();
    l.push(Line { from: from.to_owned(), text });
    // ponytail: the panel keeps the last 200 lines; scrollback beyond that isn't needed in a party.
    let excess = l.len().saturating_sub(200);
    l.drain(..excess);
}

fn handle(ctx: &Ctx, msg: ToClient) {
    let social = ctx.social;
    match msg {
        ToClient::FriendsChanged => {
            let ctx = ctx.clone();
            spawn(async move { refresh(&ctx).await });
        }
        ToClient::Presence { username, online, watching } => {
            let mut friends = social.friends;
            if let Some(f) = friends.write().friends.iter_mut().find(|f| f.username == username) {
                f.online = online;
                f.watching = watching;
            }
        }
        ToClient::Invited { from, code, title } => {
            let mut invites = social.invites;
            invites.write().retain(|i| i.code != code);
            invites.write().push(Invite { from, code, title });
        }
        ToClient::Party(view) => {
            let was = social.party.peek().as_ref().map(|p| p.code.clone());
            let new = was.as_deref() != Some(view.code.as_str());
            let (mut party, mut lines, mut invites, mut rejoin) =
                (social.party, social.lines, social.invites, social.rejoin);
            rejoin.set(None);
            invites.write().retain(|i| i.code != view.code);
            if new {
                lines.set(vec![]);
            }
            // Hosting from Drive: offer to delete the file once the party is over.
            let me = social.me.peek().as_ref().and_then(|m| m.username.clone());
            if me.as_deref() == Some(view.host.as_str())
                && let Some(id) = crate::drive::file_id(&view.url)
            {
                let mut hosted = social.hosted_drive_file;
                if hosted.peek().as_ref().map(|(i, _)| i) != Some(&id) {
                    hosted.set(Some((id, view.title.name.clone())));
                }
            }
            party.set(Some(view));
            if new {
                start_party_player(ctx);
            }
        }
        ToClient::Sync { playing, position, .. } => apply_sync(ctx, playing, position),
        ToClient::Chat { from, text } => push(ctx, &from, text),
        ToClient::Notice { text } => push(ctx, "", text),
        ToClient::Left { reason } => {
            let mut party = social.party;
            party.set(None);
            push(ctx, "", format!("You left: {reason}"));
            stop_party_player(ctx);
        }
        // Rejoining failed: the party ended (or the host removed us) while we were away.
        ToClient::Error { message } if social.rejoin.peek().is_some() => {
            let (mut party, mut rejoin) = (social.party, social.rejoin);
            rejoin.set(None);
            party.set(None);
            push(ctx, "", message);
            stop_party_player(ctx);
        }
        ToClient::Error { message } => {
            let mut error = social.error;
            error.set(Some(message.clone()));
            if social.party.peek().is_some() {
                push(ctx, "", message);
            }
        }
    }
}

/// Tells friends what's playing (or nothing), following the viewer's settings.
pub fn send_presence(ctx: &Ctx) {
    let hidden = ctx.store.get::<bool>(&ctx.profile, "settings", "appear_offline").ok().flatten().unwrap_or(false);
    let share = ctx.store.get::<bool>(&ctx.profile, "settings", "share_watching").ok().flatten().unwrap_or(true);
    let watching = ctx.now_playing.peek().as_ref().filter(|_| share).map(|np| np.title.clone());
    send_live(ctx, ToServer::Presence { watching, hidden });
}

/// Moves this player to where the party is: playing or paused, and within 1.5s of its position.
fn apply_sync(ctx: &Ctx, playing: bool, position: f64) {
    let status = ctx.player_status.peek().clone();
    let np = ctx.now_playing.peek().clone();
    // Until the party's own player has loaded (it may not have even started yet), keep the latest.
    let (Some(np), true, true) = (np, status.loaded, *ctx.social.party_player.peek()) else {
        let mut pending = ctx.social.pending;
        pending.set(Some((playing, position)));
        return;
    };
    let mpv = np.video.mpv();
    let _ = mpv.set_property("pause", !playing);
    if (status.position - position).abs() > 1.5 {
        let _ = mpv.command("seek", &[&format!("{position:.3}"), "absolute+exact"]);
    }
}

/// Pause or play. In a party it asks the server, which tells everyone (this player included).
pub fn set_paused(ctx: &Ctx, mpv: &libmpv2::Mpv, paused: bool) {
    if ctx.social.in_party_player() {
        let position = ctx.player_status.peek().position;
        send_live(ctx, ToServer::Control { playing: !paused, position });
    } else {
        let _ = mpv.set_property("pause", paused);
    }
}

/// Seek to `seconds`. In a party it goes through the server like [`set_paused`].
pub fn seek_to(ctx: &Ctx, mpv: &libmpv2::Mpv, seconds: f64) {
    let seconds = seconds.max(0.);
    if ctx.social.in_party_player() {
        let playing = !ctx.player_status.peek().paused;
        send_live(ctx, ToServer::Control { playing, position: seconds });
    } else {
        let _ = mpv.command("seek", &[&format!("{seconds:.3}"), "absolute+exact"]);
    }
}

/// Seek `delta` seconds from here.
pub fn seek_by(ctx: &Ctx, mpv: &libmpv2::Mpv, delta: f64) {
    if ctx.social.in_party_player() {
        let position = ctx.player_status.peek().position + delta;
        seek_to(ctx, mpv, position);
    } else {
        let _ = mpv.command("seek", &[&format!("{delta}"), "relative"]);
    }
}

/// Opens the party's video in the player; it then follows the party's syncs.
fn start_party_player(ctx: &Ctx) {
    let Some(view) = ctx.social.party.peek().clone() else { return };
    // Whatever was playing gives way to the party.
    if let Some(p) = ctx.player.peek().as_ref() {
        let _ = p.send(player::ToPlayer::Stop);
    }
    let ctx = ctx.clone();
    spawn(async move {
        // Let a stopped player finish closing first.
        while ctx.player.peek().is_some() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (mut playing, mut party_player, mut pending) = (ctx.player, ctx.social.party_player, ctx.social.pending);
        party_player.set(true);
        let (to_player, inbox) = std::sync::mpsc::channel();
        playing.set(Some(to_player));
        let overlay = player::Overlay {
            title: view.title.name.clone(),
            episode: view.title.episode.clone(),
            has_next: false,
            logo: None,
            background: view.title.poster.clone(),
        };
        let (to_ui, mut mpv_rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_in_window({
            let (ctx, overlay) = (ctx.clone(), overlay.clone());
            async move {
                if let Some((mpv, ready)) = mpv_rx.recv().await {
                    ctx.show_video(mpv, ready, overlay);
                    send_presence(&ctx);
                }
            }
        });
        let (to_app, mut from_player) = tokio::sync::mpsc::unbounded_channel();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (url, prefs, title) = (view.url.clone(), ctx.player_prefs(), view.title.name.clone());
        std::thread::spawn(move || {
            let video = Box::new(move |mpv, ready| drop(to_ui.send((mpv, ready))));
            let played = player::play(&url, &Default::default(), 0., &title, &prefs, Some(video), inbox, |msg| {
                let _ = to_app.send(msg);
            });
            let _ = tx.send(played);
        });
        // The player's state: on screen, plus what the party needs from it.
        spawn_in_window({
            let ctx = ctx.clone();
            async move {
                let mut status = ctx.player_status;
                let (mut was_waiting, mut beat) = (false, Instant::now());
                while let Some::<player::Status>(s) = from_player.recv().await {
                    let loaded_now = s.loaded && !status.peek().loaded;
                    status.set(s.clone());
                    if loaded_now && let Some((playing, position)) = pending.write().take() {
                        apply_sync(&ctx, playing, position);
                    }
                    // Network stalls only: a seek's brief pause would announce "Waiting for…" on every skip.
                    let waiting = s.loaded && s.buffering;
                    if waiting != was_waiting {
                        was_waiting = waiting;
                        send_live(&ctx, ToServer::Buffering { buffering: waiting });
                    }
                    let me = ctx.social.me.peek().as_ref().and_then(|m| m.username.clone());
                    let host = ctx.social.party.peek().as_ref().map(|p| p.host.clone());
                    if s.loaded && !s.paused && me.is_some() && me == host && beat.elapsed() >= Duration::from_secs(5) {
                        beat = Instant::now();
                        send_live(&ctx, ToServer::Heartbeat { position: s.position });
                    }
                }
            }
        });
        let ended = rx.await;
        if let Ok(Ok((_, Some(player::Ended::Failed)))) | Ok(Err(_)) = &ended {
            push(&ctx, "", "The party's video didn't play: the link is dead, private, or not a video.".into());
        }
        playing.set(None);
        let mut now = ctx.now_playing;
        now.set(None);
        party_player.set(false);
        pending.set(None);
        send_presence(&ctx);
        // Closing the player leaves the party (if it hasn't ended already); offline, just forget it.
        let (mut party, mut rejoin) = (ctx.social.party, ctx.social.rejoin);
        rejoin.set(None);
        if party.peek().is_some() {
            if ctx.social.connected() {
                send_live(&ctx, ToServer::Leave);
            } else {
                party.set(None);
            }
        }
    });
}

fn stop_party_player(ctx: &Ctx) {
    if *ctx.social.party_player.peek()
        && let Some(p) = ctx.player.peek().as_ref()
    {
        let _ = p.send(player::ToPlayer::Stop);
    }
}

/// Starts a party for `title` playing `link` (a Drive share link or a direct video URL), after
/// checking the link really serves something.
pub async fn create_party(ctx: &Ctx, title: PartyTitle, link: &str) -> Result<(), String> {
    let url = relix_proto::social::direct_video_url(link);
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Paste a Google Drive share link or a video link (http/https)".into());
    }
    crate::log!("party: checking {}", crate::debug::host(&url));
    let res = ctx
        .http
        .get(&url)
        .header("range", "bytes=0-1023")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Couldn't open the link: {}", e.without_url()))?;
    let kind = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    if !res.status().is_success() || kind.starts_with("text/html") {
        return Err("That link doesn't give a video: in Drive, share it as \"Anyone with the link\" and copy the link again".into());
    }
    crate::log!("party: link answers {} {kind}", res.status());
    if !ctx.social.connected() {
        return Err("Not connected to the Relix server; try again in a moment".into());
    }
    send_live(ctx, ToServer::Create { title, url });
    Ok(())
}
