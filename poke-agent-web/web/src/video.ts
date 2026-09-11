// The client half of `src/web/video.rs`'s block-diff codec: a port of its `VideoDecoder`, which
// specifies the wire format. DOM-free: it owns a plain RGBA buffer that `<Screen>` wraps once.

import type { Connection } from './api';
import { subscribeFramed } from './stream';

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
  /** RGBA, row-major; alpha stays opaque. */
  readonly rgba = new Uint8ClampedArray(WIDTH * HEIGHT * 4);

  /** Replaced outright by a keyframe, appended to by a delta. */
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

    // A keyframe names every block in order; a delta uses a bitmap or an index list.
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

/** `/api/video` over `stream.ts`; every connection opens with a keyframe, so a reconnect is the resync. */
export function subscribeVideo(
  url: string,
  onMessage: (message: ArrayBuffer) => void,
  onConnection: (connection: Connection) => void,
): () => void {
  return subscribeFramed(url, onMessage, onConnection, { inflate: true, label: '/api/video' });
}
