//! The SGB border: `gfx/sgb/red_border.{2bpp,tilemap}` and `BorderPalettes`, the picture the
//! console draws round the game.
//!
//! Input: the 160x144 RGBA the colour mode painted. Output: 256x224 RGBA with that picture in the
//! middle of it. Exact: the tilemap, the tiles and the three palettes `LoadSGB` transfers.
//!
//! The border is an image and not a palette, which is why it is an option of its own.

use poke_core::gfx::sgb::{RED_BORDER, RED_BORDER_TILEMAP};
use poke_core::gfx::SGB_BORDER_PALETTES;
use crate::gfx::colour::{rgb555, BYTES_PER_PIXEL};
use crate::gfx::compose;
use crate::gfx::tiles::pixel;

const TILEMAP_WIDTH: usize = 32;
const TILEMAP_HEIGHT: usize = 28;
pub const WIDTH: usize = TILEMAP_WIDTH * 8;
pub const HEIGHT: usize = TILEMAP_HEIGHT * 8;

/// Where the game's own screen sits: the middle of it.
const INNER_X: usize = (WIDTH - compose::WIDTH) / 2;
const INNER_Y: usize = (HEIGHT - compose::HEIGHT) / 2;

/// SNES palettes 4 to 6, sixteen colours each, of which the border uses the first four: the tiles
/// are 2bpp converted by `CopySGBBorderTiles`, which zeroes the two high planes.
const FIRST_PALETTE: usize = 4;

/// The border round `inner`, which is `compose::WIDTH` by `compose::HEIGHT` RGBA.
pub fn rgba(inner: &[u8]) -> Vec<u8> {
    let (data, tiles) = (RED_BORDER_TILEMAP, RED_BORDER);
    let mut out = vec![0u8; WIDTH * HEIGHT * BYTES_PER_PIXEL];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let cell = (y / 8) * TILEMAP_WIDTH + x / 8;
            let entry = u16::from_le_bytes([data[cell * 2], data[cell * 2 + 1]]);
            let (tile, palette) = ((entry & 0x3FF) as usize, (entry >> 10) as usize & 7);
            let (mut tx, mut ty) = (x % 8, y % 8);
            if entry & 0x4000 != 0 { tx = 7 - tx; }
            if entry & 0x8000 != 0 { ty = 7 - ty; }
            // Only the picture's 96 tiles, not the 128 `CopySGBBorderTiles` copies from the bank.
            let bytes: &[u8; 16] = tiles[tile * 16..tile * 16 + 16].try_into().unwrap();
            let colour = pixel(bytes, tx, ty);
            let inside = (INNER_X..INNER_X + compose::WIDTH).contains(&x)
                && (INNER_Y..INNER_Y + compose::HEIGHT).contains(&y);
            let at = (y * WIDTH + x) * BYTES_PER_PIXEL;
            // Colour 0 is the SNES's transparency: the game shows through it in the middle, and
            // the backdrop, which is colour 0 of the first border palette, shows through outside.
            let span = at..at + BYTES_PER_PIXEL;
            match (colour, inside) {
                (0, true) => {
                    let from = ((y - INNER_Y) * compose::WIDTH + x - INNER_X) * BYTES_PER_PIXEL;
                    out[span].copy_from_slice(&inner[from..from + BYTES_PER_PIXEL]);
                }
                _ => out[span].copy_from_slice(&rgb555(border_palette(palette)[colour as usize])),
            }
        }
    }
    out
}

/// One of the three palettes after the tilemap in the `PCT_TRN` data, addressed as the tilemap
/// addresses it: SNES palettes 4, 5 and 6.
fn border_palette(palette: usize) -> [u16; 4] {
    SGB_BORDER_PALETTES[palette.saturating_sub(FIRST_PALETTE)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `red_border.2bpp` is 96 tiles and `CopySGBBorderTiles` copies 128, so on the cartridge the
    /// last 32 are whatever follows it in the bank. Nothing draws them.
    #[test]
    fn the_tilemap_only_names_tiles_the_border_graphics_have() {
        let data = RED_BORDER_TILEMAP;
        assert_eq!(RED_BORDER.len(), 96 * 16);
        let highest = (0..TILEMAP_WIDTH * TILEMAP_HEIGHT)
            .map(|cell| u16::from_le_bytes([data[cell * 2], data[cell * 2 + 1]]) & 0x3FF)
            .max()
            .unwrap();
        assert!(highest < 96, "tile {highest} is past the 96 in red_border.2bpp");
    }

    #[test]
    fn the_middle_is_the_game_and_the_edge_is_the_border() {
        let inner = vec![0x40u8; compose::WIDTH * compose::HEIGHT * BYTES_PER_PIXEL];
        let out = rgba(&inner);
        assert_eq!(out.len(), WIDTH * HEIGHT * BYTES_PER_PIXEL);
        let at = |x: usize, y: usize| {
            let i = (y * WIDTH + x) * BYTES_PER_PIXEL;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        assert_eq!(at(INNER_X, INNER_Y), [0x40; 4], "the game's top-left corner");
        assert_eq!(at(WIDTH / 2, HEIGHT / 2), [0x40; 4], "and its middle");
        assert_eq!(at(0, 0)[3], 0xFF, "the border is opaque");
        assert_ne!(at(0, 0), [0x40; 4], "the border's own corner");
    }
}
