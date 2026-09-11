//! The video wire format: an 8×8 block diff with a persistent palette.
//! ```text
//! u8   version (=2)
//! u8   flags            bit0 = keyframe, bit1 = the block list is a bitmap
//! u16  frame_seq        wrapping, little-endian
//! u8   bits_per_pixel   1, 2, 4 or 8 — how wide an index into the palette below is
//! u8   new_palette_len
//! [new_palette_len × u24 RGB]
//! block list            keyframe: absent, all 360 in order
//!                       bit1 set: 45-byte bitmap, block n = byte n/8 bit n%8
//!                       else:     u16 count, then count × u16 block index
//! payloads              one per listed block, in ascending block order, each 64 × bits_per_pixel
//!                       bits packed low-bits-first
//! ```

use std::collections::{HashMap, HashSet};

use gb::lcd_palette::LcdColor;
use gb::ppu::{LCD_HEIGHT, LCD_WIDTH};

pub use poke_agent::frame::{Encoded, Frame, PIXELS};

/// Block edge, in pixels. 8 divides both 160 and 144 exactly, which is why 8 and not 16.
pub const BLOCK: usize = 8;
pub const BLOCKS_X: usize = LCD_WIDTH / BLOCK;
pub const BLOCKS_Y: usize = LCD_HEIGHT / BLOCK;
pub const BLOCK_COUNT: usize = BLOCKS_X * BLOCKS_Y;
const BLOCK_PIXELS: usize = BLOCK * BLOCK;

pub const VERSION: u8 = 2;
const FLAG_KEYFRAME: u8 = 0x01;
const FLAG_BITMAP: u8 = 0x02;
/// One bit per block, so a message that touches most of the screen names them all in 45 bytes
/// instead of 720.
const BITMAP_BYTES: usize = BLOCK_COUNT.div_ceil(8);
const BITMAP_WORTH_IT: usize = (BITMAP_BYTES - 2) / 2 + 1;
/// 255, not 256, and the `u8` index is not the reason.
const MAX_PALETTE: usize = 255;

/// How wide an index into a palette of `entries` has to be.
fn bits_per_pixel(entries: usize) -> u8 {
    match entries {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    }
}

fn payload_bytes(bits: u8) -> usize {
    BLOCK_PIXELS * bits as usize / 8
}

fn pack(indices: &[u8; BLOCK_PIXELS], bits: u8, out: &mut Vec<u8>) {
    if bits == 8 {
        out.extend_from_slice(indices);
        return;
    }
    let per_byte = 8 / bits as usize;
    for chunk in indices.chunks(per_byte) {
        out.push(
            chunk
                .iter()
                .enumerate()
                .fold(0u8, |acc, (i, index)| acc | (index << (i * bits as usize))),
        );
    }
}

// ── Encoder
// ──────────────────────────────────────────────────────────────────────────────────────

pub struct VideoEncoder {
    palette: Vec<LcdColor>,
    index: HashMap<LcdColor, u8>,
    /// What the decoder holds after everything emitted so far, as palette indices — see the
    /// module docs.
    last_sent: Box<[u8; PIXELS]>,
    sent_anything: bool,
    seq: u64,
    /// Scratch for one message's block payloads, kept across calls so a 30 fps stream allocates
    /// nothing per frame.
    staged: Vec<(u16, [u8; BLOCK_PIXELS])>,
}

impl Default for VideoEncoder {
    fn default() -> Self {
        Self {
            palette: Vec::new(),
            index: HashMap::new(),
            last_sent: Box::new([0; PIXELS]),
            sent_anything: false,
            seq: 0,
            staged: Vec::with_capacity(BLOCK_COUNT),
        }
    }
}

impl VideoEncoder {
    /// The sequence number of the most recent [`Self::encode`] that produced a message.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Forget everything the decoder is believed to hold, so the next [`Self::encode`] is a full
    /// keyframe with a fresh palette.
    pub fn restart(&mut self) {
        self.palette.clear();
        self.index.clear();
        self.last_sent = Box::new([0; PIXELS]);
        self.sent_anything = false;
    }

