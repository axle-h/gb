//! **W2** — the video wire format: an 8×8 block diff with a persistent palette.
//!
//! There is no image codec here and no dependency on one. The Game Boy's LCD is 160×144 with a few
//! dozen colours on screen and long horizontal runs of identical pixels, so a hand-rolled
//! palette + RLE block diff beats anything general-purpose at a fraction of the complexity: a static
//! screen costs *nothing*, and a walking animation touches 40–80 blocks at ~15 bytes each.
//!
//! ```text
//! u8   version (=1)
//! u8   flags            bit0 = keyframe
//! u16  frame_seq        wrapping, little-endian
//! u8   new_palette_len
//! [new_palette_len × u24 RGB]
//! u16  block_count
//! [block_count × {
//!     u16 block_index                  0..360, row-major
//!     u8  mode                         0 = RLE, 1 = raw (64 bytes), 2 = 2-bit packed (20 bytes)
//!     payload
//! }]
//! ```
//!
//! **Mode 2 exists because modes 0 and 1 were both worse than they looked.** The first draft assumed
//! Game Boy blocks were "extremely run-friendly, ~15 bytes RLE each"; measured against the game
//! walking around outdoors they average **39.7**. A tile is 8 pixels wide and detailed ones — grass,
//! trees, interior clutter — change colour every two or three, so runs are ~3.5 pixels and a
//! scrolling screen is close to the worst case for run-length coding at 1:1. Packing 2 bits per pixel
//! against a four-entry per-block sub-palette costs a flat 20 bytes and fits DMG exactly, since DMG
//! has exactly four shades. It is not a replacement for the other two — the encoder still picks
//! whichever of the three is smallest per block, and on a flat block RLE still wins at 4 bytes.
//!
//! **The palette is stream state, not message state.** A delta *appends* its `new_palette` entries
//! to whatever the decoder already has; a **keyframe replaces the palette outright** with the entries
//! it carries, which are the encoder's entire palette at that moment. That second rule is what makes
//! §5.2's late-joiner handshake work: a keyframe encoded from the current frame leaves the decoder's
//! palette *identical* to the encoder's, so every subsequent delta lines up. An earlier design had a
//! keyframe carry only the colours it needed, which quietly desynchronised every late joiner the
//! first time a delta referenced an index the keyframe had not listed.
//!
//! **RGB888, not RGB565.** DMG's greys are `FF/AA/55/00` and `0xAA` does not survive a round trip
//! through 5 bits (see [`LcdColor`]). The palette is at most 256 entries, so the third byte is free.
//!
//! The encoder tracks what the *decoder* will hold, not what the frame contained — [`VideoEncoder`]
//! writes palette-resolved colours back into `last_sent`. It matters only on the lossy path (a frame
//! with more than 256 distinct colours, which Pokémon Red never produces), but without it a block
//! that was approximated once would read as unchanged forever after.

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

pub const VERSION: u8 = 1;
const FLAG_KEYFRAME: u8 = 0x01;
const MODE_RLE: u8 = 0;
const MODE_RAW: u8 = 1;
const MODE_PACKED: u8 = 2;
/// Sub-palette size for [`MODE_PACKED`]: 2 bits per pixel. Not a coincidence — DMG renders four
/// shades, so almost every block on screen fits.
const PACKED_COLOURS: usize = 4;
const PACKED_BYTES: usize = BLOCK_PIXELS / 4;
/// ⚠️ **255, not 256, and the `u8` index is not the reason.** `new_palette_len` is a `u8`, and a
/// keyframe has to carry the *whole* palette in one message — so a 256th entry would encode its
/// length as `0` and the decoder would silently read the block list as palette bytes. Losing one
/// index out of 256 costs nothing on a path Pokémon Red never reaches; a length field that wraps to
/// zero costs the stream.
const MAX_PALETTE: usize = 255;

/// One LCD frame, exactly as [`crate::ppu::PPU::lcd`] hands it over.
pub type Frame = [LcdColor; PIXELS];

/// A message ready to go on the wire, with the bookkeeping the SSE route needs to order it against
/// a keyframe (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// Monotonic and **not** wrapped — the wire field is `u16`, but a late joiner compares sequence
    /// numbers to decide what to discard, and that comparison is wrong across a `u16` wrap (~36
    /// minutes at 30 fps).
    pub seq: u64,
    pub keyframe: bool,
    pub bytes: Vec<u8>,
}

// ── Encoder ──────────────────────────────────────────────────────────────────────────────────────

pub struct VideoEncoder {
    palette: Vec<LcdColor>,
    index: HashMap<LcdColor, u8>,
    /// What the decoder holds after everything emitted so far — see the module docs.
    last_sent: Box<Frame>,
    sent_anything: bool,
    seq: u64,
}

