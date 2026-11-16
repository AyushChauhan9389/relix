//! No Discord status from a web page: off.

use std::sync::mpsc::Sender;

pub struct Watching {
    pub title: String,
    pub episode: String,
    pub image: Option<String>,
}

pub fn start() -> Option<Sender<Option<Watching>>> {
    None
}