    /// Encode `frame` against everything sent so far.
    pub fn encode(&mut self, frame: &Frame) -> Option<Encoded> {
        let mut keyframe = !self.sent_anything;
        let mut blocks: Vec<u16> = if keyframe {
            (0..BLOCK_COUNT as u16).collect()
        } else {
            (0..BLOCK_COUNT as u16).filter(|&b| self.block_changed(frame, b as usize)).collect()
        };
        if blocks.is_empty() {
            return None;
        }

        // The safety valve, not a normal path: rather than run out of palette part-way through a
        // frame, spend one keyframe on a fresh one.
        if !keyframe && self.should_reset_palette(frame, &blocks) {
            keyframe = true;
            blocks = (0..BLOCK_COUNT as u16).collect();
        }
        if keyframe {
            self.palette.clear();
            self.index.clear();
        }

        // Two passes, and the split is forced: `bits_per_pixel` covers the palette *including*
        // the entries these blocks are about to introduce, so nothing can be written until they
        // have all been interned.
        let palette_base = self.palette.len();
        self.staged.clear();
        for &block in &blocks {
            // A keyframe carries all 360 blocks by definition — that is what makes it standalone.
            self.stage_block(frame, block, keyframe);
        }
        if self.staged.is_empty() {
            // Every candidate resolved to what the decoder already holds.
            self.index.retain(|_, index| (*index as usize) < palette_base);
            self.palette.truncate(palette_base);
            return None;
        }

        self.seq += 1;
        self.sent_anything = true;
        let bits = bits_per_pixel(self.palette.len());
        let bitmap = !keyframe && self.staged.len() >= BITMAP_WORTH_IT;

        let new_entries = &self.palette[palette_base..];
        let mut bytes = Vec::with_capacity(
            6 + new_entries.len() * 3 + BITMAP_BYTES + self.staged.len() * payload_bytes(bits),
        );
        bytes.push(VERSION);
        bytes.push(if keyframe { FLAG_KEYFRAME } else { 0 } | if bitmap { FLAG_BITMAP } else { 0 });
        bytes.extend_from_slice(&(self.seq as u16).to_le_bytes());
        bytes.push(bits);
        debug_assert!(new_entries.len() <= MAX_PALETTE, "the length field is a u8");
        bytes.push(new_entries.len() as u8);
        for colour in new_entries {
            bytes.extend_from_slice(&colour.to_rgb().0);
        }

        // The block list, hoisted out of the payloads so like sits with like.
        if keyframe {
            debug_assert_eq!(self.staged.len(), BLOCK_COUNT, "a keyframe is every block");
        } else if bitmap {
            let mut map = [0u8; BITMAP_BYTES];
            for (block, _) in &self.staged {
                map[*block as usize / 8] |= 1 << (*block % 8);
            }
            bytes.extend_from_slice(&map);
        } else {
            bytes.extend_from_slice(&(self.staged.len() as u16).to_le_bytes());
            for (block, _) in &self.staged {
                bytes.extend_from_slice(&block.to_le_bytes());
            }
        }
        for (_, indices) in &self.staged {
            pack(indices, bits, &mut bytes);
        }

        Some(Encoded { seq: self.seq, keyframe, bytes })
    }

