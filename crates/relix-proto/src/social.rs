//! Usernames, friends and Watch Together (ADR 0010): the REST bodies and the WebSocket messages
//! the app and the server exchange.

use serde::{Deserialize, Serialize};

/// The signed-in account, as `GET /me` returns it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub email: String,
    /// `None` until the account picks one.
    pub username: Option<String>,
    /// Watch Together preview: this account may host parties.
    pub can_host: bool,
}

/// `PUT /me/username`, and the friend actions that name someone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Username {
    pub username: String,
}

/// 3-20 characters: ASCII letters, digits and `_`. Unique regardless of case.
pub fn valid_username(name: &str) -> bool {
    (3..=20).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `GET /friends`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Friends {
    pub friends: Vec<Friend>,
    /// Requests others sent this account.
    pub incoming: Vec<String>,
    /// Requests this account sent.
    pub outgoing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Friend {
    pub username: String,
    pub online: bool,
    /// What they're watching, when they're online and share it.
    pub watching: Option<String>,
}

/// What a party watches: enough of the title to show its poster and name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartyTitle {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub poster: Option<String>,
    /// e.g. "S1:E2 Cat's in the Bag..."; empty for movies.
    #[serde(default)]
    pub episode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub username: String,
    pub buffering: bool,
}

/// A party as its members see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartyView {
    /// Six characters; the invite link is `relix://party/<code>`.
    pub code: String,
    pub title: PartyTitle,
    /// The video everyone plays (a Drive share link is already converted to a direct one).
    pub url: String,
    pub host: String,
    /// In join order; the first one after the host takes over when the host leaves.
    pub members: Vec<Member>,
    /// Everyone may play, pause and seek, not only the host.
    pub open_control: bool,
}

/// App -> server over `/ws`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToServer {
    /// What this account is watching, for friends; `hidden` shows it as offline.
    Presence { watching: Option<String>, hidden: bool },
    /// Start a party (preview hosts only) and join it as host.
    Create { title: PartyTitle, url: String },
    Join { code: String },
    Leave,
    /// Invite a friend to the party this account is in.
    Invite { username: String },
    /// Play, pause or seek: from the host, or anyone when control is open.
    Control { playing: bool, position: f64 },
    /// The host's player position every few seconds, so drift gets corrected.
    Heartbeat { position: f64 },
    /// This member's player is waiting for data: the party waits with it.
    Buffering { buffering: bool },
    Chat { text: String },
    /// Host: let everyone control playback.
    OpenControl { open: bool },
    /// Host: remove a member (they can't rejoin this party).
    Kick { username: String },
    /// Host: hand the host role over.
    MakeHost { username: String },
}

/// Server -> app over `/ws`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToClient {
    /// Requests or friendships changed: fetch `GET /friends` again.
    FriendsChanged,
    /// A friend came online, went offline or started watching something.
    Presence { username: String, online: bool, watching: Option<String> },
    /// A friend invited this account to a party.
    Invited { from: String, code: String, title: PartyTitle },
    /// This account is in a party: its current state (after joining and after every change).
    Party(PartyView),
    /// Where playback should be: `position` seconds, playing or paused. `by` names who changed it
    /// (absent for heartbeats and buffering waits).
    Sync { playing: bool, position: f64, by: Option<String> },
    Chat { from: String, text: String },
    /// A line for the chat panel ("ayush paused", "waiting for sam").
    Notice { text: String },
    /// This account isn't in the party any more: left, removed, or the party ended.
    Left { reason: String },
    Error { message: String },
}

/// A Google Drive share link as a direct, seekable download link; other links pass through.
/// `drive.google.com/file/d/<id>/view`, `/open?id=<id>` and `uc?id=<id>` are recognised.
pub fn direct_video_url(link: &str) -> String {
    let link = link.trim();
    let id = link
        .split("/file/d/")
        .nth(1)
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .or_else(|| {
            link.contains("drive.google.com").then(|| {
                link.split(['?', '&']).find_map(|part| part.strip_prefix("id=")).unwrap_or("")
            })
        })
        .filter(|id| !id.is_empty());
    match id {
        // `confirm=t` skips Drive's "can't scan this file for viruses" page on big files.
        Some(id) => format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"),
        None => link.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(valid_username("ayush_42"));
        assert!(!valid_username("ab"), "too short");
        assert!(!valid_username("a".repeat(21).as_str()), "too long");
        assert!(!valid_username("bad name"));
        assert!(!valid_username("émile"));
    }

    #[test]
    fn drive_links_become_direct_downloads() {
        let direct = "https://drive.usercontent.google.com/download?id=1AbC_d-9&export=download&confirm=t";
        assert_eq!(direct_video_url("https://drive.google.com/file/d/1AbC_d-9/view?usp=sharing"), direct);
        assert_eq!(direct_video_url("https://drive.google.com/open?id=1AbC_d-9"), direct);
        assert_eq!(direct_video_url("https://drive.google.com/uc?export=download&id=1AbC_d-9"), direct);
        assert_eq!(direct_video_url(" https://example.com/movie.mkv "), "https://example.com/movie.mkv");
    }

    #[test]
    fn messages_are_tagged_json() {
        let m = serde_json::to_string(&ToServer::Control { playing: true, position: 12.5 }).unwrap();
        assert_eq!(m, r#"{"type":"control","playing":true,"position":12.5}"#);
        let back: ToServer = serde_json::from_str(&m).unwrap();
        assert_eq!(back, ToServer::Control { playing: true, position: 12.5 });
    }
}
