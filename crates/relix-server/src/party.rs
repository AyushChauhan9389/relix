//! Watch Together (ADR 0010): every signed-in app keeps one WebSocket here for presence, parties and
//! chat. Parties live in memory only; a restart ends them.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use relix_proto::social::{Member, PartyTitle, PartyView, ToClient, ToServer};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::AppState;

/// Private parties hold at most this many people.
const MAX_MEMBERS: usize = 50;
const MAX_CHAT: usize = 500;

#[derive(Default)]
pub struct Hub {
    // ponytail: one lock for the whole hub; per-party locks if it ever serves thousands of parties.
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Every open connection per account (several devices can be signed in at once).
    conns: HashMap<i64, Vec<(u64, UnboundedSender<ToClient>)>>,
    next_conn: u64,
    presence: HashMap<i64, Presence>,
    parties: HashMap<String, Party>,
    in_party: HashMap<i64, String>,
}

#[derive(Default, Clone)]
struct Presence {
    watching: Option<String>,
    hidden: bool,
}

struct Party {
    title: PartyTitle,
    url: String,
    host: i64,
    /// (account, username, buffering), in join order.
    members: Vec<(i64, String, bool)>,
    open_control: bool,
    banned: HashSet<i64>,
    /// Whether the controllers want it playing; it only plays while nobody is buffering.
    playing: bool,
    /// Position at `at`; it moves on from there while the party is actually playing.
    position: f64,
    at: Instant,
}

impl Party {
    fn waiting(&self) -> bool {
        self.members.iter().any(|m| m.2)
    }

    fn effective_playing(&self) -> bool {
        self.playing && !self.waiting()
    }

    fn position_now(&self) -> f64 {
        self.position + if self.effective_playing() { self.at.elapsed().as_secs_f64() } else { 0. }
    }

    /// Fixes the current position as the new starting point, before anything changes how it moves.
    fn rebase(&mut self) {
        self.position = self.position_now();
        self.at = Instant::now();
    }

    fn username(&self, account: i64) -> String {
        self.members.iter().find(|m| m.0 == account).map(|m| m.1.clone()).unwrap_or_default()
    }

    fn view(&self, code: &str) -> PartyView {
        PartyView {
            code: code.to_owned(),
            title: self.title.clone(),
            url: self.url.clone(),
            host: self.username(self.host),
            members: self.members.iter().map(|(_, name, buffering)| Member { username: name.clone(), buffering: *buffering }).collect(),
            open_control: self.open_control,
        }
    }

    fn sync(&self, by: Option<String>) -> ToClient {
        ToClient::Sync { playing: self.effective_playing(), position: self.position_now(), by }
    }
}

impl Inner {
    fn send(&self, account: i64, msg: &ToClient) {
        for (_, tx) in self.conns.get(&account).into_iter().flatten() {
            let _ = tx.send(msg.clone());
        }
    }

    fn broadcast(&self, code: &str, msg: &ToClient) {
        if let Some(p) = self.parties.get(code) {
            for (account, ..) in &p.members {
                self.send(*account, msg);
            }
        }
    }

    fn broadcast_view(&self, code: &str) {
        if let Some(p) = self.parties.get(code) {
            self.broadcast(code, &ToClient::Party(p.view(code)));
        }
    }

    fn notice(&self, code: &str, text: String) {
        self.broadcast(code, &ToClient::Notice { text });
    }

    /// Takes `account` out of its party; the next member becomes host, and an empty party ends.
    fn leave(&mut self, account: i64, reason: &str) {
        let Some(code) = self.in_party.remove(&account) else { return };
        let Some(p) = self.parties.get_mut(&code) else { return };
        p.rebase();
        let name = p.username(account);
        p.members.retain(|m| m.0 != account);
        if p.members.is_empty() {
            self.parties.remove(&code);
        } else {
            let new_host = (p.host == account).then(|| p.members[0].clone());
            if let Some((id, host_name, _)) = &new_host {
                p.host = *id;
                let host_name = host_name.clone();
                self.notice(&code, format!("{name} left; {host_name} is now the host"));
            } else {
                self.notice(&code, format!("{name} left"));
            }
            self.broadcast_view(&code);
            let sync = self.parties[&code].sync(None);
            self.broadcast(&code, &sync);
        }
        self.send(account, &ToClient::Left { reason: reason.to_owned() });
    }
}