    /// A standalone keyframe for the state the encoder is currently in, carrying the whole
    /// palette so a decoder starting from nothing lands exactly where the encoder is.
    pub fn keyframe(&self) -> Option<Encoded> {
        if !self.sent_anything {
            return None;
        }
        let bits = bits_per_pixel(self.palette.len());
        let mut bytes =
            Vec::with_capacity(6 + self.palette.len() * 3 + BLOCK_COUNT * payload_bytes(bits));
        bytes.push(VERSION);
        bytes.push(FLAG_KEYFRAME);
        bytes.extend_from_slice(&(self.seq as u16).to_le_bytes());
        bytes.push(bits);
        debug_assert!(self.palette.len() <= MAX_PALETTE, "the length field is a u8");
        bytes.push(self.palette.len() as u8);
        for colour in &self.palette {
            bytes.extend_from_slice(&colour.to_rgb().0);
        }
        let mut indices = [0u8; BLOCK_PIXELS];
        for block in 0..BLOCK_COUNT {
            for (slot, p) in block_pixels(block).enumerate() {
                indices[slot] = self.last_sent[p];
            }
            pack(&indices, bits, &mut bytes);
        }

        Some(Encoded { seq: self.seq, keyframe: true, bytes })
    }

    fn block_changed(&self, frame: &Frame, block: usize) -> bool {
        block_pixels(block).any(|p| self.palette[self.last_sent[p] as usize] != frame[p])
    }

    /// Should this frame be spent on a fresh palette?
    fn should_reset_palette(&self, frame: &Frame, blocks: &[u16]) -> bool {
        let mut fresh: HashSet<LcdColor> = HashSet::new();
        let overflows = blocks.iter().flat_map(|&b| block_pixels(b as usize)).any(|p| {
            let colour = frame[p];
            !self.index.contains_key(&colour)
                && fresh.insert(colour)
                && self.palette.len() + fresh.len() > MAX_PALETTE
        });
        if !overflows {
            return false;
        }
        // The cold path, and the only place an O(pixels) distinct count is worth paying for.
        let mut distinct: HashSet<LcdColor> = HashSet::with_capacity(MAX_PALETTE);
        for pixel in frame.iter() {
            if distinct.insert(*pixel) && distinct.len() > MAX_PALETTE {
                return false; // a reset cannot fit this frame either, so it would only thrash
            }
        }
        true
    }

    /// Resolve one block to palette indices and queue it, if it says anything new.
    fn stage_block(&mut self, frame: &Frame, block: u16, force: bool) {
        let mut indices = [0u8; BLOCK_PIXELS];
        let mut changed = false;
        for (slot, p) in block_pixels(block as usize).enumerate() {
            let index = self.intern(frame[p]);
            indices[slot] = index;
            // Record what the decoder will hold, which is the frame itself except on the lossy
            // path.
            changed |= self.last_sent[p] != index;
            self.last_sent[p] = index;
        }
        if changed || force {
            self.staged.push((block, indices));
        }
    }

    fn intern(&mut self, colour: LcdColor) -> u8 {
        if let Some(&existing) = self.index.get(&colour) {
            return existing;
        }
        if self.palette.len() < MAX_PALETTE {
            let index = self.palette.len() as u8;
            self.palette.push(colour);
            self.index.insert(colour, index);
            return index;
        }
        // Unreachable for Pokémon Red — a full frame never carries 256 distinct colours, and
        // `should_reset_palette` spends a keyframe before it gets close.
        nearest(&self.palette, colour)
    }
}

fn block_pixels(block: usize) -> impl Iterator<Item = usize> {
    let x0 = (block % BLOCKS_X) * BLOCK;
    let y0 = (block / BLOCKS_X) * BLOCK;
    (0..BLOCK).flat_map(move |dy| (0..BLOCK).map(move |dx| (y0 + dy) * LCD_WIDTH + x0 + dx))
}

fn nearest(palette: &[LcdColor], colour: LcdColor) -> u8 {
    let [r, g, b] = colour.to_rgb().0;
    let distance = |candidate: &LcdColor| {
        let [cr, cg, cb] = candidate.to_rgb().0;
        let d = |a: u8, x: u8| (a as i32 - x as i32).pow(2);
        d(r, cr) + d(g, cg) + d(b, cb)
    };
    palette.iter().enumerate().min_by_key(|(_, c)| distance(c)).map(|(i, _)| i as u8).unwrap_or(0)
}

// ── Decoder
// ──────────────────────────────────────────────────────────────────────────────────────

