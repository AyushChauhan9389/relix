//! Signed app updates: the server's `latest.json`, version comparison and ed25519 signatures.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// What `GET /update/latest` returns: the newest build and the signature of its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    /// File name under `/update/files/`.
    pub file: String,
    /// Base64 ed25519 signature of `signed_message(version, file bytes)`.
    pub signature: String,
}

/// Whether `candidate` is a later version than `current`, comparing dotted numbers.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| v.split('.').map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().ok();
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

/// What a release signature covers: the version and the file, so an old build can't be relabelled as new.
pub fn signed_message(version: &str, file: &[u8]) -> Vec<u8> {
    [version.as_bytes(), b"\n", file].concat()
}

/// Signs `data` with a 32-byte secret key; returns the base64 public key and signature.
pub fn sign(secret: &[u8; 32], data: &[u8]) -> (String, String) {
    let key = SigningKey::from_bytes(secret);
    (B64.encode(key.verifying_key().to_bytes()), B64.encode(key.sign(data).to_bytes()))
}

/// Whether `signature` (base64) is `public_key`'s (base64) signature of `data`.
pub fn verify(public_key: &str, data: &[u8], signature: &str) -> bool {
    let key = B64.decode(public_key).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).and_then(|k| VerifyingKey::from_bytes(&k).ok());
    let sig = B64.decode(signature).ok().and_then(|s| Signature::from_slice(&s).ok());
    matches!((key, sig), (Some(k), Some(s)) if k.verify_strict(data, &s).is_ok())
}