impl Default for VideoEncoder {
    fn default() -> Self {
        Self {
            palette: Vec::new(),
            index: HashMap::new(),
            last_sent: Box::new([LcdColor::default(); PIXELS]),
            sent_anything: false,
            seq: 0,
        }
    }
}

impl VideoEncoder {
    /// The sequence number of the most recent [`Self::encode`] that produced a message.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Encode `frame` against everything sent so far.
    ///
    /// `None` when nothing the decoder can see changed — the common case for an idle screen and the
    /// reason a standing-still emulator costs no bandwidth at all. The sequence number only advances
    /// when a message is actually produced, so a stored keyframe stays valid across idle ticks.
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

        // Block payloads first: they are what decides which palette entries the message carries, so
        // the header cannot be written until they are done.
        let palette_base = self.palette.len();
        let mut body = Vec::with_capacity(blocks.len() * 24);
        let mut emitted = 0;
        for &block in &blocks {
            // A keyframe carries all 360 blocks by definition — that is what makes it standalone.
            if self.encode_block(frame, block as usize, keyframe, &mut body) {
                emitted += 1;
            }
        }
        if emitted == 0 {
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

        let new_entries = &self.palette[palette_base..];
        let mut bytes = Vec::with_capacity(8 + new_entries.len() * 3 + body.len());
        bytes.push(VERSION);
        bytes.push(if keyframe { FLAG_KEYFRAME } else { 0 });
        bytes.extend_from_slice(&(self.seq as u16).to_le_bytes());
        debug_assert!(new_entries.len() <= MAX_PALETTE, "the length field is a u8");
        bytes.push(new_entries.len() as u8);
        for colour in new_entries {
            bytes.extend_from_slice(&colour.to_rgb().0);
        }
        bytes.extend_from_slice(&(emitted as u16).to_le_bytes());
        bytes.extend_from_slice(&body);

        Some(Encoded { seq: self.seq, keyframe, bytes })
    }

    /// A standalone keyframe for the state the encoder is currently in, carrying the **whole**
    /// palette so a decoder starting from nothing lands exactly where the encoder is.
    ///
    /// Pure: it neither advances the sequence number nor touches the palette, so the emulator thread
    /// can publish one beside every delta and a late joiner picks it up without racing the encoder.
    /// Returns `None` before anything has been encoded, when there is no state to describe.
    pub fn keyframe(&self) -> Option<Encoded> {
        if !self.sent_anything {
            return None;
        }
        let mut body = Vec::with_capacity(BLOCK_COUNT * 24);
        for block in 0..BLOCK_COUNT {
            body.extend_from_slice(&(block as u16).to_le_bytes());
            let indices = self.block_indices(block);
            append_payload(&indices, &mut body);
        }

        let mut bytes = Vec::with_capacity(8 + self.palette.len() * 3 + body.len());
        bytes.push(VERSION);
        bytes.push(FLAG_KEYFRAME);
        bytes.extend_from_slice(&(self.seq as u16).to_le_bytes());
        debug_assert!(self.palette.len() <= MAX_PALETTE, "the length field is a u8");
        bytes.push(self.palette.len() as u8);
        for colour in &self.palette {
            bytes.extend_from_slice(&colour.to_rgb().0);
        }
        bytes.extend_from_slice(&(BLOCK_COUNT as u16).to_le_bytes());
        bytes.extend_from_slice(&body);

        Some(Encoded { seq: self.seq, keyframe: true, bytes })
    }

