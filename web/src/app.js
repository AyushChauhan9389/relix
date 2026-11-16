// The web app's page side: the player Relix (web.rs relix_play) opens over the canvas. It plays
// through player.js (the browser's own <video>, MKV repackaged on the fly) and answers with where
// it stopped, so Relix keeps Continue Watching and moves on to the next episode.
import { play } from './player.js';

// The open player's <video>, for control() below.
let active = null;

// The desktop player's icons (crates/relix-app/assets/icons, Lucide, ISC licence).
const ICONS = {
  back: '<path d="m12 19-7-7 7-7" /> <path d="M19 12H5" />',
  play: '<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z" />',
  pause: '<rect x="14" y="3" width="5" height="18" rx="1" /> <rect x="5" y="3" width="5" height="18" rx="1" />',
  back10: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" /> <path d="M3 3v5h5" />',
  fwd10: '<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" /> <path d="M21 3v5h-5" />',
  vol: '<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z" /> <path d="M16 9a5 5 0 0 1 0 6" /> <path d="M19.364 18.364a9 9 0 0 0 0-12.728" />',
  volLow: '<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z" /> <path d="M16 9a5 5 0 0 1 0 6" />',
  muted: '<path d="M11 4.702a.7.7 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.7.7 0 0 0 11 19.298z" /> <path d="m16.5 14.5 5-5" /> <path d="m16.5 9.5 5 5" />',
  next: '<path d="M21 4v16" /> <path d="M6.029 4.285A2 2 0 0 0 3 6v12a2 2 0 0 0 3.029 1.715l9.997-5.998a2 2 0 0 0 .003-3.432z" />',
  cc: '<rect width="18" height="14" x="3" y="5" rx="2" ry="2" /> <path d="M7 15h4M15 15h2M7 11h2M13 11h4" />',
  speed: '<path d="m12 14 4-4" /> <path d="M3.34 19a10 10 0 1 1 17.32 0" />',
  full: '<path d="M8 3H5a2 2 0 0 0-2 2v3" /> <path d="M21 8V5a2 2 0 0 0-2-2h-3" /> <path d="M3 16v3a2 2 0 0 0 2 2h3" /> <path d="M16 21h3a2 2 0 0 0 2-2v-3" />',
  exitFull: '<path d="M8 3v3a2 2 0 0 1-2 2H3" /> <path d="M21 8h-3a2 2 0 0 1-2-2V3" /> <path d="M3 16h3a2 2 0 0 1 2 2v3" /> <path d="M16 21v-3a2 2 0 0 1 2-2h3" />',
  check: '<path d="M20 6 9 17l-5-5" />',
};

