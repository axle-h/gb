//! Pictures. A pic is the tiles of an `n`x`n` square, row-major as the build generates every
//! asset, and the cartridge places it bottom-centred in a 7x7-tile buffer before drawing it.

use crate::gfx::MON_PICS;
use crate::species::PokemonSpecies;

/// The sprite buffer's side, in tiles; every picture returned is this size.
pub const PIC_TILES: usize = 7;
pub const PIC_PX: usize = PIC_TILES * 8;
const TILE_BYTES: usize = 16;
/// A back pic's side, in tiles.
const BACK_PIC_TILES: usize = 4;

/// The species' front pic, as tiles.
pub fn front_pic(species: PokemonSpecies) -> &'static [u8] {
    MON_PICS[species.metadata().pokedex_number as usize - 1].0
}

/// The species' back pic, as tiles.
pub fn back_pic(species: PokemonSpecies) -> &'static [u8] {
    MON_PICS[species.metadata().pokedex_number as usize - 1].1
}

/// A pic's side in tiles. `pkmncompress` derives a `.pic`'s dimension byte, which `base_stats`
/// `INCBIN`s as `wMonHSpriteDim`, from the tile count the same way, and refuses a count that is
/// not a square.
pub fn pic_side(tiles: &[u8]) -> usize {
    let count = tiles.len() / TILE_BYTES;
    (1..=PIC_TILES)
        .find(|side| side * side == count && tiles.len() % TILE_BYTES == 0)
        .unwrap_or_else(|| panic!("{} bytes are not a square pic of at most 7x7 tiles", tiles.len()))
}

/// One Pokémon's front sprite as shade indices, row-major; the palette is the caller's.
pub fn front_pic_shades(species: PokemonSpecies) -> [u8; PIC_PX * PIC_PX] {
    pic_shades(front_pic(species))
}

/// `AlignSpriteDataCentered`: any pic in the sprite buffer, centred horizontally with the odd
/// column to the right, and standing on the bottom.
pub fn pic_shades(tiles: &[u8]) -> [u8; PIC_PX * PIC_PX] {
    let side = pic_side(tiles);
    let (left, top) = ((PIC_TILES + 1 - side) / 2 * 8, (PIC_TILES - side) * 8);
    let mut shades = [0u8; PIC_PX * PIC_PX];
    for y in 0..side * 8 {
        for x in 0..side * 8 {
            shades[(top + y) * PIC_PX + left + x] = pixel(tiles, side, x, y);
        }
    }
    shades
}

/// `ScaleSpriteByTwo`: the top left 28 pixels square of a 4x4 back pic, each pixel doubled, fills
/// the buffer.
pub fn scaled_back_pic_shades(tiles: &[u8]) -> [u8; PIC_PX * PIC_PX] {
    assert_eq!(pic_side(tiles), BACK_PIC_TILES, "a back pic is 4x4 tiles");
    let mut shades = [0u8; PIC_PX * PIC_PX];
    for y in 0..PIC_PX {
        for x in 0..PIC_PX {
            shades[y * PIC_PX + x] = pixel(tiles, BACK_PIC_TILES, x / 2, y / 2);
        }
    }
    shades
}

