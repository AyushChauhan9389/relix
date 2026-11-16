// The web app's page side: the player Relix (web.rs relix_play) opens over the canvas. It plays
// through player.js (the browser's own <video>, MKV repackaged on the fly) and answers with where
// it stopped, so Relix keeps Continue Watching and moves on to the next episode.
import { play } from './player.js';

// The open player's <video>, for control() below.
let active = null;

function open(url, title, start) {
  return new Promise((resolve) => {
    const box = document.createElement('div');
    box.id = 'relix-player';
    box.innerHTML = `
      <video controls autoplay playsinline></video>
      <div class="bar"><button class="close" aria-label="Close">‹</button><span class="title"></span><span class="status"></span></div>`;
    box.querySelector('.title').textContent = title;
    document.body.append(box);
    const video = box.querySelector('video');
    active = video;
    const status = box.querySelector('.status');
    let current = null;
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      const position = video.currentTime || start;
      const duration = Number.isFinite(video.duration) ? video.duration : 0;
      current?.stop();
      if (active === video) active = null;
      box.remove();
      document.removeEventListener('keydown', onKey, true);
      document.getElementById('canvas')?.focus();
      resolve({ position, duration });
    };
    // Esc closes, like the desktop player; the canvas doesn't see keys while this is open.
    const onKey = (e) => {
      if (e.key === 'Escape') close();
      e.stopPropagation();
    };
    document.addEventListener('keydown', onKey, true);
    box.querySelector('.close').addEventListener('click', close);
    video.addEventListener('ended', close);
    // A host that blocks the page: what's wrong, and the helper extension that fixes it.
    const needsHelper = (text) => {
      status.textContent = `${text} `;
      const a = document.createElement('a');
      a.href = 'relix-web-helper.zip';
      a.textContent = 'Get the Relix Web Helper';
      status.append(a);
    };
    play(video, url, (event, detail) => {
      if (event === 'error') status.textContent = detail.message;
      if (event === 'no-audio') needsHelper("No sound: this host doesn't let the page convert the audio.");
      if (event === 'direct') video.addEventListener('error', () => needsHelper("This host doesn't let the page play the file."), { once: true });
    })
      .then((c) => {
        current = c;
        if (closed) return c.stop();
        if (start > 1) video.currentTime = start;
        video.focus();
      })
      .catch((e) => {
        status.textContent = String(e.message ?? e);
      });
  });
}

/** mpv's property and command names on the open <video> (Watch Together, the player screen). */
const control = {
  set(name, value) {
    const v = active;
    if (!v) return;
    if (name === 'pause') value === 'true' ? v.pause() : v.play().catch(() => {});
    else if (name === 'speed') v.playbackRate = Number(value);
    else if (name === 'volume') v.volume = Math.min(1, Number(value) / 100);
    else if (name === 'mute') v.muted = value === 'true';
  },
  get(name) {
    const v = active;
    if (!v) return null;
    const duration = Number.isFinite(v.duration) ? v.duration : 0;
    switch (name) {
      case 'time-pos': return v.currentTime;
      case 'duration': return duration;
      case 'pause': return v.paused;
      case 'speed': return v.playbackRate;
      // Everything Watch Together needs at once.
      case 'relix-status': return JSON.stringify({ position: v.currentTime, duration, paused: v.paused, buffering: v.readyState < 3 && !v.paused, seeking: v.seeking, loaded: v.readyState >= 1 });
      default: return null;
    }
  },
  command(name, args) {
    const v = active;
    if (!v || name !== 'seek') return;
    const to = Number(args[0]);
    v.currentTime = (args[1] ?? '').startsWith('absolute') ? to : v.currentTime + to;
  },
};

globalThis.relixPlayer = { open, control };
