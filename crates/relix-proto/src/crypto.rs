//! Client-side keys (ADR 0005). The server only ever sees the login hash and wrapped keys.
//!
//! password --Argon2id(salt)--> master key --HKDF--> login hash (sent to the server)
//!                                         \-HKDF--> wrapping key --XChaCha20-Poly1305--> wrapped data key
//! recovery key --HKDF--> wrapping key --> second wrapped copy of the data key
//! data key --XChaCha20-Poly1305--> encrypted records (the addon list)

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, Generate, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;

const NONCE_LEN: usize = 24;

/// Wrong password, wrong key, or tampered data. Deliberately says nothing more.
#[derive(Debug, PartialEq)]
pub struct CryptoError;

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("couldn't decrypt: wrong key or damaged data")
    }
}

impl std::error::Error for CryptoError {}

fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    rand::fill(&mut b);
    b
}

fn hkdf(ikm: &[u8], info: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(None, ikm).expand(info.as_bytes(), &mut out).expect("32 bytes is a valid HKDF length");
    out
}

fn seal(key: &[u8; 32], plaintext: &[u8]) -> String {
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let nonce = XNonce::generate();
    let ct = cipher.encrypt(&nonce, plaintext).expect("encryption with a valid key can't fail");
    B64.encode([nonce.as_slice(), &ct].concat())
}

fn open(key: &[u8; 32], sealed: &str) -> Result<Vec<u8>, CryptoError> {
    let bytes = B64.decode(sealed).map_err(|_| CryptoError)?;
    if bytes.len() < NONCE_LEN {
        return Err(CryptoError);
    }
    let (nonce, ct) = bytes.split_at(NONCE_LEN);
    let nonce = XNonce::try_from(nonce).map_err(|_| CryptoError)?;
    XChaCha20Poly1305::new(&Key::from(*key)).decrypt(&nonce, ct).map_err(|_| CryptoError)
}

fn open_key(key: &[u8; 32], sealed: &str) -> Result<DataKey, CryptoError> {
    let bytes: [u8; 32] = open(key, sealed)?.try_into().map_err(|_| CryptoError)?;
    Ok(DataKey(bytes))
}

/// A fresh per-account salt for the password KDF, stored by the server.
pub fn new_salt() -> String {
    let mut b = [0u8; 16];
    rand::fill(&mut b);
    B64.encode(b)
}

// ponytail: keys aren't zeroized on drop; add `zeroize` if memory scraping enters the threat model.
/// Derived from the password; never leaves the device.
pub struct MasterKey([u8; 32]);

impl MasterKey {
    pub fn derive(password: &str, salt: &str) -> Self {
        let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("valid Argon2 params");
        let mut out = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(password.as_bytes(), salt.as_bytes(), &mut out)
            .expect("salt from new_salt() is long enough");
        Self(out)
    }

    /// What the server stores (hashed again) to check logins; can't unwrap anything.
    pub fn login_hash(&self) -> String {
        B64.encode(hkdf(&self.0, "relix login"))
    }

    pub fn wrap(&self, data: &DataKey) -> String {
        seal(&hkdf(&self.0, "relix wrap"), &data.0)
    }

    pub fn unwrap(&self, wrapped: &str) -> Result<DataKey, CryptoError> {
        open_key(&hkdf(&self.0, "relix wrap"), wrapped)
    }
}

/// The random key that encrypts the addon list.
pub struct DataKey([u8; 32]);

impl DataKey {
    pub fn generate() -> Self {
        Self(random32())
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> String {
        seal(&self.0, plaintext)
    }

    pub fn decrypt(&self, sealed: &str) -> Result<Vec<u8>, CryptoError> {
        open(&self.0, sealed)
    }

    /// For keeping the unlocked key on this device between launches.
    pub fn to_base64(&self) -> String {
        B64.encode(self.0)
    }

    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let bytes: [u8; 32] = B64.decode(s).map_err(|_| CryptoError)?.try_into().map_err(|_| CryptoError)?;
        Ok(Self(bytes))
    }
}

/// Shown once at signup; unwraps the data key if the password is lost.
pub struct Recovery([u8; 32]);

impl Recovery {
    /// The key and how to show it: 64 hex digits in dash-separated groups of 4.
    pub fn generate() -> (Self, String) {
        let bytes = random32();
        let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
        let shown = hex.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-");
        (Self(bytes), shown)
    }

    /// Accepts the key typed with any case, spaces or dashes.
    pub fn parse(typed: &str) -> Result<Self, CryptoError> {
        let hex: String = typed.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
        if hex.len() != 64 {
            return Err(CryptoError);
        }
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| CryptoError)?;
        }
        Ok(Self(bytes))
    }

    pub fn wrap(&self, data: &DataKey) -> String {
        seal(&hkdf(&self.0, "relix recovery"), &data.0)
    }

    pub fn unwrap(&self, wrapped: &str) -> Result<DataKey, CryptoError> {
        open_key(&hkdf(&self.0, "relix recovery"), wrapped)
    }
}
