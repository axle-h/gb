//! The eight gym badges, decoded from the cartridge the binary already carries.
//!
//! `pokered`'s trainer card draws a 4×2 grid of gym-leader faces and replaces a face with its badge
//! once the badge is owned (`engine/menus/draw_badges.asm`). Both live in one blob,
//! `GymLeaderFaceAndBadgeTileGraphics`, which the game copies to `vChars2 tile $20`:
//!
//! ```text
//! blob tile  0…3   4…7   8…11  12…15  …     each group of 8 tiles is one gym:
//!            face  badge face  badge        a 2×2 face, then its 2×2 badge
//! ```
//!
//! So badge `i` is blob tiles `8i+4 … 8i+7`, laid out top-left, top-right, bottom-left,
//! bottom-right — which is exactly the order `DrawBadges.PlaceTiles` writes them in. Badge order is
//! the order of the `wObtainedBadges` bits, so index `i` here is
//! [`Badge::ORDER`](crate::pokemon::badge::Badge::ORDER)`[i]`.
//!
//! Nothing is committed: this reads the ROM at run time, the same ROM the emulator boots.

use crate::pokemon::rom_gfx::{TILE_BYTES, tile_grid_shades};
use crate::pokemon::symbols::pokered_symbols::GymLeaderFaceAndBadgeTileGraphics;

/// Badges are 2×2 tiles.
pub const BADGE_PX: usize = 16;
pub const BADGE_COUNT: usize = 8;

/// Face + badge, 2×2 tiles each.
const TILES_PER_GYM: usize = 8;

/// One badge as shade indices, row-major, `0` (lightest) to `3` (darkest) — the 2bpp values
/// themselves, not colours. What they look like is the caller's business: the trainer card runs them
/// through the Game Boy's greys, and `src/web/badges.rs` picks something that reads on a dark page.
///
/// # Panics
/// If `index >= BADGE_COUNT`.
pub fn badge_shades(index: usize) -> [u8; BADGE_PX * BADGE_PX] {
    assert!(index < BADGE_COUNT, "there are only {BADGE_COUNT} badges");
    let first_tile = index * TILES_PER_GYM + 4; // past the gym leader's face
    let shades = tile_grid_shades(GymLeaderFaceAndBadgeTileGraphics + (first_tile * TILE_BYTES) as u16, 2, 2);
    shades.try_into().expect("2×2 tiles is 16×16 pixels")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::rom_gfx::rom_slice;

    fn tile_bytes(tile: usize) -> &'static [u8] {
        let at = tile * TILE_BYTES;
        &rom_slice(GymLeaderFaceAndBadgeTileGraphics)[at..at + TILE_BYTES]
    }

    /// The offsets are arithmetic over a symbol address, and arithmetic that is one tile out still
    /// produces a plausible-looking 16×16 sprite — of half a gym leader's face. So: eight of them,
    /// all different, none blank, and each one drawn rather than a flat block.
    #[test]
    fn eight_distinct_badges_come_out_of_the_rom() {
        let badges: Vec<_> = (0..BADGE_COUNT).map(badge_shades).collect();

        for (index, shades) in badges.iter().enumerate() {
            let mut used = shades.to_vec();
            used.sort_unstable();
            used.dedup();
            assert!(used.len() >= 3, "badge {index} uses only {} shades — {used:?}", used.len());

            // A badge is a shape on a light background: mostly-dark or entirely-light both mean the
            // window has slipped onto the wrong tiles.
            let dark = shades.iter().filter(|&&s| s >= 2).count();
            assert!(
                (BADGE_PX * 2..BADGE_PX * BADGE_PX * 3 / 4).contains(&dark),
                "badge {index} has {dark} dark pixels of {}",
                BADGE_PX * BADGE_PX,
            );
        }

        for (a, first) in badges.iter().enumerate() {
            for (b, second) in badges.iter().enumerate().skip(a + 1) {
                assert_ne!(first, second, "badges {a} and {b} decoded identically");
            }
        }
    }

    /// The 2×2 assembly, checked without trusting the decoder: the four quadrants come from four
    /// consecutive tiles, so re-reading those tiles by hand must reproduce the sprite.
    #[test]
    fn the_quadrants_are_four_consecutive_tiles() {
        let shades = badge_shades(0);
        for tile in 0..4 {
            let bytes = tile_bytes(4 + tile);
            let (left, top) = ((tile % 2) * 8, (tile / 2) * 8);
            for y in 0..8 {
                let (low, high) = (bytes[y * 2], bytes[y * 2 + 1]);
                for x in 0..8 {
                    let expected = ((high >> (7 - x)) & 1) << 1 | ((low >> (7 - x)) & 1);
                    assert_eq!(shades[(top + y) * BADGE_PX + left + x], expected, "({x}, {y}) of tile {tile}");
                }
            }
        }
    }

}
