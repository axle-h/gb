// The client half of the block-diff codec in `src/web/video.rs`. That module's docs are the wire
// format's specification; this is a direct port of its `VideoDecoder`, which is the reference
// implementation and the thing the Rust tests hold to.
//
// Deliberately DOM-free: it owns a plain RGBA buffer rather than an `ImageData`, so the whole
// decoder can be exercised under node against a live server without a browser. `<Screen>` wraps the
// buffer in an `ImageData` once, with no copy.

export const WIDTH = 160;
export const HEIGHT = 144;

const BLOCK = 8;
const BLOCKS_X = WIDTH / BLOCK;
const BLOCK_COUNT = (WIDTH / BLOCK) * (HEIGHT / BLOCK);
const BLOCK_PIXELS = BLOCK * BLOCK;
const BITMAP_BYTES = Math.ceil(BLOCK_COUNT / 8);

const VERSION = 2;
const FLAG_KEYFRAME = 1;
const FLAG_BITMAP = 2;

export interface AppliedMessage {
  seq: number;
  keyframe: boolean;
}

export class VideoDecoder {
  /** RGBA, row-major, ready to hand to `new ImageData(...)`. Alpha is opaque and stays that way. */
  readonly rgba = new Uint8ClampedArray(WIDTH * HEIGHT * 4);

  /** Persists across messages: replaced outright by a keyframe, appended to by a delta. */
  private palette: number[][] = [];

  constructor() {
    this.rgba.fill(255);
  }

  /** Throws on a malformed message rather than painting nonsense; the caller re-syncs. */
  apply(buffer: ArrayBuffer): AppliedMessage {
    const data = new DataView(buffer);
    const bytes = new Uint8Array(buffer);
    let at = 0;
    const u8 = () => data.getUint8(at++);
    const u16 = () => {
      const value = data.getUint16(at, true);
      at += 2;
      return value;
    };

    if (u8() !== VERSION) throw new Error('unsupported video version');
    const flags = u8();
    const keyframe = (flags & FLAG_KEYFRAME) !== 0;
    const seq = u16();
    const bits = u8();
    if (bits !== 1 && bits !== 2 && bits !== 4 && bits !== 8) {
      throw new Error(`${bits} bits per pixel is not 1, 2, 4 or 8`);
    }

    const paletteLength = u8();
    if (keyframe) this.palette = [];
    for (let i = 0; i < paletteLength; i++) this.palette.push([u8(), u8(), u8()]);

    // A keyframe's block list is implicit — every block, in order — which is what makes it
    // standalone. A delta names its blocks either as a bitmap or as a list of indices, whichever the
    // encoder found smaller.
    const blocks: number[] = [];
    if (keyframe) {
      for (let b = 0; b < BLOCK_COUNT; b++) blocks.push(b);
    } else if ((flags & FLAG_BITMAP) !== 0) {
      const map = bytes.subarray(at, at + BITMAP_BYTES);
      at += BITMAP_BYTES;
      for (let b = 0; b < BLOCK_COUNT; b++) {
        if ((map[b >> 3] & (1 << (b & 7))) !== 0) blocks.push(b);
      }
    } else {
      const count = u16();
      for (let i = 0; i < count; i++) blocks.push(u16());
    }

    const perByte = 8 / bits;
    const mask = (1 << bits) - 1;
    const payloadBytes = BLOCK_PIXELS / perByte;
    for (const block of blocks) {
      if (block >= BLOCK_COUNT) throw new Error(`block index ${block} out of range`);
      if (at + payloadBytes > bytes.length) throw new Error('video message ended early');
      this.paint(block, bytes.subarray(at, at + payloadBytes), bits, perByte, mask);
      at += payloadBytes;
    }
    if (at !== bytes.length) throw new Error(`${bytes.length - at} trailing bytes`);
    return { seq, keyframe };
  }