impl Hub {
    /// Whether the account has an open connection, and what it shares that it's watching.
    pub fn presence(&self, account: i64) -> (bool, Option<String>) {
        let inner = self.inner.lock().unwrap();
        let online = inner.conns.get(&account).is_some_and(|c| !c.is_empty());
        match inner.presence.get(&account) {
            Some(p) if p.hidden => (false, None),
            Some(p) if online => (true, p.watching.clone()),
            _ => (online, None),
        }
    }

    pub fn send(&self, account: i64, msg: &ToClient) {
        self.inner.lock().unwrap().send(account, msg);
    }

    /// Tells the account's online friends where it is now.
    fn announce(&self, friends: &[i64], username: &str, account: i64) {
        let (online, watching) = self.presence(account);
        let msg = ToClient::Presence { username: username.to_owned(), online, watching };
        let inner = self.inner.lock().unwrap();
        for f in friends {
            inner.send(*f, &msg);
        }
    }

    fn handle(&self, account: i64, me: &str, msg: ToServer, can_host: impl FnOnce() -> bool, friend: impl FnOnce(&str) -> Option<i64>) {
        let mut inner = self.inner.lock().unwrap();
        let error = |inner: &Inner, text: &str| inner.send(account, &ToClient::Error { message: text.to_owned() });
        let code = inner.in_party.get(&account).cloned();
        match msg {
            // Handled by the connection (it needs the friends list).
            ToServer::Presence { .. } => {}
            ToServer::Create { title, url } => {
                drop(inner);
                if !can_host() {
                    return self.send(account, &ToClient::Error { message: "Watch Together is a preview: ask for access to host".into() });
                }
                let mut inner = self.inner.lock().unwrap();
                inner.leave(account, "you started another party");
                let code = loop {
                    let code = party_code();
                    if !inner.parties.contains_key(&code) {
                        break code;
                    }
                };
                inner.parties.insert(code.clone(), Party {
                    title,
                    url,
                    host: account,
                    members: vec![(account, me.to_owned(), false)],
                    open_control: false,
                    banned: HashSet::new(),
                    playing: false,
                    position: 0.,
                    at: Instant::now(),
                });
                inner.in_party.insert(account, code.clone());
                inner.broadcast_view(&code);
                let sync = inner.parties[&code].sync(None);
                inner.send(account, &sync);
            }
            ToServer::Join { code: wanted } => {
                let wanted = wanted.trim().to_uppercase();
                let Some(p) = inner.parties.get(&wanted) else { return error(&inner, "No party with that code (it may have ended)") };
                if p.banned.contains(&account) {
                    return error(&inner, "The host removed you from this party");
                }
                if p.members.iter().any(|m| m.0 == account) {
                    inner.broadcast_view(&wanted);
                    let sync = inner.parties[&wanted].sync(None);
                    return inner.send(account, &sync);
                }
                if p.members.len() >= MAX_MEMBERS {
                    return error(&inner, "That party is full");
                }
                inner.leave(account, "you joined another party");
                let p = inner.parties.get_mut(&wanted).unwrap();
                p.members.push((account, me.to_owned(), false));
                inner.in_party.insert(account, wanted.clone());
                inner.notice(&wanted, format!("{me} joined"));
                inner.broadcast_view(&wanted);
                let sync = inner.parties[&wanted].sync(None);
                inner.send(account, &sync);
            }
            ToServer::Leave => inner.leave(account, "you left the party"),
            ToServer::Invite { username } => {
                let Some(code) = code else { return error(&inner, "Start or join a party first") };
                drop(inner);
                let Some(to) = friend(&username) else {
                    return self.send(account, &ToClient::Error { message: format!("{username} isn't your friend") });
                };
                let inner = self.inner.lock().unwrap();
                let title = inner.parties[&code].title.clone();
                inner.send(to, &ToClient::Invited { from: me.to_owned(), code, title });
            }
            ToServer::Control { playing, position } => {
                let Some(code) = code else { return };
                let p = inner.parties.get_mut(&code).unwrap();
                if p.host != account && !p.open_control {
                    return error(&inner, "Only the host controls playback in this party");
                }
                let text = if playing != p.playing {
                    format!("{me} {}", if playing { "resumed" } else { "paused" })
                } else {
                    format!("{me} skipped to {}", clock(position))
                };
                p.playing = playing;
                p.position = position.max(0.);
                p.at = Instant::now();
                let sync = p.sync(Some(me.to_owned()));
                inner.broadcast(&code, &sync);
                inner.notice(&code, text);
            }
            ToServer::Heartbeat { position } => {
                let Some(code) = code else { return };
                let p = inner.parties.get_mut(&code).unwrap();
                // Only the host's player is the reference, and only while it's actually moving.
                if p.host != account || !p.effective_playing() {
                    return;
                }
                p.position = position.max(0.);
                p.at = Instant::now();
                let sync = p.sync(None);
                inner.broadcast(&code, &sync);
            }
            ToServer::Buffering { buffering } => {
                let Some(code) = code else { return };
                let p = inner.parties.get_mut(&code).unwrap();
                let was_waiting = p.waiting();
                p.rebase();
                if let Some(m) = p.members.iter_mut().find(|m| m.0 == account) {
                    m.2 = buffering;
                }
                let (now_waiting, playing) = (p.waiting(), p.playing);
                if was_waiting != now_waiting {
                    let sync = p.sync(None);
                    inner.broadcast(&code, &sync);
                    if now_waiting && playing {
                        inner.notice(&code, format!("Waiting for {me}…"));
                    }
                }
                inner.broadcast_view(&code);
            }
            ToServer::Chat { text } => {
                let Some(code) = code else { return };
                let text: String = text.trim().chars().take(MAX_CHAT).collect();
                if !text.is_empty() {
                    inner.broadcast(&code, &ToClient::Chat { from: me.to_owned(), text });
                }
            }
            ToServer::OpenControl { open } => {
                let Some(code) = code else { return };
                let p = inner.parties.get_mut(&code).unwrap();
                if p.host != account {
                    return error(&inner, "Only the host can change that");
                }
                p.open_control = open;
                inner.notice(&code, if open { "Everyone can control playback now".into() } else { "Only the host controls playback now".into() });
                inner.broadcast_view(&code);
            }
            ToServer::Kick { username } => {
                let Some(code) = code else { return };
                let p = inner.parties.get_mut(&code).unwrap();
                if p.host != account {
                    return error(&inner, "Only the host can remove people");
                }
                let Some(target) = p.members.iter().find(|m| m.1.eq_ignore_ascii_case(&username) && m.0 != account).map(|m| m.0) else { return };
                p.banned.insert(target);
                inner.leave(target, "the host removed you from the party");
            }
            ToServer::MakeHost { username } => {
                let Some(code) = code else { return };
                let p = inner.parties.get_mut(&code).unwrap();
                if p.host != account {
                    return error(&inner, "Only the host can hand that over");
                }
                let Some((id, name, _)) = p.members.iter().find(|m| m.1.eq_ignore_ascii_case(&username)).cloned() else { return };
                p.host = id;
                inner.notice(&code, format!("{name} is now the host"));
                inner.broadcast_view(&code);
            }
        }
    }
}

