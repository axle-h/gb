//! 2bpp tiles as shade indices.

/// One 8×8 tile of 2bpp Game Boy graphics.
pub const TILE_BYTES: usize = 16;

/// A `tiles_wide × tiles_high` rectangle of consecutive 2bpp tiles as shade indices, row-major.
pub fn tile_grid_shades(bytes: &[u8], tiles_wide: usize, tiles_high: usize) -> Vec<u8> {
    let width = tiles_wide * 8;
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
    tile_grid_shades(crate::gfx::sprites::POKE_BALL, 2, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let bytes = crate::gfx::sprites::POKE_BALL;
        let shades = tile_grid_shades(bytes, 2, 2);
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
