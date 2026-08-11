//! Reading graphics straight out of the cartridge the binary already carries.
//!
//! The primitives [`crate::pokemon::badge_gfx`], [`crate::pokemon::mon_gfx`] and the favicon all
//! sit on: where a ROM pointer's bytes actually are, and how to turn a rectangle of 2bpp tiles into
//! shade indices. Nothing here knows what any of it looks like — colour is the caller's business,
//! which is why `src/web/` is where the palettes live.

use crate::pokemon::roms::POKERED;
use crate::pokemon::symbols::{DmgBank, DmgPointer};

/// One 8×8 tile of 2bpp Game Boy graphics.
pub const TILE_BYTES: usize = 16;
/// Every bank but 0 is windowed here.
const BANK_SIZE: usize = 0x4000;

/// A ROM pointer as a slice running to the end of its bank.
///
/// ⚠️ **Bank 0 lives at the bottom of the address space; every other bank is *windowed* at
/// `0x4000`**, so its addresses need the window subtracted before they are offsets. Getting that
/// wrong for a bank-0 pointer reads 16 KB past the data and produces a plausible-looking sprite of
/// something else entirely.
pub fn rom_slice(pointer: DmgPointer) -> &'static [u8] {
    let DmgBank::ROM { bank } = pointer.bank else {
        panic!("{pointer} is not a ROM pointer");
    };
    let bank = bank as usize;
    let window = if bank == 0 { 0 } else { BANK_SIZE };
    &POKERED[bank * BANK_SIZE + (pointer.address as usize - window)..(bank + 1) * BANK_SIZE]
}

/// A `tiles_wide × tiles_high` rectangle of consecutive 2bpp tiles, as shade indices `0` (lightest)
/// to `3` (darkest), row-major over the whole rectangle.
///
/// ⚠️ **Tiles are taken in row-major order** — left to right, then down — which is what `rgbgfx`
/// emits unless pokered's Makefile passes it `--columns`, and it does not for either the badge sheet
/// or the overworld sprites. The *decompressed* Pokémon pics are the exception and are column-major;
/// see [`crate::pokemon::mon_gfx`].
pub fn tile_grid_shades(first_tile: DmgPointer, tiles_wide: usize, tiles_high: usize) -> Vec<u8> {
    let width = tiles_wide * 8;
    let bytes = rom_slice(first_tile);
    let mut shades = vec![0u8; width * tiles_high * 8];
    for tile in 0..tiles_wide * tiles_high {
        let (left, top) = ((tile % tiles_wide) * 8, (tile / tiles_wide) * 8);
        let tile = &bytes[tile * TILE_BYTES..(tile + 1) * TILE_BYTES];
        for y in 0..8 {
            let (low, high) = (tile[y * 2], tile[y * 2 + 1]);
            for x in 0..8 {
                let bit = 7 - x;
                shades[(top + y) * width + left + x] = ((high >> bit) & 1) << 1 | ((low >> bit) & 1);
            }
        }
    }
    shades
}

/// The overworld Poké Ball — the sprite an item lying on the floor is drawn with, 2×2 uncompressed
/// tiles. It is the favicon, for want of anything else on this cartridge that says "Pokémon" in
/// sixteen pixels and is not a logo somebody owns.
pub const BALL_PX: usize = 16;

pub fn poke_ball_shades() -> Vec<u8> {
    tile_grid_shades(crate::pokemon::symbols::pokered_symbols::PokeBallSprite, 2, 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::symbols::pokered_symbols;

    /// Bank 0 is the case the windowing gets wrong. Nothing in the crate reads a bank-0 pointer
    /// today, so this checks the contract rather than a caller: a bank-0 address is a raw file
    /// offset, and the same address in bank 1 is 16 KB further on than the naive arithmetic says.
    #[test]
    fn bank_zero_is_not_windowed_and_every_other_bank_is() {
        let bank_0 = DmgPointer { bank: DmgBank::ROM { bank: 0 }, address: 0x0100 };
        assert_eq!(rom_slice(bank_0)[..16], POKERED[0x0100..0x0110]);

        let bank_1 = DmgPointer { bank: DmgBank::ROM { bank: 1 }, address: 0x4100 };
        assert_eq!(rom_slice(bank_1)[..16], POKERED[0x4100..0x4110]);

        let bank_9 = DmgPointer { bank: DmgBank::ROM { bank: 9 }, address: 0x4000 };
        assert_eq!(rom_slice(bank_9)[..16], POKERED[9 * BANK_SIZE..9 * BANK_SIZE + 16]);
    }

    /// The ball is line art on a transparent background: mostly-empty and entirely-full both mean
    /// the window has slipped onto the wrong tiles.
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

        // A ball is round: the four corners of its box are background.
        for (x, y) in [(0, 0), (BALL_PX - 1, 0), (0, BALL_PX - 1), (BALL_PX - 1, BALL_PX - 1)] {
            assert_eq!(shades[y * BALL_PX + x], 0, "({x}, {y}) should be outside the ball");
        }
    }

    /// The 2×2 assembly, checked without trusting the decoder: quadrant `n` must be tile `n`, in
    /// reading order. Column-major would put the top-right quadrant where the bottom-left goes,
    /// which on a symmetrical sprite like this one is invisible.
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
