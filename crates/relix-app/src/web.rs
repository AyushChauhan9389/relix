//! The web build's way to the browser: HTTP (fetch), timers and the video player, through the
//! JavaScript bridge (web/relix-bridge.js, linked with `--js-library`). Each call gets an id;
//! the bridge answers by calling back `relix_*_done(id, ...)`, which completes the waiting future.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_char};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::net::Response;

unsafe extern "C" {
    fn relix_fetch(id: u32, method: *const c_char, url: *const c_char, headers_json: *const c_char, body: *const u8, body_len: usize, timeout_ms: u32);
    fn relix_set_timeout(id: u32, ms: u32);
    fn relix_play(id: u32, url: *const c_char, title: *const c_char, start: f64);
}

thread_local! {
    static NEXT: RefCell<u32> = const { RefCell::new(1) };
    static FETCHES: RefCell<HashMap<u32, oneshot::Sender<Result<Response, String>>>> = RefCell::default();
    static TIMERS: RefCell<HashMap<u32, oneshot::Sender<()>>> = RefCell::default();
    static PLAYS: RefCell<HashMap<u32, oneshot::Sender<(f64, f64)>>> = RefCell::default();
}

fn next_id() -> u32 {
    NEXT.with(|n| {
        let mut n = n.borrow_mut();
        *n = n.wrapping_add(1).max(1);
        *n
    })
}

fn c(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

pub async fn fetch(method: &str, url: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, timeout: Duration) -> Result<Response, String> {
    let (tx, rx) = oneshot::channel();
    let id = next_id();
    FETCHES.with(|f| f.borrow_mut().insert(id, tx));
    let headers: HashMap<&str, &str> = headers.iter().copied().collect();
    let headers = serde_json::to_string(&headers).unwrap_or_default();
    let body = body.unwrap_or_default();
    let (m, u, h) = (c(method), c(url), c(&headers));
    unsafe { relix_fetch(id, m.as_ptr(), u.as_ptr(), h.as_ptr(), body.as_ptr(), body.len(), timeout.as_millis() as u32) };
    rx.await.unwrap_or_else(|_| Err("request dropped".into()))
}

/// The bridge's answer to [`fetch`]: `status` 0 is a network error, with `data` its message.
#[unsafe(no_mangle)]
pub extern "C" fn relix_fetch_done(id: u32, status: u32, data: *const u8, len: usize) {
    let bytes = if data.is_null() { vec![] } else { unsafe { std::slice::from_raw_parts(data, len) }.to_vec() };
    let result = if status == 0 {
        Err(format!("Can't reach it: {}", String::from_utf8_lossy(&bytes)))
    } else {
        Ok(Response { status: status as u16, body: bytes })
    };
    if let Some(tx) = FETCHES.with(|f| f.borrow_mut().remove(&id)) {
        let _ = tx.send(result);
    }
}

pub async fn sleep(d: Duration) {
    let (tx, rx) = oneshot::channel();
    let id = next_id();
    TIMERS.with(|t| t.borrow_mut().insert(id, tx));
    unsafe { relix_set_timeout(id, d.as_millis().min(u32::MAX as u128) as u32) };
    let _ = rx.await;
}

#[unsafe(no_mangle)]
pub extern "C" fn relix_timer_done(id: u32) {
    if let Some(tx) = TIMERS.with(|t| t.borrow_mut().remove(&id)) {
        let _ = tx.send(());
    }
}

/// Plays `url` in the page's video player (over the app) from `start` seconds; resolves when it's
/// closed or ends, with the last (position, duration).
pub async fn play(url: &str, title: &str, start: f64) -> (f64, f64) {
    let (tx, rx) = oneshot::channel();
    let id = next_id();
    PLAYS.with(|p| p.borrow_mut().insert(id, tx));
    let (u, t) = (c(url), c(title));
    unsafe { relix_play(id, u.as_ptr(), t.as_ptr(), start) };
    rx.await.unwrap_or((start, 0.))
}

#[unsafe(no_mangle)]
pub extern "C" fn relix_play_done(id: u32, position: f64, duration: f64) {
    if let Some(tx) = PLAYS.with(|p| p.borrow_mut().remove(&id)) {
        let _ = tx.send((position, duration));
    }
}