const svg = (name, size, filled) =>
  `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="${filled ? 'white' : 'none'}" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;

/** The desktop player's round icon button: `size` px icon in 8 px padding, an optional "10" badge. */
const button = (cls, name, size, { filled = false, badge = '', label = '' } = {}) =>
  `<button class="icon ${cls}" aria-label="${label}" data-size="${size}" data-filled="${filled}">${svg(name, size, filled)}${badge ? `<b style="font-size:${size * 0.3}px">${badge}</b>` : ''}</button>`;

const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];
/** The volume goes up to 150% on desktop; the page's player stops at 100%. */
const clock = (s) => {
  s = Math.max(0, Math.floor(Number.isFinite(s) ? s : 0));
  const h = Math.floor(s / 3600), m = Math.floor(s / 60) % 60, sec = s % 60;
  return h ? `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}` : `${m}:${String(sec).padStart(2, '0')}`;
};

/**
 * The player Relix opens over the canvas (web.rs relix_play), drawn like the desktop player
 * (player_ui.rs): the top bar with back, title and episode; the timeline with the time left; play,
 * -10 / +10, volume, and next episode, captions, speed and fullscreen; the big play button while
 * paused; a spinner while it waits; menus like the desktop's panel. `info`: { title, episode,
 * hasNext, accent }. Resolves with where it stopped and whether Next was pressed.
 */
function open(url, info, start) {
  return new Promise((resolve) => {
    const box = document.createElement('div');
    box.id = 'relix-player';
    box.style.setProperty('--accent', info.accent || 'rgb(229, 9, 20)');
    box.innerHTML = `
      <video autoplay playsinline></video>
      <div class="spinner"></div>
      <div class="center">${button('big', 'play', 64, { filled: true, label: 'Play' })}</div>
      <div class="top">
        ${button('back', 'back', 30, { label: 'Back' })}
        <div class="titles"><div class="title"></div><div class="episode"></div></div>
        <div class="status"></div>
      </div>
      <div class="menu" hidden></div>
      <div class="bottom">
        <div class="timeline"><div class="slider seek"><div class="track"><div class="fill"></div></div><div class="thumb"></div></div><span class="left"></span></div>
        <div class="row">
          <div class="group">
            ${button('play', 'pause', 30, { filled: true, label: 'Play or pause' })}
            ${button('back10', 'back10', 30, { badge: '10', label: 'Back 10 seconds' })}
            ${button('fwd10', 'fwd10', 30, { badge: '10', label: 'Forward 10 seconds' })}
            ${button('mute', 'vol', 28, { label: 'Mute' })}
            <div class="slider volume"><div class="track"><div class="fill"></div></div><div class="thumb"></div></div>
          </div>
          <div class="group">
            ${info.hasNext ? button('next', 'next', 28, { label: 'Next episode' }) : ''}
            ${button('cc', 'cc', 28, { label: 'Audio' })}
            ${button('speed', 'speed', 28, { label: 'Speed' })}
            ${button('full', 'full', 28, { label: 'Fullscreen' })}
          </div>
        </div>
      </div>`;
    box.querySelector('.title').textContent = info.title || '';
    box.querySelector('.episode').textContent = info.episode || '';
    document.body.append(box);
    const video = box.querySelector('video');
    active = video;
    const status = box.querySelector('.status');
    let current = null;
    let closed = false;
    const close = (next = false) => {
      if (closed) return;
      closed = true;
      const position = video.currentTime || start;
      const duration = Number.isFinite(video.duration) ? video.duration : 0;
      current?.stop();
      if (active === video) active = null;
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
      box.remove();
      document.removeEventListener('keydown', onKey, true);
      document.getElementById('canvas')?.focus();
      resolve({ position, duration, next });
    };
    const ui = wireControls(box, video, info, () => close(true), () => current);
    // Esc closes (after leaving fullscreen), like the desktop player; the canvas doesn't see keys.
    const onKey = (e) => {
      if (e.key === 'Escape') document.fullscreenElement ? document.exitFullscreen() : close();
      else ui.key(e);
      e.stopPropagation();
    };
    document.addEventListener('keydown', onKey, true);
    box.querySelector('.back').addEventListener('click', () => close());
    video.addEventListener('ended', () => close(info.hasNext));
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
      })
      .catch((e) => {
        status.textContent = String(e.message ?? e);
      });
  });
}

/** A slider like Freya's (the desktop's): drag or click; `onValue(0..1)`. */
function slider(el, onValue) {
  const at = (e) => {
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left - 9) / (r.width - 18)));
  };
  el.addEventListener('pointerdown', (e) => {
    el.setPointerCapture(e.pointerId);
    el.dragging = true;
    onValue(at(e), true);
  });
  el.addEventListener('pointermove', (e) => el.dragging && onValue(at(e), true));
  el.addEventListener('pointerup', (e) => {
    el.dragging = false;
    onValue(at(e), false);
  });
  return (v) => {
    el.querySelector('.fill').style.width = `${v * 100}%`;
    el.querySelector('.thumb').style.left = `calc(${v} * (100% - 18px))`;
  };
}

function wireControls(box, video, info, next, current) {
  const $ = (s) => box.querySelector(s);
  const setIcon = (cls, name) => {
    const b = $(`.${cls}`);
    b.querySelector('svg').outerHTML = svg(name, Number(b.dataset.size), b.dataset.filled === 'true');
  };
  const duration = () => (Number.isFinite(video.duration) ? video.duration : 0);
  const toggle = () => (video.paused ? video.play().catch(() => {}) : video.pause());
  let seekPreview = null;
  const setSeek = slider($('.seek'), (v, dragging) => {
    seekPreview = dragging ? v : null;
    if (!dragging) video.currentTime = v * duration();
    paint();
  });
  const setVolume = slider($('.volume'), (v) => {
    video.volume = v;
    video.muted = v === 0;
  });
  const paint = () => {
    const d = duration();
    const pos = seekPreview != null ? seekPreview * d : video.currentTime;
    setSeek(d ? pos / d : 0);
    $('.left').textContent = `-${clock(d - pos)}`;
    setIcon('play', video.paused ? 'play' : 'pause');
    setIcon('mute', video.muted || video.volume === 0 ? 'muted' : video.volume < 0.5 ? 'volLow' : 'vol');
    setVolume(video.muted ? 0 : video.volume);
    setIcon('full', document.fullscreenElement ? 'exitFull' : 'full');
    box.classList.toggle('paused', video.paused && video.readyState >= 2);
    box.classList.toggle('waiting', !video.paused && video.readyState < 3);
  };
  for (const e of ['play', 'pause', 'timeupdate', 'waiting', 'playing', 'canplay', 'seeking', 'seeked', 'volumechange', 'durationchange', 'loadeddata']) video.addEventListener(e, paint);
  document.addEventListener('fullscreenchange', paint);
  paint();
  // The controls fade out 3 s after the pointer stops, except while paused or a menu is open.
  let idle = null;
  const wake = () => {
    box.classList.remove('idle');
    clearTimeout(idle);
    idle = setTimeout(() => !video.paused && $('.menu').hidden && box.classList.add('idle'), 3000);
  };
  box.addEventListener('pointermove', wake);
  video.addEventListener('pause', wake);
  wake();
  // A click pauses or plays; a double click toggles fullscreen (and undoes the pause), as on desktop.
  let lastClick = 0;
  video.addEventListener('click', () => {
    $('.menu').hidden = true;
    toggle();
    if (performance.now() - lastClick < 300) {
      toggle();
      fullscreen();
    }
    lastClick = performance.now();
  });
  const fullscreen = () => (document.fullscreenElement ? document.exitFullscreen() : box.requestFullscreen()).catch(() => {});
  $('.play').addEventListener('click', toggle);
  $('.big').addEventListener('click', toggle);
  $('.back10').addEventListener('click', () => (video.currentTime -= 10));
  $('.fwd10').addEventListener('click', () => (video.currentTime += 10));
  $('.mute').addEventListener('click', () => (video.muted = !video.muted));
  $('.full').addEventListener('click', fullscreen);
  $('.next')?.addEventListener('click', next);
  // Menus: the desktop's panel (bottom right), one column of options with a check on the chosen one.
  const menu = $('.menu');
  const openMenu = (kind) => {
    if (!menu.hidden && menu.dataset.kind === kind) return (menu.hidden = true);
    menu.dataset.kind = kind;
    const option = (text, selected, on) => {
      const b = document.createElement('button');
      b.className = 'option';
      b.innerHTML = `<span class="check">${selected ? svg('check', 18) : ''}</span>`;
      b.append(text);
      b.addEventListener('click', () => {
        on();
        menu.hidden = true;
      });
      return b;
    };
    const column = (title, items) => {
      const c = document.createElement('div');
      c.className = 'column';
      c.innerHTML = `<div class="head">${title}</div>`;
      c.append(...items);
      return c;
    };
    menu.replaceChildren();
    if (kind === 'speed') {
      menu.append(column('Speed', SPEEDS.map((v) => option(v === 1 ? 'Normal' : `${v}x`, video.playbackRate === v, () => (video.playbackRate = v)))));
    } else {
      const audio = current()?.plan?.audio;
      menu.append(column('Audio', [option(audio ? audio.replace(/\..*/, '').toUpperCase() : 'Default', true, () => {})]));
      menu.append(column('Subtitles', [option('Off', true, () => {})]));
    }
    menu.hidden = false;
    wake();
  };
  $('.cc').addEventListener('click', () => openMenu('cc'));
  $('.speed').addEventListener('click', () => openMenu('speed'));
  return {
    // The desktop player's keys: Space/Enter pause, arrows seek and change volume, F, M, N.
    key(e) {
      const keys = {
        ' ': toggle,
        Enter: toggle,
        ArrowLeft: () => (video.currentTime -= 10),
        ArrowRight: () => (video.currentTime += 10),
        ArrowUp: () => (video.volume = Math.min(1, video.volume + 0.05)),
        ArrowDown: () => (video.volume = Math.max(0, video.volume - 0.05)),
        f: fullscreen,
        m: () => (video.muted = !video.muted),
        n: () => info.hasNext && next(),
      };
      const run = keys[e.key.length === 1 ? e.key.toLowerCase() : e.key];
      if (!run) return;
      e.preventDefault();
      run();
      wake();
    },
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
