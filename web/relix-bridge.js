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

  // A WebSocket signed in with `token` (as the subprotocol `relix, <token>`: browsers can't set
  // headers). Events go to relix_ws_event(id, kind, text): 0 open, 1 message, 2 closed.
  relix_ws_open__deps: ['malloc', 'free'],
  relix_ws_open: function (id, urlPtr, tokenPtr) {
    const sockets = (globalThis.relixSockets ??= new Map());
    const event = (kind, text) => {
      const len = lengthBytesUTF8(text) + 1;
      const ptr = _malloc(len);
      stringToUTF8(text, ptr, len);
      _relix_ws_event(id, kind, ptr);
      _free(ptr);
    };
    let ws;
    try {
      ws = new WebSocket(UTF8ToString(urlPtr), ['relix', UTF8ToString(tokenPtr)]);
    } catch (e) {
      return setTimeout(() => event(2, String(e)), 0);
    }
    sockets.set(id, ws);
    ws.onopen = () => event(0, '');
    ws.onmessage = (m) => typeof m.data === 'string' && event(1, m.data);
    ws.onclose = (c) => {
      sockets.delete(id);
      event(2, c.reason || `closed (${c.code})`);
    };
  },

  relix_ws_send: function (id, textPtr) {
    const ws = globalThis.relixSockets?.get(id);
    if (ws && ws.readyState === 1) ws.send(UTF8ToString(textPtr));
  },

  relix_ws_close: function (id) {
    globalThis.relixSockets?.get(id)?.close();
  },

  // Fullscreen for the whole page (Relix's fullscreen button, F11 in the app).
  relix_toggle_fullscreen: function () {
    if (document.fullscreenElement) document.exitFullscreen();
    else document.documentElement.requestFullscreen().catch(() => {});
  },

  // The page player under mpv's names (mpv.rs's web stand-in): the player screen and Watch
  // Together control it through these.
  relix_player_set: function (namePtr, valuePtr) {
    globalThis.relixPlayer?.control?.set(UTF8ToString(namePtr), UTF8ToString(valuePtr));
  },

  relix_player_get: function (namePtr, buf, len) {
    const value = globalThis.relixPlayer?.control?.get(UTF8ToString(namePtr));
    if (value == null) return -1;
    return stringToUTF8(String(value), buf, len);
  },

  relix_player_command: function (namePtr, argsPtr) {
    const args = UTF8ToString(argsPtr);
    globalThis.relixPlayer?.control?.command(UTF8ToString(namePtr), args ? args.split('\n') : []);
  },

  relix_set_timeout: function (id, ms) {
    setTimeout(() => _relix_timer_done(id), ms);
  },

  // The page's player (web/src/app.js) over the app; resolves when it's closed or ends.
  // `info`: { title, episode, hasNext, accent } (web.rs play). Answers where it stopped and
  // whether Next was pressed.
  relix_play: function (id, urlPtr, infoPtr, start) {
    const url = UTF8ToString(urlPtr);
    const info = JSON.parse(UTF8ToString(infoPtr));
    const open = globalThis.relixPlayer && globalThis.relixPlayer.open;
    const result = open ? open(url, info, start) : Promise.resolve({ position: start, duration: 0 });
    result.then(
      ({ position, duration, next }) => _relix_play_done(id, position || 0, duration || 0, next ? 1 : 0),
      () => _relix_play_done(id, start, 0, 0),
    );
  },
});
