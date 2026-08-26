// The client half of `/api/audio`: raw Opus packets in, sound out.
//
// The stream is `src/web/audio.rs`'s — a twelve-byte header, then one bare Opus packet per 20 ms,
// length-prefixed by `src/web/mod.rs` and **not** compressed. Decoding is WebCodecs' `AudioDecoder`
// and playback is one `AudioBufferSourceNode` per frame, scheduled on the `AudioContext` clock.
//
// ⚠️ **No `description` is ever passed to `configure()`.** The W3C WebCodecs Opus registration says
// supplying one means the bitstream is Ogg-encapsulated, so handing it an `OpusHead` — the obvious
// "we have a header, let us make it the standard one" — puts the decoder in the wrong mode for the
// bare packets it is about to receive. Our header is for `sampleRate` and `numberOfChannels` and is
// never given to the decoder.

import type { Connection } from './api';
import { subscribeFramed, type Fatal } from './stream';

/** How far ahead of the context clock audio is kept scheduled. The whole latency of the feature. */
const TARGET_LEAD_S = 0.18;
/** Below this the next frame cannot be scheduled at all — a source cannot be started in the past. */
const UNDERRUN_S = 0.005;
/** Above this we are not streaming, we are replaying. */
const MAX_LEAD_S = 0.6;
/** Ceiling on the playback-rate trim. 0.5% is about nine cents of pitch. */
const MAX_TRIM = 0.005;
/** The lead error at which the trim saturates. */
const TRIM_FULL_SCALE_S = 0.15;
/** Every fade here. Long enough to kill a click, short enough not to be heard as a swell. */
const FADE_S = 0.008;
/** Backpressure: a decoder this far behind is not going to catch up, and queueing builds latency. */
const MAX_DECODE_QUEUE = 20;

const MAGIC = 0x31414247; // "GBA1", little-endian
export const HEADER_LEN = 12;

export interface AudioFormat {
  version: number;
  channels: number;
  sampleRate: number;
  frameMs: number;
}

/** The first message of every connection. Anything else is a stream we do not understand. */
export function parseHeader(buffer: ArrayBuffer): AudioFormat {
  if (buffer.byteLength !== HEADER_LEN) throw new Error(`audio header was ${buffer.byteLength} bytes`);
  const view = new DataView(buffer);
  if (view.getUint32(0, true) !== MAGIC) throw new Error('audio stream did not open with a header');
  const version = view.getUint8(4);
  if (version !== 1) throw new Error(`audio stream is version ${version} and this page speaks 1`);
  return {
    version,
    channels: view.getUint8(5),
    sampleRate: view.getUint32(6, true),
    frameMs: view.getUint16(10, true),
  };
}

// ── The scheduler ────────────────────────────────────────────────────────────────────────────────

export type Action = 'anchor' | 'resync' | 'play';

export interface Scheduled {
  action: Action;
  /** When this frame should start, on the `AudioContext` clock. */
  startAt: number;
  /** What to set `playbackRate` to. */
  rate: number;
  /** Where the *next* frame should start, given this one's duration. */
  nextAt: number;
}

/**
 * What to do with one decoded frame. **Pure**, and deliberately so: the whole drift algorithm is
 * one function that can be read, reasoned about and exercised without an `AudioContext`, in the
 * spirit of `video.ts`'s DOM-free decoder.
 *
 * Three cases, and each is a different failure:
 *
 * - **anchor** — nothing is scheduled, or what was scheduled has already run out. The gap has
 *   happened; all this can do is re-seat the buffer. It is also, unchanged, the path a run parked on
 *   a spent quota takes: no packets for hours, then one.
 * - **resync** — we are further behind live than `MAX_LEAD_S`. A lagged subscriber handed a burst
 *   out of the server's ring, a tab that was frozen, a decoder that stalled and caught up. Dropping
 *   one frame per excess would be gentler per event but leaves a discontinuity each time and fires
 *   dozens of times per burst; one bounded cut is a single artifact instead of fifty.
 * - **play** — the steady state, with a rate trim of at most ±`MAX_TRIM`.
 *
 * ⚠️ **The trim is the part that earns its place, and it is not a refinement.** `CLAUDE.md` records
 * that the host's wall/emulated ratio has a *ceiling* of 1.0007×, so the emulator's clock and the
 * `AudioContext`'s separate by around 2.5 s an hour in a perfectly healthy run — and a sound card is
 * independently off the system clock by up to 0.1% again. Without the trim that drift alone crosses
 * `MAX_LEAD_S` and forces an audible cut every ten minutes or so, for no reason. ±0.5% corrects far
 * faster than either accumulates (it closes a 100 ms error in 20 s) and is inaudible on chiptune, so
 * drift is absorbed silently and the cut is kept for things that really are discontinuities.
 */
