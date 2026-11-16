// The browser side of web.rs (linked into the web build with --js-library): HTTP through fetch,
// timers, and the video player. Each call carries an id; the answer goes back to Rust through
// relix_*_done(id, ...).
addToLibrary({
  relix_fetch__deps: ['malloc', 'free'],
  relix_fetch: function (id, methodPtr, urlPtr, headersPtr, bodyPtr, bodyLen, timeoutMs) {
    const method = UTF8ToString(methodPtr);
    const url = UTF8ToString(urlPtr);
    const headers = JSON.parse(UTF8ToString(headersPtr) || '{}');
    const body = bodyLen > 0 ? HEAPU8.slice(bodyPtr, bodyPtr + bodyLen) : undefined;
    const done = (status, bytes) => {
      const ptr = bytes.length ? _malloc(bytes.length) : 0;
      if (ptr) HEAPU8.set(bytes, ptr);
      _relix_fetch_done(id, status, ptr, bytes.length);
      if (ptr) _free(ptr);
    };
    const abort = new AbortController();
    const timer = timeoutMs ? setTimeout(() => abort.abort(), timeoutMs) : null;
    fetch(url, { method, headers, body, signal: abort.signal })
      .then(async (res) => done(res.status, new Uint8Array(await res.arrayBuffer())))
      // Status 0: a network error (or the host doesn't allow browser requests: CORS).
      .catch((e) => done(0, new TextEncoder().encode(String(e && e.message ? e.message : e))))
      .finally(() => timer && clearTimeout(timer));
  },

  relix_set_timeout: function (id, ms) {
    setTimeout(() => _relix_timer_done(id), ms);
  },

  // The page's player (web/src/app.js) over the app; resolves when it's closed or ends.
  relix_play: function (id, urlPtr, titlePtr, start) {
    const url = UTF8ToString(urlPtr);
    const title = UTF8ToString(titlePtr);
    const open = globalThis.relixPlayer && globalThis.relixPlayer.open;
    const result = open ? open(url, title, start) : Promise.resolve({ position: start, duration: 0 });
    result.then(
      ({ position, duration }) => _relix_play_done(id, position || 0, duration || 0),
      () => _relix_play_done(id, start, 0),
    );
  },
});
