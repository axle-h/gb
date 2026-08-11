//! **W2** — the video wire format: an 8×8 block diff with a persistent palette.
//!
//! There is no image codec here and no dependency on one. The Game Boy's LCD is 160×144 with a
//! handful of colours on screen and most of it unchanged from one frame to the next, so a
//! hand-rolled palette block diff beats anything general-purpose at a fraction of the complexity: a
//! static screen costs *nothing*.
//!
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
//!
//! ## Why it looks like this
//!
//! **v1 had three per-block modes (RLE, raw, a 4-colour sub-palette) and picked the smallest.**
//! `src/web/video/bench.rs` measured them against four minutes of real play and the answer was that
//! the cleverness was costing bytes rather than saving them. Two facts did it in:
//!
//! - **`gb serve` runs `GameBoy::dmg`, so the screen is four shades and nothing else** — the bench
//!   asserts it. v1's packed mode therefore spent 4 of its 23 bytes naming a per-block sub-palette
//!   that was always a permutation of `0,1,2,3`, and another 3 on a block index and a mode tag. The
//!   payload underneath was 16 bytes. **A third of the stream was describing the encoding rather
//!   than the picture.**
//! - **Anything downstream compresses far better than a per-block size contest does.** Variable-size
//!   blocks with embedded tags are close to the worst input you can hand an LZ77 window; a fixed
//!   16-byte payload at a fixed offset is close to the best. Measured end to end, dropping all three
//!   modes for one flat width made the *uncompressed* stream 21% smaller and the *compressed* stream
//!   **2.6×** smaller.
//!
//! So: one width for the whole message, wide enough for the palette (2 bits in practice, 8 in the
//! worst case, which is still v1's raw mode). No modes, no tags, and the block list is hoisted out
//! of the payloads so like sits with like — which is again for the compressor's benefit.
//!
//! **The palette is stream state, not message state.** A delta *appends* its `new_palette` entries
//! to whatever the decoder already has; a **keyframe replaces the palette outright** with the entries
//! it carries, which are the encoder's entire palette at that moment. That second rule is what makes
//! the late-joiner handshake work: a keyframe encoded from the current frame leaves the decoder's
//! palette *identical* to the encoder's, so every subsequent delta lines up. An earlier design had a
//! keyframe carry only the colours it needed, which quietly desynchronised every late joiner the
//! first time a delta referenced an index the keyframe had not listed.
//!
//! ⚠️ **`bits_per_pixel` is a property of the message, not of the stream.** It is whatever the
//! palette needs *after* this message's new entries are folded in, so a frame that introduces a
//! fifth colour widens from 2 bits to 4 for that message alone. Blocks already on screen are not
//! resent, and do not need to be: the decoder holds pixels, not packed bytes.
//!
//! **RGB888, not RGB565.** DMG's greys are `FF/AA/55/00` and `0xAA` does not survive a round trip
//! through 5 bits (see [`LcdColor`]). The palette is at most 255 entries, so the third byte is free.
//!
//! The encoder tracks what the *decoder* will hold, not what the frame contained — [`VideoEncoder`]
//! writes palette-resolved indices into `last_sent`. It matters only on the lossy path (a frame with
//! more than 255 distinct colours, which Pokémon Red never produces), but without it a block that
//! was approximated once would read as unchanged forever after.
//!
//! ## What goes on the wire
//!
//! Nothing here base64s anything. `src/web/mod.rs` streams these messages as binary, length-prefixed
//! and deflated per connection — see that module for why, and for the measurement that base64 costs
//! **twice** as much after compression as the 33% it costs before.

use std::collections::{HashMap, HashSet};

use crate::lcd_palette::LcdColor;
use crate::ppu::{LCD_HEIGHT, LCD_WIDTH};

/// Block edge, in pixels. 8 divides both 160 and 144 exactly, which is why 8 and not 16.
pub const BLOCK: usize = 8;
pub const BLOCKS_X: usize = LCD_WIDTH / BLOCK;
pub const BLOCKS_Y: usize = LCD_HEIGHT / BLOCK;
pub const BLOCK_COUNT: usize = BLOCKS_X * BLOCKS_Y;
pub const PIXELS: usize = LCD_WIDTH * LCD_HEIGHT;
const BLOCK_PIXELS: usize = BLOCK * BLOCK;

pub const VERSION: u8 = 2;
const FLAG_KEYFRAME: u8 = 0x01;
const FLAG_BITMAP: u8 = 0x02;
/// One bit per block, so a message that touches most of the screen names them all in 45 bytes
/// instead of 720. The alternative — a `u16` each — wins below 22 blocks, and the encoder picks.
const BITMAP_BYTES: usize = BLOCK_COUNT.div_ceil(8);
const BITMAP_WORTH_IT: usize = (BITMAP_BYTES - 2) / 2 + 1;
/// ⚠️ **255, not 256, and the `u8` index is not the reason.** `new_palette_len` is a `u8`, and a
/// keyframe has to carry the *whole* palette in one message — so a 256th entry would encode its
/// length as `0` and the decoder would silently read the block list as palette bytes. Losing one
/// index out of 256 costs nothing on a path Pokémon Red never reaches; a length field that wraps to
/// zero costs the stream.
const MAX_PALETTE: usize = 255;

/// One LCD frame, exactly as [`crate::ppu::PPU::lcd`] hands it over.
pub type Frame = [LcdColor; PIXELS];

/// A message ready to go on the wire, with the bookkeeping the streaming route needs to order it
/// against a keyframe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// Monotonic and **not** wrapped — the wire field is `u16`, but a late joiner compares sequence
    /// numbers to decide what to discard, and that comparison is wrong across a `u16` wrap (~36
    /// minutes at 30 fps).
    pub seq: u64,
    pub keyframe: bool,
    pub bytes: Vec<u8>,
}

