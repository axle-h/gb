//! The SGB packets, decoded where `SendSGBPacket` would bit-bang them out of the joypad port.
//!
//! Inputs: a label in `data/sgb/sgb_packets.asm`, or a copy of one the game has patched. Outputs:
//! the four palette ids a `PAL_SET` names, and the rectangles an `ATTR_BLK` colours. The packets
//! are read out of the cartridge rather than transcribed, so the `ATTR_BLK_DATA` rows are the
//! cartridge's own.

use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::DmgPointer;
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};

/// One packet. A transfer is up to seven of them.
pub const PACKET_BYTES: usize = 16;

/// A whole transfer as `SendSGBPacket` reads it: the low three bits of the first byte are how many
/// packets it is, and the other five are the command.
pub fn transfer(at: DmgPointer) -> Vec<u8> {
    let bytes = rom_slice(at);
    bytes[..(bytes[0] & 0x07) as usize * PACKET_BYTES].to_vec()
}

/// The four `SuperPalettes` ids a `PAL_SET` names. They are written as words, and no palette id
/// needs the high byte, which is why `SetPal_Overworld` patches one byte and is done.
pub fn pal_set(transfer: &[u8]) -> [u8; 4] {
    std::array::from_fn(|i| transfer[1 + i * 2])
}

/// Every `ATTR_BLK_DATA` row of an `ATTR_BLK` transfer. The count is a byte of its own, so a row
/// zeroed in place (`SetPal_TrainerCard` does this to a badge the player has not won) is still
/// counted and simply colours nothing.
pub fn attr_blk(transfer: &[u8]) -> Vec<AttrBlkData> {
    (0..transfer[1] as usize)
        .map(|i| AttrBlkData::read(&transfer[2 + i * 6..8 + i * 6]))
        .collect()
}

/// A rectangle of screen cells with a palette for the cells inside it, the cells on its edge and
/// the cells outside it, each written only if its control bit says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttrBlkData {
    control: u8,
    palettes: u8,
    x1: usize,
    y1: usize,
    x2: usize,
    y2: usize,
}

impl AttrBlkData {
    const INSIDE: u8 = 1;
    const LINE: u8 = 2;
    const OUTSIDE: u8 = 4;

    fn read(row: &[u8]) -> Self {
        Self {
            control: row[0],
            palettes: row[1],
            x1: row[2] as usize,
            y1: row[3] as usize,
            x2: row[4] as usize,
            y2: row[5] as usize,
        }
    }

    /// Writes this row into the attribute file, a palette per screen cell.
    pub fn apply(&self, attributes: &mut [u8]) {
        for row in 0..SCREEN_TILES_Y {
            for column in 0..SCREEN_TILES_X {
                let within = (self.x1..=self.x2).contains(&column) && (self.y1..=self.y2).contains(&row);
                let edge = column == self.x1 || column == self.x2 || row == self.y1 || row == self.y2;
                let region = match (within, edge) {
                    (true, true) => Self::LINE,
                    (true, false) => Self::INSIDE,
                    (false, _) => Self::OUTSIDE,
                };
                if self.control & region != 0 {
                    let shift = match region {
                        Self::INSIDE => 0,
                        Self::LINE => 2,
                        _ => 4,
                    };
                    attributes[row * SCREEN_TILES_X + column] = (self.palettes >> shift) & 3;
                }
            }
        }
    }
}
