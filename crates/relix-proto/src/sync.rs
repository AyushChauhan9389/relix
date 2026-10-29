//! Account and sync wire types shared by the app and relix-server (ADR 0004, 0005).

use serde::{Deserialize, Serialize};

/// Signup sends only what the server may know: the login hash and wrapped keys, never the password.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signup {
    pub email: String,
    pub salt: String,
    pub login_hash: String,
    /// The data key wrapped by the password-derived key.
    pub wrapped_key: String,
    /// The data key wrapped by the recovery key.
    pub recovery_wrapped_key: String,
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
pub struct Prelogin {
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreloginResponse {
    pub salt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Login {
    pub email: String,
    pub login_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub wrapped_key: String,
    pub recovery_wrapped_key: String,
}

/// A record as the client sends it; the server assigns its sequence number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushRecord {
    pub kind: String,
    pub key: String,
    /// JSON text, or ciphertext for encrypted kinds. Opaque to the server.
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

/// Record kinds whose values are encrypted with the data key before they leave the device.
pub const ENCRYPTED_KINDS: [&str; 1] = ["addons"];
