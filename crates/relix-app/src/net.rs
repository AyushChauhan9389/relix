//! What differs between the desktop/Android build and the web build (wasm32-unknown-emscripten):
//! HTTP, timers and sets of concurrent tasks. Desktop uses reqwest and tokio; the web build asks
//! the browser through the JavaScript bridge (web/relix-bridge.js).

use std::future::Future;
use std::time::Duration;

/// A finished HTTP request: status and body.
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_slice(&self.body).map_err(|e| format!("unexpected reply: {e}"))
    }
}

/// The one HTTP client. On desktop it's reqwest's (and derefs to it, for the desktop-only
/// streaming uses); [`Client::request`] works everywhere.
#[derive(Clone)]
pub struct Client {
    #[cfg(not(target_os = "emscripten"))]
    inner: reqwest::Client,
}

#[cfg(not(target_os = "emscripten"))]
impl std::ops::Deref for Client {
    type Target = reqwest::Client;
    fn deref(&self) -> &reqwest::Client {
        &self.inner
    }
}

impl Client {
    #[cfg(not(target_os = "emscripten"))]
    pub fn new(inner: reqwest::Client) -> Self {
        Self { inner }
    }

    #[cfg(target_os = "emscripten")]
    pub fn new() -> Self {
        Self {}
    }

    /// Sends a request and reads the whole reply. Errors never carry the URL (addon URLs can hold
    /// debrid keys, and errors end up in logs).
    pub async fn request(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, timeout: Duration) -> Result<Response, String> {
        #[cfg(not(target_os = "emscripten"))]
        {
            let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
            let mut req = self.inner.request(method, url).timeout(timeout);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            if let Some(body) = body {
                req = req.body(body);
            }
            let res = req.send().await.map_err(|e| e.without_url().to_string())?;
            let status = res.status().as_u16();
            let body = res.bytes().await.map_err(|e| e.without_url().to_string())?.to_vec();
            Ok(Response { status, body })
        }
        #[cfg(target_os = "emscripten")]
        {
            crate::web::fetch(method, url, headers, body, timeout).await
        }
    }

    /// GET returning JSON, failing on an error status.
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str, headers: &[(&str, &str)]) -> Result<T, String> {
        let res = self.request("GET", url, headers, None, Duration::from_secs(20)).await?;
        if !res.ok() {
            return Err(format!("HTTP {}", res.status));
        }
        res.json()
    }
}

/// Waits `d` without blocking.
pub async fn sleep(d: Duration) {
    #[cfg(not(target_os = "emscripten"))]
    tokio::time::sleep(d).await;
    #[cfg(target_os = "emscripten")]
    crate::web::sleep(d).await;
}

/// Runs `f` off the UI thread on desktop; the web has one thread, so it runs there.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ()> {
    #[cfg(not(target_os = "emscripten"))]
    return tokio::task::spawn_blocking(f).await.map_err(drop);
    #[cfg(target_os = "emscripten")]
    Ok(f())
}

/// `fut`'s result, or `Err(())` if `d` passes first.
pub async fn timeout<T>(d: Duration, fut: impl Future<Output = T>) -> Result<T, ()> {
    use futures_util::future::{Either, select};
    let (fut, wait) = (std::pin::pin!(fut), std::pin::pin!(sleep(d)));
    match select(fut, wait).await {
        Either::Left((v, _)) => Ok(v),
        Either::Right(_) => Err(()),
    }
}

/// Concurrent tasks whose results come back as they finish: tokio's JoinSet on desktop, a set
/// polled on the page's one thread on the web.
#[cfg(not(target_os = "emscripten"))]
pub use tokio::task::JoinSet as Tasks;

#[cfg(target_os = "emscripten")]
pub struct Tasks<T> {
    set: futures_util::stream::FuturesUnordered<std::pin::Pin<Box<dyn Future<Output = T>>>>,
}

/// A task that didn't finish (never happens on the web; it matches JoinSet's error slot).
#[cfg(target_os = "emscripten")]
#[derive(Debug)]
pub struct TaskFailed;

#[cfg(target_os = "emscripten")]
impl<T> Default for Tasks<T> {
    fn default() -> Self {
        Self { set: Default::default() }
    }
}

#[cfg(target_os = "emscripten")]
impl<T: 'static> Tasks<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&mut self, fut: impl Future<Output = T> + 'static) {
        self.set.push(Box::pin(fut));
    }

    pub async fn join_next(&mut self) -> Option<Result<T, TaskFailed>> {
        use futures_util::StreamExt;
        self.set.next().await.map(Ok)
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }
}
