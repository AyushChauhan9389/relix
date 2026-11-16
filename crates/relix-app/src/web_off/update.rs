//! A web page updates by reloading: no self-update.

use crate::Ctx;

pub fn enabled() -> bool {
    false
}

pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

pub fn channel(_: &Ctx) -> String {
    "stable".into()
}

pub fn set_channel(_: &Ctx, _: &str) {}

#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    pub version: String,
}

pub async fn check(_: Ctx) {}

pub async fn check_now(_: Ctx) -> String {
    "The web version is always the latest.".into()
}

pub fn apply_in_background(_: Ready) {}

pub fn apply(_: &Ready) -> std::io::Result<()> {
    Ok(())
}
