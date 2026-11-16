// Relix in the browser: plays a stream link through the browser's own <video> (hardware decoding,
// the browser's compositor, PiP, media keys). Mediabunny reads the file (MKV, MP4, WebM...) as it
// downloads and repackages it into fragmented MP4 for Media Source Extensions: video is copied as is,
// never decoded; audio the browser can't play (AC-3, E-AC-3, DTS) is converted to AAC or Opus.
import {
  ALL_FORMATS, Conversion, Input, Mp4OutputFormat, NullTarget, Output, UrlSource, canEncodeAudio,
} from 'mediabunny';
import { registerAc3Decoder } from '@mediabunny/ac3';
import { registerDtsDecoder } from '@mediabunny/dts';

registerAc3Decoder();
registerDtsDecoder();

/** Seconds kept buffered ahead of the playhead, and behind it before being dropped. */
const AHEAD = 30;
const BEHIND = 30;

const mse = (type, codec) => codec != null && MediaSource.isTypeSupported(`${type}/mp4; codecs="${codec}"`);

/**
 * What the browser can do with this stream: the plan for its tracks, or why it can't play here.
 * @returns {Promise<{ok: true, video: string|null, audio: string|null, convertAudio: 'aac'|'opus'|null, duration: number} | {ok: false, reason: string}>}
 */
export async function inspect(input) {
  const [vt, at] = await Promise.all([input.getPrimaryVideoTrack(), pickAudio(input)]);
  const video = vt && (await vt.getCodecParameterString());
  const audio = at && (await at.getCodecParameterString());
  if (vt && !mse('video', video)) {
    return { ok: false, reason: `This browser can't play ${video ?? 'this video'} (try Chrome or Edge, or open it in Relix)` };
  }
  let convertAudio = null;
  if (at && !mse('audio', audio)) {
    for (const codec of ['aac', 'opus']) {
      const mime = codec === 'aac' ? 'mp4a.40.2' : 'opus';
      if (mse('audio', mime) && (await canEncodeAudio(codec))) {
        convertAudio = codec;
        break;
      }
    }
    if (!convertAudio) return { ok: false, reason: `This browser can't play the audio (${audio})` };
  }
  return { ok: true, video, audio, audioTrack: at, convertAudio, duration: await input.computeDuration() };
}

/**
 * The audio track to play: releases often carry several (TrueHD 7.1 and an AC-3 one for
 * compatibility). One the browser plays as is, else one that can be converted, else the first.
 */
async function pickAudio(input) {
  const tracks = await input.getAudioTracks();
  for (const t of tracks) if (mse('audio', await t.getCodecParameterString())) return t;
  for (const t of tracks) if (await t.canDecode()) return t;
  return tracks[0] ?? null;
}

/** The host doesn't let the page read the file (CORS), or it's unreachable: fetch's TypeError. */
const blocked = (e) => e instanceof TypeError || /fetch|cors/i.test(String(e?.message ?? e));

/**
 * Plays `url` in `video`. Returns { plan, stop() }; seeking with the video's own controls works.
 * `log(event, detail)` reports progress (for the page and for measuring).
 */
export async function play(video, url, log = () => {}) {
  let current;
  try {
    current = await playOnce(video, url, log, false);
  } catch (e) {
    if (!blocked(e)) throw e;
    return playDirect(video, url, log);
  }
  // Audio the browser said it plays can still be refused (AAC with an unusual channel map, say):
  // once, play again with the audio converted.
  const retry = () => {
    if (!current.plan.audio || current.plan.convertAudio || current.stopped) return;
    log('retry', { why: video.error?.message, convertAudio: true });
    current.stop();
    playOnce(video, url, log, true).then((next) => (current = next), (e) => log('error', { message: String(e) }));
  };
  video.addEventListener('error', retry, { once: true });
  return {
    get plan() {
      return current.plan;
    },
    stop() {
      video.removeEventListener('error', retry);
      current.stop();
    },
  };
}