/// Six characters without look-alikes (no 0/O, 1/I).
fn party_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut b = [0u8; 6];
    rand::fill(&mut b);
    b.iter().map(|x| ALPHABET[*x as usize % ALPHABET.len()] as char).collect()
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// One app's connection: registers it, relays its messages, and cleans up when it closes.
pub async fn run(s: AppState, account: i64, username: String, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = unbounded_channel::<ToClient>();
    let id = {
        let mut inner = s.hub.inner.lock().unwrap();
        inner.next_conn += 1;
        let id = inner.next_conn;
        inner.conns.entry(account).or_default().push((id, tx));
        id
    };
    let friends = || crate::social::friend_ids(&s, account).unwrap_or_default();
    s.hub.announce(&friends(), &username, account);

    let out = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let Ok(text) = serde_json::to_string(&msg) else { continue };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(frame)) = stream.next().await {
        let Message::Text(text) = frame else { continue };
        let Ok(msg) = serde_json::from_str::<ToServer>(&text) else { continue };
        if let ToServer::Presence { watching, hidden } = &msg {
            s.hub.inner.lock().unwrap().presence.insert(account, Presence { watching: watching.clone(), hidden: *hidden });
            s.hub.announce(&friends(), &username, account);
            continue;
        }
        s.hub.handle(
            account,
            &username,
            msg,
            || crate::social::can_host(&s, account).unwrap_or(false),
            |name| crate::social::friend_by_name(&s, account, name).ok().flatten(),
        );
    }

    // Closed: the last connection of an account takes it out of its party and offline.
    let last = {
        let mut inner = s.hub.inner.lock().unwrap();
        let conns = inner.conns.entry(account).or_default();
        conns.retain(|(c, _)| *c != id);
        let last = conns.is_empty();
        if last {
            inner.conns.remove(&account);
            inner.leave(account, "disconnected");
            inner.presence.remove(&account);
        }
        last
    };
    if last {
        s.hub.announce(&friends(), &username, account);
    }
    out.abort();
}
