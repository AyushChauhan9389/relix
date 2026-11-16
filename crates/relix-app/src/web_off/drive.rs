//! The web build has no Google Drive (its sign-in waits on a local port): off.

use crate::Ctx;
pub use crate::drive_links::{file_id, party_url};

const OFF: &str = "Google Drive isn't available in the web version yet";

/// Stops a running relay (none on the web).
pub struct Relay;

impl Relay {
    pub fn abort(&self) {}
}

pub fn enabled() -> bool {
    false
}

pub async fn email(_: &Ctx) -> Result<String, String> {
    Err(OFF.into())
}

pub fn connected_email(_: &Ctx) -> Option<String> {
    None
}

pub async fn disconnect(_: &Ctx) {}

pub async fn check_hostable(_: &Ctx, _: &str) -> Result<(), String> {
    Err(OFF.into())
}

pub async fn share(_: &Ctx, _: &str, _: &str) -> Result<(), String> {
    Err(OFF.into())
}

pub async fn readable(_: &Ctx, _: &str) -> bool {
    false
}

pub async fn trash(_: &Ctx, _: &str) -> Result<(), String> {
    Err(OFF.into())
}

pub async fn relay(_: &Ctx, _: &str) -> Result<(String, Relay), String> {
    Err(OFF.into())
}