/// The shade at `(x, y)` of a `side`-tile square of row-major 2bpp tiles.
fn pixel(tiles: &[u8], side: usize, x: usize, y: usize) -> u8 {
    let row = &tiles[((y / 8) * side + x / 8) * TILE_BYTES + (y % 8) * 2..][..2];
    let bit = 7 - x % 8;
    ((row[1] >> bit) & 1) << 1 | ((row[0] >> bit) & 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    /// FNV-1a.
    fn checksum(shades: &[u8; PIC_PX * PIC_PX]) -> u32 {
        let mut hash = 0x811C_9DC5u32;
        for &shade in shades.iter() {
            hash = (hash ^ shade as u32).wrapping_mul(0x0100_0193);
        }
        hash
    }

    /// Every front pic placed, as a ROM-free stand-in for `rom_equality`'s comparison with the
    /// cartridge; regenerate only while that is green.
    #[test]
    fn every_front_pic_matches_its_committed_checksum() {
        let expected = include_bytes!("data/gfx/front_pic_checksums.bin");
        assert_eq!(expected.len(), 151 * 4, "one u32 per species, in Pokédex order");
        for species in PokemonSpecies::iter() {
            let dex = species.metadata().pokedex_number as usize;
            let at = (dex - 1) * 4;
            let want = u32::from_le_bytes(expected[at..at + 4].try_into().unwrap());
            assert_eq!(checksum(&front_pic_shades(species)), want, "{species} (#{dex})");
        }
    }

    /// Writes `src/data/gfx/front_pic_checksums.bin`.
    #[test]
    #[ignore = "a tool: writes the checksum fixture"]
    #[cfg(feature = "slow-tests")]
    fn dump_front_pic_checksums() {
        let mut bytes = vec![0u8; 151 * 4];
        for species in PokemonSpecies::iter() {
            let at = (species.metadata().pokedex_number as usize - 1) * 4;
            bytes[at..at + 4].copy_from_slice(&checksum(&front_pic_shades(species)).to_le_bytes());
        }
        std::fs::write("src/data/gfx/front_pic_checksums.bin", &bytes).expect("write the fixture");
        println!("wrote {} bytes of front-pic checksums", bytes.len());
    }

    /// Every species decodes to a distinct, drawn sprite.
    #[test]
    fn every_species_decodes_to_a_distinct_drawn_sprite() {
        let sprites: Vec<_> = PokemonSpecies::iter().map(|s| (s, front_pic_shades(s))).collect();
        assert_eq!(sprites.len(), 151);

        for (species, shades) in &sprites {
            let mut used = shades.to_vec();
            used.sort_unstable();
            used.dedup();
            assert!(used.len() >= 3, "{species} uses only {} shades: {used:?}", used.len());

            let drawn = shades.iter().filter(|&&s| s != 0).count();
            assert!(
                (PIC_PX * 4..PIC_PX * PIC_PX * 9 / 10).contains(&drawn),
                "{species} has {drawn} non-background pixels of {}",
                PIC_PX * PIC_PX,
            );
        }

        for (index, (species, first)) in sprites.iter().enumerate() {
            for (other, second) in sprites.iter().skip(index + 1) {
                assert_ne!(first, second, "{species} and {other} decoded identically");
            }
        }
    }

    #[test]
    fn sprites_are_centred_horizontally_and_stand_on_the_bottom() {
        let mut reached_the_floor = 0;
        for species in PokemonSpecies::iter() {
            let shades = front_pic_shades(species);
            let side = pic_side(front_pic(species));
            let left = (PIC_TILES + 1 - side) / 2;

            for y in 0..PIC_PX {
                for x in 0..PIC_PX {
                    let inside = (left * 8..(left + side) * 8).contains(&x) && y >= (PIC_TILES - side) * 8;
                    assert!(inside || shades[y * PIC_PX + x] == 0, "{species} has ink at ({x}, {y})");
                }
            }
            if (0..PIC_PX).any(|x| shades[(PIC_PX - 1) * PIC_PX + x] != 0) {
                reached_the_floor += 1;
            }
        }
        // Bottom alignment is only observable on sprites that fill their box.
        assert!(reached_the_floor > 50, "only {reached_the_floor} sprites touch the bottom row");
    }

    /// Mew's pics come from its own entry, outside `BaseStats`, which the table puts last.
    #[test]
    fn mew_has_its_own_pics() {
        assert_eq!(front_pic(PokemonSpecies::Mew), crate::gfx::pokemon::front::MEW);
        assert_eq!(back_pic(PokemonSpecies::Mew), crate::gfx::pokemon::back::MEWB);
    }
}
