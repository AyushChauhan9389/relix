// The web app's page side: the player Relix (web.rs relix_play) opens over the canvas. It plays
// through player.js (the browser's own <video>, MKV repackaged on the fly) and answers with where
// it stopped, so Relix keeps Continue Watching and moves on to the next episode.
import { play } from './player.js';

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
    const status = box.querySelector('.status');
    let current = null;
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      const position = video.currentTime || start;
      const duration = Number.isFinite(video.duration) ? video.duration : 0;
      current?.stop();
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
    play(video, url, (event, detail) => {
      if (event === 'error') status.textContent = detail.message;
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

globalThis.relixPlayer = { open };
