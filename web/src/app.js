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
      <video autoplay playsinline></video>
      <div class="spinner"></div>
      <div class="bar"><button class="close" aria-label="Close">‹</button><span class="title"></span><span class="status"></span></div>
      <div class="controls">
        <div class="seek"><div class="buffered"></div><div class="played"></div><input type="range" min="0" max="1000" value="0" aria-label="Seek"></div>
        <div class="row">
          <button class="play" aria-label="Play or pause"></button>
          <button class="back" aria-label="Back 10 seconds">${ICONS.back}</button>
          <button class="fwd" aria-label="Forward 10 seconds">${ICONS.fwd}</button>
          <button class="mute" aria-label="Mute"></button>
          <input class="volume" type="range" min="0" max="1" step="0.05" value="1" aria-label="Volume">
          <span class="time"></span>
          <button class="full" aria-label="Fullscreen">${ICONS.full}</button>
        </div>
      </div>`;
    box.querySelector('.title').textContent = title;
    document.body.append(box);
    const video = box.querySelector('video');
    active = video;
    const status = box.querySelector('.status');
    wireControls(box, video);
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
      else box.relixKey?.(e);
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
      if (event === 'direct') status.textContent = '';
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

const ICONS = {
  play: '<svg viewBox="0 0 24 24"><path d="M7 4l13 8-13 8z"/></svg>',
  pause: '<svg viewBox="0 0 24 24"><path d="M6 4h4v16H6zM14 4h4v16h-4z"/></svg>',
  back: '<svg viewBox="0 0 24 24"><path d="M12 5V1L7 6l5 5V7a6 6 0 1 1-6 6H4a8 8 0 1 0 8-8z"/><text x="12" y="16" font-size="7" text-anchor="middle">10</text></svg>',
  fwd: '<svg viewBox="0 0 24 24"><path d="M12 5V1l5 5-5 5V7a6 6 0 1 0 6 6h2a8 8 0 1 1-8-8z"/><text x="12" y="16" font-size="7" text-anchor="middle">10</text></svg>',
  sound: '<svg viewBox="0 0 24 24"><path d="M4 9h4l5-4v14l-5-4H4zM16 8a5 5 0 0 1 0 8M18.5 5.5a8.5 8.5 0 0 1 0 13" fill="none" stroke="currentColor" stroke-width="2"/></svg>',
  muted: '<svg viewBox="0 0 24 24"><path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M16 9l5 6M21 9l-5 6" stroke="currentColor" stroke-width="2"/></svg>',
  full: '<svg viewBox="0 0 24 24"><path d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5" fill="none" stroke="currentColor" stroke-width="2"/></svg>',
};

const clock = (s) => {
  if (!Number.isFinite(s) || s < 0) s = 0;
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = Math.floor(s % 60);
  return h ? `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}` : `${m}:${String(sec).padStart(2, '0')}`;
};

/**
 * Relix's player controls in the page (the canvas can't draw over the video): a spinner while it
 * waits, the seek bar, play/pause, ±10 s, volume, time and fullscreen; hidden after 3 s still; the
 * desktop player's keys (Space, arrows, F, M).
 */
function wireControls(box, video) {
  const $ = (s) => box.querySelector(s);
  const playBtn = $('.play'), muteBtn = $('.mute'), seek = $('.seek input'), vol = $('.volume');
  const toggle = () => (video.paused ? video.play().catch(() => {}) : video.pause());
  const duration = () => (Number.isFinite(video.duration) ? video.duration : 0);
  const paint = () => {
    playBtn.innerHTML = video.paused ? ICONS.play : ICONS.pause;
    muteBtn.innerHTML = video.muted || video.volume === 0 ? ICONS.muted : ICONS.sound;
    const d = duration();
    if (!seeking) seek.value = d ? (video.currentTime / d) * 1000 : 0;
    $('.played').style.width = `${d ? (video.currentTime / d) * 100 : 0}%`;
    const b = video.buffered;
    const end = b.length ? b.end(b.length - 1) : 0;
    $('.buffered').style.width = `${d ? (end / d) * 100 : 0}%`;
    $('.time').textContent = `${clock(video.currentTime)} / ${clock(d)}`;
    box.classList.toggle('waiting', !video.paused && video.readyState < 3);
  };
  let seeking = false;
  seek.addEventListener('input', () => {
    seeking = true;
    $('.time').textContent = `${clock((seek.value / 1000) * duration())} / ${clock(duration())}`;
  });
  seek.addEventListener('change', () => {
    video.currentTime = (seek.value / 1000) * duration();
    seeking = false;
  });
  vol.addEventListener('input', () => {
    video.volume = Number(vol.value);
    video.muted = video.volume === 0;
  });
  playBtn.addEventListener('click', toggle);
  video.addEventListener('click', toggle);
  video.addEventListener('dblclick', () => $('.full').click());
  $('.back').addEventListener('click', () => (video.currentTime -= 10));
  $('.fwd').addEventListener('click', () => (video.currentTime += 10));
  muteBtn.addEventListener('click', () => (video.muted = !video.muted));
  $('.full').addEventListener('click', () => (document.fullscreenElement ? document.exitFullscreen() : box.requestFullscreen().catch(() => {})));
  for (const e of ['play', 'pause', 'timeupdate', 'progress', 'waiting', 'playing', 'canplay', 'seeking', 'seeked', 'volumechange', 'durationchange']) video.addEventListener(e, paint);
  paint();
  // Controls fade out after 3 s without the mouse moving, except while paused.
  let idle = null;
  const wake = () => {
    box.classList.remove('idle');
    clearTimeout(idle);
    idle = setTimeout(() => !video.paused && box.classList.add('idle'), 3000);
  };
  box.addEventListener('pointermove', wake);
  video.addEventListener('pause', wake);
  wake();
  box.relixKey = (e) => {
    const keys = {
      ' ': toggle,
      k: toggle,
      ArrowLeft: () => (video.currentTime -= 10),
      ArrowRight: () => (video.currentTime += 10),
      ArrowUp: () => (video.volume = Math.min(1, video.volume + 0.05)),
      ArrowDown: () => (video.volume = Math.max(0, video.volume - 0.05)),
      f: () => $('.full').click(),
      m: () => (video.muted = !video.muted),
    };
    const run = keys[e.key.length === 1 ? e.key.toLowerCase() : e.key];
    if (!run) return;
    e.preventDefault();
    run();
    wake();
  };
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