  private paint(
    block: number,
    payload: Uint8Array,
    bits: number,
    perByte: number,
    mask: number,
  ): void {
    const x0 = (block % BLOCKS_X) * BLOCK;
    const y0 = Math.floor(block / BLOCKS_X) * BLOCK;
    for (let slot = 0; slot < BLOCK_PIXELS; slot++) {
      const index =
        bits === 8 ? payload[slot] : (payload[(slot / perByte) | 0] >> ((slot % perByte) * bits)) & mask;
      const colour = this.palette[index];
      if (colour === undefined) throw new Error('a block referenced a colour the palette never carried');
      const offset = ((y0 + ((slot / BLOCK) | 0)) * WIDTH + x0 + (slot % BLOCK)) * 4;
      this.rgba[offset] = colour[0];
      this.rgba[offset + 1] = colour[1];
      this.rgba[offset + 2] = colour[2];
    }
  }
}

/**
 * `/api/video` is a length-prefixed binary stream, deflated across the whole connection — see the
 * route's docs in `src/web/mod.rs` for the measurements that made it one. This turns the response
 * body into the messages `VideoDecoder.apply` wants.
 *
 * ⚠️ **The compression is part of the protocol, not a `Content-Encoding`.** A declared encoding
 * invites a proxy to decompress and recompress it, which buffers whole messages and shows up as
 * stutter only in production. `DecompressionStream` is native and does the same job here, where we
 * can see it.
 *
 * A zero-length message is the keep-alive and yields nothing.
 */
async function* readVideoStream(
  body: ReadableStream<Uint8Array>,
  signal: AbortSignal,
): AsyncGenerator<ArrayBuffer> {
  // Piped rather than `pipeThrough`ed so the abort signal reaches the *source*: aborting the reader
  // alone leaves the response body draining in the background until the server notices.
  // The cast is the DOM types being stricter than the spec: `DecompressionStream` accepts any
  // `BufferSource`, which `Uint8Array` is, but the two `WritableStream`s are not assignable.
  const inflating = new DecompressionStream('deflate');
  body.pipeTo(inflating.writable as WritableStream<Uint8Array>, { signal }).catch(() => {});
  const reader = (inflating.readable as ReadableStream<Uint8Array>).getReader();
  // The tail of the last chunk that was not yet a whole message. A message spans chunk boundaries
  // routinely — deflate's output has nothing to do with our framing.
  let pending = new Uint8Array(0);

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done || signal.aborted) return;
      const merged = new Uint8Array(pending.length + value.length);
      merged.set(pending);
      merged.set(value, pending.length);
      pending = merged;

      let at = 0;
      for (;;) {
        if (pending.length - at < 4) break;
        const length = new DataView(pending.buffer, pending.byteOffset + at, 4).getUint32(0, true);
        if (pending.length - at - 4 < length) break;
        at += 4;
        if (length > 0) {
          yield pending.buffer.slice(pending.byteOffset + at, pending.byteOffset + at + length);
        }
        at += length;
      }
      pending = pending.subarray(at);
    }
  } finally {
    reader.cancel().catch(() => {});
  }
}

/**
 * Keep `/api/video` open for the length of the run, reconnecting for as long as anyone is watching.
 *
 * `EventSource` used to do this part by itself; a `fetch` does not, so the retry is here. It is not
 * a regression in robustness — `EventSource` gives up permanently on some errors and
 * `useEventStream` already had to rebuild it — but it *is* the one thing binary framing costs.
 *
 * ⚠️ **A reconnect is also the resync.** Every connection opens with a keyframe, so a decoder that
 * has lost the thread is repaired by dropping the connection and starting another. That is why the
 * caller does not need a resync path of its own.
 */
export function subscribeVideo(
  url: string,
  onMessage: (message: ArrayBuffer) => void,
  onConnection: (connection: 'connecting' | 'live' | 'reconnecting') => void,
): () => void {
  const controller = new AbortController();
  const RETRY_MS = 1000;

  (async () => {
    let first = true;
    while (!controller.signal.aborted) {
      try {
        onConnection(first ? 'connecting' : 'reconnecting');
        const response = await fetch(url, { signal: controller.signal, cache: 'no-store' });
        if (!response.ok || !response.body) throw new Error(`/api/video answered ${response.status}`);
        onConnection('live');
        first = false;
        for await (const message of readVideoStream(response.body, controller.signal)) {
          onMessage(message);
        }
      } catch (failure) {
        if (controller.signal.aborted) return;
        console.error('video stream dropped, reconnecting', failure);
      }
      if (controller.signal.aborted) return;
      onConnection('reconnecting');
      await new Promise((resolve) => setTimeout(resolve, RETRY_MS));
    }
  })();

  return () => controller.abort();
}