    fn block_changed(&self, frame: &Frame, block: usize) -> bool {
        block_pixels(block).any(|p| frame[p] != self.last_sent[p])
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

    /// Encode one block, returning whether it was written.
    ///
    /// A block is written when its palette-resolved form differs from what the decoder holds — not
    /// when the *source* differs, which is a weaker thing that is only equivalent off the lossy path.
    /// `force` is how a keyframe gets all 360 blocks regardless, which is what makes it standalone.
    fn encode_block(&mut self, frame: &Frame, block: usize, force: bool, out: &mut Vec<u8>) -> bool {
        let mut indices = [0u8; BLOCK_PIXELS];
        let mut changed = false;
        for (slot, p) in block_pixels(block).enumerate() {
            let index = self.intern(frame[p]);
            indices[slot] = index;
            // Record what the decoder will hold, which is the frame itself except on the lossy path.
            let resolved = self.palette[index as usize];
            changed |= self.last_sent[p] != resolved;
            self.last_sent[p] = resolved;
        }
        if !(changed || force) {
            return false;
        }
        out.extend_from_slice(&(block as u16).to_le_bytes());
        append_payload(&indices, out);
        true
    }

    /// The palette indices for a block of `last_sent`, i.e. of what the decoder already holds. Only
    /// [`Self::keyframe`] needs this, and it is why keyframing costs no palette churn.
    fn block_indices(&self, block: usize) -> [u8; BLOCK_PIXELS] {
        let mut indices = [0u8; BLOCK_PIXELS];
        for (slot, p) in block_pixels(block).enumerate() {
            indices[slot] = self.index.get(&self.last_sent[p]).copied().unwrap_or(0);
        }
        indices
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
        // `would_overflow` spends a keyframe before it gets close. Approximating beats failing: a
        // slightly wrong pixel is a better outcome for a livestream than a dropped stream.
        nearest(&self.palette, colour)
    }
}

/// Whichever of the three modes is smallest for this block.
///
/// A flat block is 2 bytes of RLE and nothing beats that; a four-shade detailed block is 20 bytes
/// packed against ~40 RLE; a block with five or more colours cannot pack at all and falls back to
/// raw's flat 64. Deciding per block rather than per frame matters because a single frame routinely
/// contains all three shapes — a scrolling route has flat sky, detailed grass, and the sprite
/// overlaps that bring in a fifth colour.
fn append_payload(indices: &[u8; BLOCK_PIXELS], out: &mut Vec<u8>) {
    let mut rle: Vec<u8> = Vec::with_capacity(16);
    let mut run_start = 0;
    for slot in 1..=BLOCK_PIXELS {
        if slot == BLOCK_PIXELS || indices[slot] != indices[run_start] {
            rle.push((slot - run_start) as u8);
            rle.push(indices[run_start]);
            run_start = slot;
        }
    }

    // The sub-palette is built in first-appearance order, so it is deterministic and the decoder
    // needs no sorting rule. More than four distinct indices and the block simply cannot pack.
    let mut sub: Vec<u8> = Vec::with_capacity(PACKED_COLOURS);
    for index in indices {
        if !sub.contains(index) {
            if sub.len() == PACKED_COLOURS {
                sub.clear();
                break;
            }
            sub.push(*index);
        }
    }
    let packed_len = if sub.is_empty() { usize::MAX } else { PACKED_COLOURS + PACKED_BYTES };

    if rle.len() <= packed_len.min(BLOCK_PIXELS) {
        out.push(MODE_RLE);
        out.extend_from_slice(&rle);
    } else if packed_len <= BLOCK_PIXELS {
        out.push(MODE_PACKED);
        // Padded to four so the payload is a fixed size the decoder can read without a count. A
        // repeat of entry 0 is harmless: nothing indexes it.
        let mut entries = [sub[0]; PACKED_COLOURS];
        entries[..sub.len()].copy_from_slice(&sub);
        out.extend_from_slice(&entries);
        for chunk in indices.chunks(4) {
            let bits = chunk.iter().enumerate().fold(0u8, |acc, (i, index)| {
                let slot = sub.iter().position(|s| s == index).expect("built from these indices");
                acc | ((slot as u8) << (i * 2))
            });
            out.push(bits);
        }
    } else {
        out.push(MODE_RAW);
        out.extend_from_slice(indices);
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

        let palette_len = reader.u8()? as usize;
        if keyframe {
            self.palette.clear();
        }
        for _ in 0..palette_len {
            let [r, g, b] = [reader.u8()?, reader.u8()?, reader.u8()?];
            self.palette.push(LcdColor::rgb(r, g, b));
        }

        let block_count = reader.u16()? as usize;
        if keyframe && block_count != BLOCK_COUNT {
            return Err(format!("keyframe carries {block_count} blocks, expected {BLOCK_COUNT}"));
        }
        for _ in 0..block_count {
            let block = reader.u16()? as usize;
            if block >= BLOCK_COUNT {
                return Err(format!("block index {block} out of range"));
            }
            let mut indices = [0u8; BLOCK_PIXELS];
            match reader.u8()? {
                MODE_RLE => {
                    let mut slot = 0;
                    while slot < BLOCK_PIXELS {
                        let run = reader.u8()? as usize;
                        let index = reader.u8()?;
                        if run == 0 || slot + run > BLOCK_PIXELS {
                            return Err(format!("run of {run} overruns block {block}"));
                        }
                        indices[slot..slot + run].fill(index);
                        slot += run;
                    }
                }
                MODE_RAW => {
                    for slot in indices.iter_mut() {
                        *slot = reader.u8()?;
                    }
                }
                MODE_PACKED => {
                    let mut sub = [0u8; PACKED_COLOURS];
                    for entry in sub.iter_mut() {
                        *entry = reader.u8()?;
                    }
                    for chunk in indices.chunks_mut(4) {
                        let bits = reader.u8()?;
                        for (i, slot) in chunk.iter_mut().enumerate() {
                            *slot = sub[((bits >> (i * 2)) & 0b11) as usize];
                        }
                    }
                }
                other => return Err(format!("unknown block mode {other}")),
            }
            for (slot, p) in block_pixels(block).enumerate() {
                let index = indices[slot] as usize;
                self.pixels[p] = *self.palette.get(index)
                    .ok_or_else(|| format!("palette index {index} beyond {} entries", self.palette.len()))?;
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
}

#[cfg(test)]
mod tests;