/// The reference decoder: the regression net for the wire format, and the thing the TypeScript
/// decoder in the SPA is a direct port of. Every rule in the module docs is enforced here, so a
/// change to the format that forgets one fails a test rather than showing up as a corrupt canvas.
pub struct VideoDecoder {
    palette: Vec<LcdColor>,
    pixels: Box<Frame>,
    seq: Option<u16>,
}

impl Default for VideoDecoder {
    fn default() -> Self {
        Self { palette: Vec::new(), pixels: Box::new([LcdColor::default(); PIXELS]), seq: None }
    }
}

impl VideoDecoder {
    pub fn pixels(&self) -> &Frame {
        &self.pixels
    }

    pub fn apply(&mut self, message: &[u8]) -> Result<(), String> {
        let mut reader = Reader { bytes: message, at: 0 };
        let version = reader.u8()?;
        if version != VERSION {
            return Err(format!("unsupported video message version {version}"));
        }
        let flags = reader.u8()?;
        let keyframe = flags & FLAG_KEYFRAME != 0;
        let seq = reader.u16()?;
        let bits = reader.u8()?;
        if !matches!(bits, 1 | 2 | 4 | 8) {
            return Err(format!("{bits} bits per pixel is not 1, 2, 4 or 8"));
        }

        let palette_len = reader.u8()? as usize;
        if keyframe {
            self.palette.clear();
        }
        for _ in 0..palette_len {
            let [r, g, b] = [reader.u8()?, reader.u8()?, reader.u8()?];
            self.palette.push(LcdColor::rgb(r, g, b));
        }
        if self.palette.len() > 1 << bits {
            return Err(format!(
                "{} palette entries do not fit in {bits} bits",
                self.palette.len()
            ));
        }

        // A keyframe's block list is implicit: every block, in order.
        let blocks: Vec<usize> = if keyframe {
            (0..BLOCK_COUNT).collect()
        } else if flags & FLAG_BITMAP != 0 {
            let mut map = [0u8; BITMAP_BYTES];
            for byte in map.iter_mut() {
                *byte = reader.u8()?;
            }
            (0..BLOCK_COUNT).filter(|b| map[b / 8] & (1 << (b % 8)) != 0).collect()
        } else {
            let count = reader.u16()? as usize;
            let mut blocks = Vec::with_capacity(count);
            for _ in 0..count {
                let block = reader.u16()? as usize;
                if block >= BLOCK_COUNT {
                    return Err(format!("block index {block} out of range"));
                }
                blocks.push(block);
            }
            blocks
        };

        let mask = if bits == 8 { 0xFF } else { (1u16 << bits) as u8 - 1 };
        for block in blocks {
            for (slot, p) in block_pixels(block).enumerate() {
                let index = if bits == 8 {
                    reader.u8()?
                } else {
                    let per_byte = 8 / bits as usize;
                    let byte = reader.peek(slot / per_byte)?;
                    (byte >> ((slot % per_byte) * bits as usize)) & mask
                } as usize;
                self.pixels[p] = *self.palette.get(index).ok_or_else(|| {
                    format!("palette index {index} beyond {} entries", self.palette.len())
                })?;
            }
            if bits != 8 {
                reader.at += payload_bytes(bits);
            }
        }

        if reader.at != message.len() {
            return Err(format!("{} trailing bytes", message.len() - reader.at));
        }
        self.seq = Some(seq);
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8, String> {
        let byte = *self.bytes.get(self.at).ok_or("video message ended early")?;
        self.at += 1;
        Ok(byte)
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes([self.u8()?, self.u8()?]))
    }

    /// Read without advancing — the packed payload is addressed by pixel, not consumed byte by
    /// byte.
    fn peek(&self, offset: usize) -> Result<u8, String> {
        self.bytes.get(self.at + offset).copied().ok_or_else(|| "video message ended early".into())
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "slow-tests"))]
mod bench;
