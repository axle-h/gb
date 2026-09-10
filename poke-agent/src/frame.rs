//! One LCD frame and one encoded video message: the two types the emulator→UI buffers in
//! [`crate::published`] need from the video path. The codec itself lives with the server.

use gb::lcd_palette::LcdColor;
use gb::ppu::{LCD_HEIGHT, LCD_WIDTH};

pub const PIXELS: usize = LCD_WIDTH * LCD_HEIGHT;

/// One LCD frame, exactly as `gb::ppu::PPU::lcd` hands it over.
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
