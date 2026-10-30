//! Account and sync wire types shared by the app and relix-server (ADR 0004, 0005).
//! Like Stremio's, the server checks the password and stores records readable (ADR 0005).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signup {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupResponse {
    /// Present when the server doesn't verify emails; otherwise call `/verify` with the emailed code.
    pub session: Option<Session>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verify {
    pub email: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Login {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
}

/// A record as the client sends it; the server assigns its sequence number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushRecord {
    pub kind: String,
    pub key: String,
    /// JSON text.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Push {
    pub records: Vec<PushRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResponse {
    /// The sequence number given to each pushed record, in order.
    pub seqs: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncedRecord {
    pub kind: String,
    pub key: String,
    pub value: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    /// Records changed after the requested sequence number, oldest first.
    pub records: Vec<SyncedRecord>,
}

/// An addon the server admin publishes to every signed-in account; users can switch them off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminAddon {
    pub id: u64,
    #[serde(flatten)]
    pub addon: crate::addon::Addon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddedAdminAddon {
    pub id: u64,
}
