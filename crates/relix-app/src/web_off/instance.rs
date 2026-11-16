//! A web page is its own instance: nothing to bring forward.

use std::sync::Mutex;

pub static SHOW: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<()>>> = Mutex::new(None);
