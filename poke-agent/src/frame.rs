//! One LCD frame and one encoded video message, the two video types [`crate::published`] needs.

use gb::lcd_palette::LcdColor;
use gb::ppu::{LCD_HEIGHT, LCD_WIDTH};

pub const PIXELS: usize = LCD_WIDTH * LCD_HEIGHT;

/// One LCD frame, exactly as `gb::ppu::PPU::lcd` hands it over.
pub type Frame = [LcdColor; PIXELS];

/// A message ready for the wire, with what the stream needs to order it against a keyframe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// Unlike the wire's `u16`, never wraps: a late joiner compares seqs to decide what to discard.
    pub seq: u64,
    pub keyframe: bool,
    pub bytes: Vec<u8>,
}