export function schedule(nextAt: number | null, now: number, duration: number): Scheduled {
  const lead = nextAt === null ? Number.NEGATIVE_INFINITY : nextAt - now;

  if (nextAt === null || lead <= UNDERRUN_S) {
    const startAt = now + TARGET_LEAD_S;
    return { action: 'anchor', startAt, rate: 1, nextAt: startAt + duration };
  }
  if (lead > MAX_LEAD_S) {
    const startAt = now + TARGET_LEAD_S;
    return { action: 'resync', startAt, rate: 1, nextAt: startAt + duration };
  }
  const error = (lead - TARGET_LEAD_S) / TRIM_FULL_SCALE_S;
  const rate = 1 + Math.max(-1, Math.min(1, error)) * MAX_TRIM;
  return { action: 'play', startAt: nextAt, rate, nextAt: nextAt + duration / rate };
}

// ── The player ───────────────────────────────────────────────────────────────────────────────────

/** Whether this browser can play the stream at all. */
export async function audioIsSupported(): Promise<boolean> {
  if (typeof AudioDecoder === 'undefined') return false;
  try {
    const support = await AudioDecoder.isConfigSupported({
      codec: 'opus',
      sampleRate: 48000,
      numberOfChannels: 1,
    });
    return support.supported === true;
  } catch {
    return false;
  }
}

/**
 * One listening session: a context, a decoder and the connection that feeds them.
 *
 * ⚠️ **Nothing is constructed until the viewer asks.** An `AudioContext` starts suspended under
 * every browser's autoplay policy, and a `/api/audio` fetch opened against a suspended context is
 * 24 kbit/s being decoded into nothing — so the stream is opened *after* `resume()` succeeds, which
 * is also what makes sound free for every viewer who never turns it on.
 */
