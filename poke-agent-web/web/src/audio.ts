// The client half of `/api/audio`: a twelve-byte header, then one bare Opus packet per frame,
// decoded by WebCodecs and scheduled on the `AudioContext` clock. Never pass a `description` to
// `configure()`: WebCodecs reads one as Ogg encapsulation.

import type { Connection } from './api';
import { subscribeFramed, type Fatal } from './stream';

/** How far ahead of the context clock audio is scheduled: the feature's whole latency. */
const TARGET_LEAD_S = 0.18;
/** Below this the next frame cannot be scheduled, since a source cannot start in the past. */
const UNDERRUN_S = 0.005;
/** Above this we are not streaming, we are replaying. */
const MAX_LEAD_S = 0.6;
/** Ceiling on the playback-rate trim, about nine cents of pitch. */
const MAX_TRIM = 0.005;
const TRIM_FULL_SCALE_S = 0.15;
/** Long enough to kill a click, short enough not to be heard as a swell. */
const FADE_S = 0.008;
/** A decoder this far behind will not catch up, and queueing only builds latency. */
const MAX_DECODE_QUEUE = 20;
/** How long `AudioContext.resume()` gets before the browser is assumed to want a gesture. */
const RESUME_GRACE_MS = 1000;

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
  startAt: number;
  rate: number;
  /** Where the *next* frame should start, given this one's duration. */
  nextAt: number;
}

// What to do with one decoded frame, kept pure. `anchor` re-seats after an underrun, `resync` makes
// one bounded cut when too far behind, and `play` trims the rate by at most `MAX_TRIM`, which absorbs
// the clock drift that would otherwise force a cut every few minutes.
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

/** One listening session; nothing is fetched until `resume()` succeeds. */
export class AudioPlayer {
  private context: AudioContext | null = null;
  private master: GainNode | null = null;
  private decoder: AudioDecoder | null = null;
  private format: AudioFormat | null = null;
  /** What the decoder was configured with, which outlives `format` being forgotten. */
  private format0: { sampleRate: number; numberOfChannels: number } | null = null;
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

  /** Returns whether sound actually started; `false` means the browser would not resume. */
  async start(): Promise<boolean> {
    if (this.context) return true;
    let context: AudioContext;
    try {
      // Matching the stream's rate avoids a resample; not every device accepts it.
      context = new AudioContext({ sampleRate: 48000, latencyHint: 'playback' });
    } catch {
      context = new AudioContext();
    }
    // Raced: without user activation `resume()` never settles, and `start` would hang at mount.
    await Promise.race([
      context.resume().catch(() => {}),
      new Promise((settle) => setTimeout(settle, RESUME_GRACE_MS)),
    ]);
    if (context.state !== 'running') {
      await context.close().catch(() => {});
      return false;
    }

    this.context = context;
    this.master = context.createGain();
    this.master.gain.value = 0;
    this.master.connect(context.destination);
    // Safari parks a context in `'interrupted'`; re-anchor so it does not resume into stale deadlines.
    context.addEventListener('statechange', this.onStateChange);

    this.unsubscribe = subscribeFramed(this.url, this.onMessage, this.connectionChanged, {
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
    this.format0 = null;
    this.context?.removeEventListener('statechange', this.onStateChange);
    this.context?.close().catch(() => {});
    this.context = null;
    this.master = null;
    this.format = null;
    this.nextAt = null;
    this.timestamp = 0;
  }

  // Each connection re-sends the header, so the format is forgotten on `'live'`, before the first
  // message. `nextAt` is kept: a reconnect is not a discontinuity.
  private connectionChanged = (connection: Connection) => {
    if (connection === 'live') this.format = null;
    this.onConnection(connection);
  };

  private onStateChange = () => {
    if (!this.context) return;
    if (this.context.state !== 'running') {
      void this.context.resume().catch(() => {});
      this.nextAt = null;
    }
  };

  private onMessage = (message: ArrayBuffer) => {
    if (!this.format) {
      let format: AudioFormat;
      try {
        format = parseHeader(message);
      } catch (failure) {
        console.error('audio stream opened with something else', failure);
        return;
      }
      const changed =
        this.format0?.sampleRate !== format.sampleRate ||
        this.format0?.numberOfChannels !== format.channels;
      this.format = format;
      // A reconnect keeps its decoder, whose timestamp counter must stay monotonic.
      if (!this.decoder || this.decoder.state !== 'configured' || changed) this.buildDecoder();
      return;
    }
    const decoder = this.decoder;
    if (!decoder || decoder.state !== 'configured') return;
    if (decoder.decodeQueueSize > MAX_DECODE_QUEUE) return;
    // Every Opus packet is a key chunk. The timestamp is a local monotonic counter, never used for scheduling.
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
    // Closed, not dropped: a configured decoder holds resources.
    if (this.decoder && this.decoder.state !== 'closed') this.decoder.close();
    this.timestamp = 0;
    this.format0 = { sampleRate: format.sampleRate, numberOfChannels: format.channels };
    this.decoder = new AudioDecoder({
      output: this.onFrame,
      // Rebuild and keep the connection: the next Opus packet repairs itself, unlike a video frame.
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
    // `data.sampleRate` is the authority: Chrome decodes Opus at 48 kHz whatever was configured.
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

    // Armed on every frame and re-armed by the next, so it fires only when the stream really stops.
    clearTimeout(this.tail);
    const fadeIn = (plan.nextAt - FADE_S - now) * 1000;
    this.tail = setTimeout(
      () => {
        if (this.context) this.rampTo(0, this.context.currentTime);
      },
      Math.max(0, fadeIn),
    );
  };

  /** Pins the current value before ramping, since `cancelScheduledValues` alone clicks. */
  private rampTo(value: number, at: number) {
    const gain = this.master?.gain;
    if (!gain) return;
    if (Math.abs(gain.value - value) < 1e-3) return;
    gain.cancelScheduledValues(at);
    gain.setValueAtTime(gain.value, at);
    gain.linearRampToValueAtTime(value, at + FADE_S);
  }
}
