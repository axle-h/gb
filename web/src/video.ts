// The client half of the block-diff codec in `src/web/video.rs` (§5.1 of
// docs/llm-web-playthrough-plan.md). This is a port of W2's hand-written decoder in
// `web/dev/video.html`, which is the only version that has been checked against a real stream — the
// wire format is small enough to rewrite and subtle enough that rewriting it would be a mistake.
//
// Deliberately DOM-free: it owns a plain RGBA buffer rather than an `ImageData`, so the whole
// decoder can be exercised under node against a live server without a browser. `<Screen>` wraps the
// buffer in an `ImageData` once, with no copy.

export const WIDTH = 160;
export const HEIGHT = 144;

const BLOCK = 8;
const BLOCKS_X = WIDTH / BLOCK;
const BLOCK_PIXELS = BLOCK * BLOCK;

const VERSION = 1;
const FLAG_KEYFRAME = 1;
const MODE_RLE = 0;
const MODE_RAW = 1;
const MODE_PACKED = 2;

export interface AppliedMessage {
  seq: number;
  keyframe: boolean;
}

export class VideoDecoder {
  /** RGBA, row-major, ready to hand to `new ImageData(...)`. Alpha is opaque and stays that way. */
  readonly rgba = new Uint8ClampedArray(WIDTH * HEIGHT * 4);

  /** Persists across messages: replaced outright by a keyframe, appended to by a delta. */
  private palette: number[][] = [];
  /** Scratch, reused for every block so a 30 fps stream allocates nothing per frame. */
  private readonly indices = new Uint8Array(BLOCK_PIXELS);

  constructor() {
    this.rgba.fill(255);
  }

  /** Throws on a malformed message rather than painting nonsense; the caller re-syncs. */
  apply(buffer: ArrayBuffer): AppliedMessage {
    const data = new DataView(buffer);
    let at = 0;
    const u8 = () => data.getUint8(at++);
    const u16 = () => {
      const value = data.getUint16(at, true);
      at += 2;
      return value;
    };

    if (u8() !== VERSION) throw new Error('unsupported video version');
    const keyframe = (u8() & FLAG_KEYFRAME) !== 0;
    const seq = u16();

    const paletteLength = u8();
    if (keyframe) this.palette = [];
    for (let i = 0; i < paletteLength; i++) this.palette.push([u8(), u8(), u8()]);

    const blocks = u16();
    for (let b = 0; b < blocks; b++) {
      const block = u16();
      const mode = u8();
      if (mode === MODE_RLE) {
        let slot = 0;
        while (slot < BLOCK_PIXELS) {
          const run = u8();
          const index = u8();
          this.indices.fill(index, slot, slot + run);
          slot += run;
        }
      } else if (mode === MODE_RAW) {
        for (let i = 0; i < BLOCK_PIXELS; i++) this.indices[i] = u8();
      } else if (mode === MODE_PACKED) {
        // Four palette indices, then 2 bits per pixel into them, low bits first.
        const sub = [u8(), u8(), u8(), u8()];
        for (let i = 0; i < BLOCK_PIXELS; i += 4) {
          const bits = u8();
          for (let j = 0; j < 4; j++) this.indices[i + j] = sub[(bits >> (j * 2)) & 3];
        }
      } else {
        throw new Error(`unknown block mode ${mode}`);
      }
      this.paint(block);
    }
    return { seq, keyframe };
  }

  private paint(block: number): void {
    const x0 = (block % BLOCKS_X) * BLOCK;
    const y0 = Math.floor(block / BLOCKS_X) * BLOCK;
    for (let dy = 0; dy < BLOCK; dy++) {
      for (let dx = 0; dx < BLOCK; dx++) {
        const colour = this.palette[this.indices[dy * BLOCK + dx]];
        if (colour === undefined) throw new Error('block references a colour the palette never carried');
        const offset = ((y0 + dy) * WIDTH + x0 + dx) * 4;
        this.rgba[offset] = colour[0];
        this.rgba[offset + 1] = colour[1];
        this.rgba[offset + 2] = colour[2];
      }
    }
  }
}

/** SSE carries the message base64'd on one `data:` line — see `Published::publish_video`. */
export function base64ToBytes(text: string): ArrayBuffer {
  const binary = atob(text);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out.buffer;
}
