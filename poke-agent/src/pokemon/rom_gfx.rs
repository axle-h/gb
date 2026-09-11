//! Reading graphics straight out of the cartridge the binary carries.

use crate::pokemon::roms::POKERED;
use crate::pokemon::symbols::{DmgBank, DmgPointer};

/// One 8×8 tile of 2bpp Game Boy graphics.
pub const TILE_BYTES: usize = 16;
const BANK_SIZE: usize = 0x4000;

/// A ROM pointer as a slice running to the end of its bank. Bank 0 is a raw file offset and every
/// other bank a `0x4000` window; read ROM through here rather than redoing that arithmetic.
pub fn rom_slice(pointer: DmgPointer) -> &'static [u8] {
    let DmgBank::ROM { bank } = pointer.bank else {
        panic!("{pointer} is not a ROM pointer");
    };
    let bank = bank as usize;
    let window = if bank == 0 { 0 } else { BANK_SIZE };
    &POKERED[bank * BANK_SIZE + (pointer.address as usize - window)..(bank + 1) * BANK_SIZE]
}

/// A `tiles_wide × tiles_high` rectangle of consecutive 2bpp tiles as shade indices, row-major.
pub fn tile_grid_shades(first_tile: DmgPointer, tiles_wide: usize, tiles_high: usize) -> Vec<u8> {
    let width = tiles_wide * 8;
    let bytes = rom_slice(first_tile);
    let mut shades = vec![0u8; width * tiles_high * 8];
    for tile in 0..tiles_wide * tiles_high {
        let (left, top) = ((tile % tiles_wide) * 8, (tile / tiles_wide) * 8);
        let pixels = decode_tile(&bytes[tile * TILE_BYTES..(tile + 1) * TILE_BYTES]);
        for y in 0..8 {
            shades[(top + y) * width + left..(top + y) * width + left + 8]
                .copy_from_slice(&pixels[y * 8..y * 8 + 8]);
        }
    }
    shades
}

/// One 8×8 tile of 2bpp as shade indices `0` (lightest) to `3` (darkest), row-major.
pub fn decode_tile(tile: &[u8]) -> [u8; 64] {
    assert_eq!(tile.len(), TILE_BYTES, "a 2bpp tile is {TILE_BYTES} bytes");
    let mut pixels = [0u8; 64];
    for y in 0..8 {
        let (low, high) = (tile[y * 2], tile[y * 2 + 1]);
        for x in 0..8 {
            let bit = 7 - x;
            pixels[y * 8 + x] = ((high >> bit) & 1) << 1 | ((low >> bit) & 1);
        }
    }
    pixels
}

/// The overworld Poké Ball an item on the floor is drawn with, 2×2 uncompressed tiles: the favicon.
pub const BALL_PX: usize = 16;

pub fn poke_ball_shades() -> Vec<u8> {
    tile_grid_shades(crate::pokemon::symbols::pokered_symbols::PokeBallSprite, 2, 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::symbols::pokered_symbols;

    /// Bank 0 is not windowed and every other bank is.
    #[test]
    fn bank_zero_is_not_windowed_and_every_other_bank_is() {
        let bank_0 = DmgPointer { bank: DmgBank::ROM { bank: 0 }, address: 0x0100 };
        assert_eq!(rom_slice(bank_0)[..16], POKERED[0x0100..0x0110]);

        let bank_1 = DmgPointer { bank: DmgBank::ROM { bank: 1 }, address: 0x4100 };
        assert_eq!(rom_slice(bank_1)[..16], POKERED[0x4100..0x4110]);

        let bank_9 = DmgPointer { bank: DmgBank::ROM { bank: 9 }, address: 0x4000 };
        assert_eq!(rom_slice(bank_9)[..16], POKERED[9 * BANK_SIZE..9 * BANK_SIZE + 16]);
    }

    /// The ball is a round drawing, neither mostly empty nor full.
    #[test]
    fn the_poke_ball_is_a_drawn_sixteen_pixel_sprite() {
        let shades = poke_ball_shades();
        assert_eq!(shades.len(), BALL_PX * BALL_PX);

        let mut used = shades.clone();
        used.sort_unstable();
        used.dedup();
        assert!(used.len() >= 3, "the ball uses only {} shades — {used:?}", used.len());

        let drawn = shades.iter().filter(|&&s| s != 0).count();
        assert!((32..192).contains(&drawn), "{drawn} of 256 pixels are drawn");

        for (x, y) in [(0, 0), (BALL_PX - 1, 0), (0, BALL_PX - 1), (BALL_PX - 1, BALL_PX - 1)] {
            assert_eq!(shades[y * BALL_PX + x], 0, "({x}, {y}) should be outside the ball");
        }
    }

    /// Quadrant `n` is tile `n` in reading order, decoded here by hand.
    #[test]
    fn quadrants_are_four_consecutive_tiles_in_reading_order() {
        let shades = tile_grid_shades(pokered_symbols::PokeBallSprite, 2, 2);
        let bytes = rom_slice(pokered_symbols::PokeBallSprite);
        for tile in 0..4 {
            let (left, top) = ((tile % 2) * 8, (tile / 2) * 8);
            for y in 0..8 {
                let (low, high) = (bytes[tile * TILE_BYTES + y * 2], bytes[tile * TILE_BYTES + y * 2 + 1]);
                for x in 0..8 {
                    let expected = ((high >> (7 - x)) & 1) << 1 | ((low >> (7 - x)) & 1);
                    assert_eq!(shades[(top + y) * BALL_PX + left + x], expected, "({x}, {y}) of tile {tile}");
                }
            }
        }
    }
}