/// How wide an index into a palette of `entries` has to be. Rounded up to a power of two so a
/// payload is a whole number of pixels per byte, which is what keeps packing and unpacking a shift
/// rather than a division.
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

// ── Encoder ──────────────────────────────────────────────────────────────────────────────────────

pub struct VideoEncoder {
    palette: Vec<LcdColor>,
    index: HashMap<LcdColor, u8>,
    /// What the decoder holds after everything emitted so far, **as palette indices** — see the
    /// module docs. Indices rather than colours so [`VideoEncoder::keyframe`] is a straight pack:
    /// holding colours meant one hash lookup per pixel, 23040 of them, every published frame.
    last_sent: Box<[u8; PIXELS]>,
    sent_anything: bool,
    seq: u64,
    /// Scratch for one message's block payloads, kept across calls so a 30 fps stream allocates
    /// nothing per frame. `(block index, indices)`.
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
    ///
    /// For `POST /api/new-run`, which replaces the emulator's state outright. Deltas are encoded
    /// against `last_sent`, so without this the first frame of the new game would be diffed against
    /// the last frame of the old one and every block that happened to match would simply not be
    /// sent — leaving fragments of the abandoned run on screen with nothing to repair them.
    ///
    /// ⚠️ **`seq` is deliberately kept.** It is not part of what the decoder holds, it is the
    /// ordering every connected client filters on (`/api/video` drops anything at or below the seq
    /// it opened with), so restarting the count at zero would make a live viewer discard the whole
    /// new run. `VideoEncoder::default()` is therefore *not* a substitute for this.
    pub fn restart(&mut self) {
        self.palette.clear();
        self.index.clear();
        self.last_sent = Box::new([0; PIXELS]);
        self.sent_anything = false;
    }

    /// Encode `frame` against everything sent so far.
    ///
    /// `None` when nothing the decoder can see changed — the common case for an idle screen and the
    /// reason a standing-still emulator costs no bandwidth at all. Roughly half of all published
    /// frames come back `None` in ordinary play. The sequence number only advances when a message is
    /// actually produced, so a stored keyframe stays valid across idle ticks.
    ///
    /// "Nothing the decoder can see" rather than "nothing changed" is the precise rule, and on the
    /// lossy path they differ: a block whose true colours moved but whose *approximation* did not
    /// carries no information, and emitting it would mean re-sending an unchanged screen forever.
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

        // Two passes, and the split is forced: `bits_per_pixel` covers the palette *including* the
        // entries these blocks are about to introduce, so nothing can be written until they have all
        // been interned.
        let palette_base = self.palette.len();
        self.staged.clear();
        for &block in &blocks {
            // A keyframe carries all 360 blocks by definition — that is what makes it standalone.
            self.stage_block(frame, block, keyframe);
        }
        if self.staged.is_empty() {
            // Every candidate resolved to what the decoder already holds. Only reachable on the lossy
            // path, and only reachable at all *because* the candidates were chosen by comparing the
            // source against the approximation. Roll the palette back: an entry that was interned but
            // never sent would be referenced by a later message the decoder cannot resolve.
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

    /// A standalone keyframe for the state the encoder is currently in, carrying the **whole**
    /// palette so a decoder starting from nothing lands exactly where the encoder is.
    ///
    /// Pure: it neither advances the sequence number nor touches the palette, so the emulator thread
    /// can hand one to a joiner without racing the encoder. Returns `None` before anything has been
    /// encoded, when there is no state to describe.
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

    /// Should this frame be spent on a fresh palette? Decided before anything is written, so the
    /// answer is taken once rather than discovered mid-frame.
    ///
    /// Two conditions, and the second is easy to forget: the changed blocks must need more palette
    /// than is left, **and** a fresh palette must actually be able to hold the whole frame. Without
    /// the second, a frame carrying more than [`MAX_PALETTE`] distinct colours resets, approximates,
    /// reads as changed again on the next tick because the approximation is not the source, and
    /// resets again — a full keyframe every tick, forever, for a screen nobody is touching.
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
    ///
    /// A block is queued when its palette-resolved form differs from what the decoder holds — not
    /// when the *source* differs, which is a weaker thing that is only equivalent off the lossy
    /// path. `force` is how a keyframe gets all 360 blocks regardless.
    fn stage_block(&mut self, frame: &Frame, block: u16, force: bool) {
        let mut indices = [0u8; BLOCK_PIXELS];
        let mut changed = false;
        for (slot, p) in block_pixels(block as usize).enumerate() {
            let index = self.intern(frame[p]);
            indices[slot] = index;
            // Record what the decoder will hold, which is the frame itself except on the lossy path.
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
        // `should_reset_palette` spends a keyframe before it gets close. Approximating beats
        // failing: a slightly wrong pixel is a better outcome for a livestream than a dropped stream.
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

// ── Decoder ──────────────────────────────────────────────────────────────────────────────────────

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

    /// The `frame_seq` of the last message applied, as it appeared on the wire.
    pub fn seq(&self) -> Option<u16> {
        self.seq
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

        // A keyframe's block list is implicit: every block, in order. That is what makes it
        // standalone, and it is also 45 bytes and a validation case that cannot go wrong.
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

    /// Read without advancing — the packed payload is addressed by pixel, not consumed byte by byte.
    fn peek(&self, offset: usize) -> Result<u8, String> {
        self.bytes.get(self.at + offset).copied().ok_or_else(|| "video message ended early".into())
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "bench"))]
mod bench;