export class AudioPlayer {
  private context: AudioContext | null = null;
  private master: GainNode | null = null;
  private decoder: AudioDecoder | null = null;
  private format: AudioFormat | null = null;
  private unsubscribe: (() => void) | null = null;
  private live = new Set<AudioBufferSourceNode>();
  private nextAt: number | null = null;
  private timestamp = 0;
  /** Fires as the last scheduled audio ends, so a stream that stops fades instead of clicking. */
  private tail: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly url: string,
    private readonly onConnection: (connection: Connection) => void,
    private readonly onFatal: (why: Fatal) => void,
  ) {}

  /** Returns whether sound actually started; `false` means the browser refused to resume. */
  async start(): Promise<boolean> {
    if (this.context) return true;
    let context: AudioContext;
    try {
      // Matching the stream's rate removes a resample per buffer and makes the drift arithmetic
      // exact. Not every device will take the hint, hence the fallback.
      context = new AudioContext({ sampleRate: 48000, latencyHint: 'playback' });
    } catch {
      context = new AudioContext();
    }
    try {
      await context.resume();
    } catch {
      /* handled by the state check below */
    }
    if (context.state !== 'running') {
      await context.close().catch(() => {});
      return false;
    }

    this.context = context;
    this.master = context.createGain();
    this.master.gain.value = 0;
    this.master.connect(context.destination);
    // ⚠️ Safari parks a context in `'interrupted'` after a call or a lock screen, and an OS sleep
    // does the same elsewhere. Re-anchoring is what stops it resuming into deadlines set hours ago.
    context.addEventListener('statechange', this.onStateChange);

    this.unsubscribe = subscribeFramed(this.url, this.onMessage, this.onConnection, {
      inflate: false,
      label: '/api/audio',
      onFatal: this.onFatal,
    });
    return true;
  }

  stop() {
    clearTimeout(this.tail);
    this.unsubscribe?.();
    this.unsubscribe = null;
    if (this.master && this.context) this.rampTo(0, this.context.currentTime);
    for (const source of this.live) {
      try {
        source.stop();
      } catch {
        /* already ended */
      }
    }
    this.live.clear();
    if (this.decoder && this.decoder.state !== 'closed') this.decoder.close();
    this.decoder = null;
    this.context?.removeEventListener('statechange', this.onStateChange);
    this.context?.close().catch(() => {});
    this.context = null;
    this.master = null;
    this.format = null;
    this.nextAt = null;
    this.timestamp = 0;
  }

  private onStateChange = () => {
    if (!this.context) return;
    if (this.context.state !== 'running') {
      void this.context.resume().catch(() => {});
      this.nextAt = null;
    }
  };

  private onMessage = (message: ArrayBuffer) => {
    if (!this.format) {
      try {
        this.format = parseHeader(message);
      } catch (failure) {
        console.error('audio stream opened with something else', failure);
        return;
      }
      this.buildDecoder();
      return;
    }
    const decoder = this.decoder;
    if (!decoder || decoder.state !== 'configured') return;
    // ⚠️ A decoder this far behind will not catch up, and every queued packet is latency the
    // scheduler then has to tear down with an audible cut. Dropping is the cheaper answer.
    if (decoder.decodeQueueSize > MAX_DECODE_QUEUE) return;
    // ⚠️ **Every Opus packet is a key chunk.** Opus has no delta frames, and WebCodecs rejects a
    // stream whose first chunk is not one. The timestamp is a local counter, monotonic because the
    // spec requires it — it is deliberately *not* used for scheduling, since there is no shared
    // clock with the server and the emulated one legitimately gaps.
    decoder.decode(
      new EncodedAudioChunk({
        type: 'key',
        timestamp: this.timestamp,
        duration: this.format.frameMs * 1000,
        data: message,
      }),
    );
    this.timestamp += this.format.frameMs * 1000;
  };

  private buildDecoder() {
    const format = this.format;
    if (!format) return;
    this.decoder = new AudioDecoder({
      output: this.onFrame,
      // ⚠️ **Rebuild and keep the connection, which is the opposite of what `Screen` does.** A video
      // decode error means the palette and the pixels are both suspect and only a fresh keyframe
      // repairs them, so reconnecting is the fix. The next Opus packet repairs itself, so
      // reconnecting here would throw away the jitter buffer to fix nothing.
      error: (failure) => {
        console.error('audio decoder failed, rebuilding', failure);
        if (this.decoder && this.decoder.state !== 'closed') this.decoder.close();
        this.nextAt = null;
        this.buildDecoder();
      },
    });
    this.decoder.configure({
      codec: 'opus',
      sampleRate: format.sampleRate,
      numberOfChannels: format.channels,
    });
  }

  private onFrame = (data: AudioData) => {
    const context = this.context;
    const master = this.master;
    if (!context || !master) {
      data.close();
      return;
    }
    // ⚠️ **Never assume the output rate matches what we configured.** Chrome decodes Opus at 48 kHz
    // whatever `sampleRate` said, so `data.sampleRate` is the authority for both the buffer and the
    // duration.
    const frames = data.numberOfFrames;
    const rate = data.sampleRate;
    const channels = data.numberOfChannels;
    const buffer = context.createBuffer(channels, frames, rate);
    for (let channel = 0; channel < channels; channel += 1) {
      const plane = new Float32Array(frames);
      data.copyTo(plane, { planeIndex: channel, format: 'f32-planar' });
      buffer.copyToChannel(plane, channel);
    }
    data.close();

    const now = context.currentTime;
    const plan = schedule(this.nextAt, now, frames / rate);

    if (plan.action === 'resync') {
      this.rampTo(0, now);
      for (const source of this.live) {
        try {
          source.stop(now + FADE_S);
        } catch {
          /* already ended */
        }
      }
      this.live.clear();
    }
    if (plan.action !== 'play') this.rampTo(1, plan.startAt);
    else this.rampTo(1, now);

    const source = context.createBufferSource();
    source.buffer = buffer;
    source.playbackRate.value = plan.rate;
    source.connect(master);
    source.onended = () => this.live.delete(source);
    source.start(plan.startAt);
    this.live.add(source);
    this.nextAt = plan.nextAt;

    // Arm the fade-out for the moment this frame runs out. Every subsequent frame cancels and
    // re-arms it, so it only ever fires when the stream has genuinely stopped — which is what turns
    // a park, a network gap and a `MAX_CATCHUP` drop into a fade rather than a click.
    clearTimeout(this.tail);
    const fadeIn = (plan.nextAt - FADE_S - now) * 1000;
    this.tail = setTimeout(
      () => {
        if (this.context) this.rampTo(0, this.context.currentTime);
      },
      Math.max(0, fadeIn),
    );
  };

  /**
   * ⚠️ **Never `cancelScheduledValues` on its own**, which drops the gain to whatever was last set
   * and clicks. Pinning the current value first and ramping from it is what makes every transition
   * here inaudible.
   */
  private rampTo(value: number, at: number) {
    const gain = this.master?.gain;
    if (!gain) return;
    if (Math.abs(gain.value - value) < 1e-3) return;
    gain.cancelScheduledValues(at);
    gain.setValueAtTime(gain.value, at);
    gain.linearRampToValueAtTime(value, at + FADE_S);
  }
}