async function playOnce(video, url, log, forceAudio) {
  const started = performance.now();
  const input = new Input({ source: new UrlSource(url), formats: ALL_FORMATS });
  const plan = await inspect(input);
  if (plan.ok && forceAudio && plan.audio && !plan.convertAudio) {
    plan.convertAudio = (await canEncodeAudio('aac')) && mse('audio', 'mp4a.40.2') ? 'aac' : 'opus';
  }
  log('inspected', { ms: Math.round(performance.now() - started), ...plan });
  if (!plan.ok) throw new Error(plan.reason);

  const ms = new MediaSource();
  video.src = URL.createObjectURL(ms);
  await new Promise((r) => ms.addEventListener('sourceopen', r, { once: true }));
  ms.duration = plan.duration;

  let sb = null;
  let run = null; // the conversion feeding the buffer, from one seek point
  let stopped = false;

  // Appends go one at a time: a SourceBuffer takes one change at once.
  const idle = () => (sb.updating ? new Promise((r) => sb.addEventListener('updateend', r, { once: true })) : null);
  const append = async (data) => {
    await idle();
    try {
      sb.appendBuffer(data);
    } catch (e) {
      if (e.name !== 'QuotaExceededError') throw e;
      // Full: drop what's behind the playhead and try again.
      sb.remove(0, Math.max(0, video.currentTime - 5));
      await idle();
      sb.appendBuffer(data);
    }
    await idle();
  };

  const bufferedEnd = () => {
    const b = video.buffered;
    for (let i = 0; i < b.length; i++) {
      if (b.start(i) <= video.currentTime + 0.5 && video.currentTime <= b.end(i)) return b.end(i);
    }
    return video.currentTime;
  };

  /** Feeds the buffer from `start` seconds until the end, AHEAD seconds at a time. */
  const feed = async (start) => {
    const mine = { cancelled: false, conversion: null };
    run = mine;
    let queue = Promise.resolve();
    let first = true;
    const enqueue = (data) => {
      // Copies: Mediabunny may reuse its buffers.
      const copy = data.slice();
      queue = queue.then(async () => {
        if (mine.cancelled) return;
        if (first) {
          first = false;
          // The output's type is known once it writes (not before the conversion starts).
          if (!sb) {
            const mime = await output.getMimeType();
            sb = ms.addSourceBuffer(mime);
            sb.mode = 'segments';
            log('mime', { mime });
          }
          await idle();
          // Output timestamps start at 0 at `start`.
          sb.timestampOffset = start;
        }
        await append(copy);
      });
    };
    const output = new Output({
      format: new Mp4OutputFormat({
        fastStart: 'fragmented',
        minimumFragmentDuration: 1,
        onFtyp: enqueue,
        onMoov: enqueue,
        onMoof: enqueue,
        onMdat: enqueue,
      }),
      target: new NullTarget(),
    });
    const conversion = await Conversion.init({
      input,
      output,
      trim: { start },
      // The chosen audio track only.
      audio: (track) => (track !== plan.audioTrack ? { discard: true } : plan.convertAudio ? { codec: plan.convertAudio, forceTranscode: true } : {}),
      // Copy video and audio exactly where they are (keeps the timeline as the file has it).
      copy: { boundaryPolicy: 'expand' },
    });
    if (!conversion.isValid) throw new Error(`Can't repackage: ${JSON.stringify(conversion.discardedTracks.map((d) => d.reason))}`);
    mine.conversion = conversion;
    // Keep converting while less than AHEAD seconds are buffered.
    let until = 0;
    const end = plan.duration - start;
    while (!mine.cancelled && until < end) {
      if (bufferedEnd() - video.currentTime > AHEAD) {
        await new Promise((r) => setTimeout(r, 250));
        continue;
      }
      until = Math.min(until + 10, end + 1);
      await conversion.execute({ until: until >= end ? Infinity : until });
      await queue;
      if (!mine.cancelled && sb && video.currentTime > BEHIND && !sb.updating) {
        sb.remove(0, video.currentTime - BEHIND);
        await idle();
      }
    }
    await queue;
    if (!mine.cancelled && ms.readyState === 'open' && sb && !sb.updating) ms.endOfStream();
  };

  const startAt = async (t) => {
    if (run) {
      run.cancelled = true;
      await run.conversion?.cancel().catch(() => {});
    }
    if (sb) {
      await idle();
      sb.abort();
    }
    if (ms.readyState === 'ended') {
      // A stream that had ended takes appends again after a seek once its duration is re-set.
      ms.duration = plan.duration;
    }
    feed(t).catch((e) => {
      if (e?.name !== 'ConversionCanceledError') log('error', { message: String(e) });
    });
  };

  // A seek outside what's buffered restarts the conversion there (from the key frame before it).
  video.addEventListener('seeking', () => {
    if (stopped) return;
    const t = video.currentTime;
    const b = video.buffered;
    for (let i = 0; i < b.length; i++) if (b.start(i) <= t && t < b.end(i) - 0.5) return;
    log('seek', { to: t });
    startAt(t);
  });
  video.addEventListener('playing', () => log('playing', { ms: Math.round(performance.now() - started), at: video.currentTime }), { once: true });

  await startAt(0);
  return {
    plan,
    get stopped() {
      return stopped;
    },
    stop() {
      stopped = true;
      if (run) run.cancelled = true;
      run?.conversion?.cancel().catch(() => {});
      video.removeAttribute('src');
      video.load();
      input.dispose?.();
    },
  };
}

/**
 * When the host won't let the page read the file (no CORS, common for debrid CDNs): the browser's
 * own player takes the link as is, which needs no permission. Chrome plays many MKV and MP4 files
 * that way, but not DTS, TrueHD or AC-3 audio; the Relix Web Helper extension lifts the block.
 */
function playDirect(video, url, log) {
  log('direct', { why: "the host doesn't allow the page to read the file" });
  video.src = url;
  video.play().catch(() => {});
  // No sound after a few seconds of playing: the audio is a kind the browser can't play.
  const check = setTimeout(() => {
    if (!video.paused && video.webkitAudioDecodedByteCount === 0) log('no-audio', {});
  }, 4000);
  return {
    plan: { ok: true, direct: true },
    stopped: false,
    stop() {
      clearTimeout(check);
      video.removeAttribute('src');
      video.load();
    },
  };
}
