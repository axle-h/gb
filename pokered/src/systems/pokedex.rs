//! What the Pokédex screens compute rather than draw: the two flag arrays' counts, the order table
//! both number conversions read, and one dex entry decoded out of the cartridge.
//!
//! A *dex number* is what the player sees, 1 to 151; an *index* is the cartridge's own species id,
//! 1 to 190, with 39 of them belonging to no species. `PokedexOrder` maps index to dex number and
//! is searched backwards for the other direction.

use poke_core::mon_gfx::{front_pic_shades, PIC_PX, PIC_TILES};
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_symbols, DmgBank, DmgPointer};

/// `NUM_POKEMON`.
pub const NUM_POKEMON: u8 = 151;
/// `NUM_POKEMON_INDEXES`: longer than the dex, because the table names the indexes no species has
/// with a dex number of 0.
pub const NUM_POKEMON_INDEXES: usize = 190;
/// `wPokedexSeenEnd - wPokedexSeen`, which is also `wPokedexOwned`'s length: 152 bits for 151 mons.
pub const FLAG_BYTES: usize = 19;

/// `CountSetBits`, whose running total is one byte: 256 flags set would count as none.
pub fn count_set_bits(flags: &[u8]) -> u8 {
    flags.iter().fold(0u8, |count, byte| (0..8).fold(count, |c, bit| c.wrapping_add(byte >> bit & 1)))
}

/// `PokedexOrder`: the dex number of every index, from index 1.
fn pokedex_order() -> &'static [u8] {
    &rom_slice(pokered_symbols::PokedexOrder)[..NUM_POKEMON_INDEXES]
}

/// `PokedexToIndex`. The cartridge walks the table until it matches and has no exit if nothing
/// does, so a dex number out of range never returns there; here it is a panic rather than a wrong
/// answer. Dex number 0 matches the first MISSINGNO. entry, as it does on the cartridge.
pub fn pokedex_to_index(dex: u8) -> u8 {
    pokedex_order().iter().position(|&number| number == dex)
        .map(|index| index as u8 + 1)
        .unwrap_or_else(|| panic!("no index has Pokédex number {dex}"))
}

/// `IndexToPokedex`, which is also `BaseStats::of(species).dex` for an index a species has.
pub fn index_to_pokedex(index: u8) -> u8 {
    pokedex_order()[index as usize - 1]
}

/// The species a dex number names, or `None` for one of the table's MISSINGNO. entries.
pub fn species_of(dex: u8) -> Option<PokemonSpecies> {
    PokemonSpecies::from_repr(pokedex_to_index(dex))
}

/// Bit `dex - 1` of a flag array, which is how both arrays are indexed.
pub fn is_set(flags: &[u8], dex: u8) -> bool {
    flags[(dex as usize - 1) / 8] >> ((dex - 1) % 8) & 1 != 0
}

/// `.maxSeenPokemonLoop`: the highest dex number seen, found by walking the flags from the end with
/// a counter that starts one past the last bit. Nothing seen walks off the front of the array on
/// the cartridge, which is a memory-layout glitch rather than behaviour, so it answers 0 here.
pub fn max_seen_mon(seen: &[u8]) -> u8 {
    let mut counter = (FLAG_BYTES * 8 + 1) as u8;
    for &byte in seen.iter().rev() {
        let mut bits = byte;
        for _ in 0..8 {
            counter -= 1;
            let set = bits & 0x80 != 0;
            bits <<= 1;
            if set {
                return counter;
            }
        }
    }
    0
}

/// `TX_FAR`, the command a dex entry ends in.
const TX_FAR: u8 = 0x17;
/// `@`, which ends the species line — and which a description does not carry.
const TERMINATOR: u8 = 0x50;

/// One row of `PokedexEntryPointers`: what the data screen prints besides the picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexEntry {
    /// The species line, charmap bytes without its `@`: `DRILL`, `SEED` and so on.
    pub species: Vec<u8>,
    pub feet: u8,
    pub inches: u8,
    /// Tenths of a pound, as the entry stores it, little-endian on the cartridge.
    pub weight: u16,
    /// The description as charmap bytes, ending in the `<DEXEND>` that stops the printer.
    pub description: Vec<u8>,
}

impl DexEntry {
    /// The entry for an index, which is what `PokedexEntryPointers` is indexed by.
    pub fn of(index: u8) -> Self {
        let table = rom_slice(pokered_symbols::PokedexEntryPointers);
        let row = (index as usize - 1) * 2;
        let address = u16::from_le_bytes([table[row], table[row + 1]]);
        let at = DmgPointer { bank: pokered_symbols::PokedexEntryPointers.bank, address };
        let bytes = rom_slice(at);
        let end = bytes.iter().position(|&byte| byte == TERMINATOR).expect("a dex entry's species line ends");
        let far = end + 5;
        assert_eq!(bytes[far], TX_FAR, "a dex entry ends in a text_far");
        let script = DmgPointer {
            bank: DmgBank::ROM { bank: bytes[far + 3] },
            address: u16::from_le_bytes([bytes[far + 1], bytes[far + 2]]),
        };
        Self {
            species: bytes[..end].to_vec(),
            feet: bytes[end + 1],
            inches: bytes[end + 2],
            weight: u16::from_le_bytes([bytes[end + 3], bytes[end + 4]]),
            description: description(script),
        }
    }
}

/// The description's printable run. A dex description is the one text in the cartridge with no
/// `text_end`: `PlaceDexEnd` ends the whole script from inside the printed run, so the bytes past
/// its `<DEXEND>` are the next entry's. `text_script` stops there for that reason.
fn description(script: DmgPointer) -> Vec<u8> {
    let commands = poke_core::text_script::decode_slice(rom_slice(script), script)
        .expect("a dex description is a text script");
    commands.into_iter().fold(Vec::new(), |mut run, command| {
        if let poke_core::text_script::TextCommand::Text(text) = command {
            run.extend(text);
        }
        run
    })
}

/// The 49 tiles `LoadFrontSpriteByMonIndex` leaves in `vFrontPic`, in the order it writes them:
/// seven down the first column, then the next. `flipped` is `wSpriteFlipped`, which reverses the
/// pixels within every byte; laying the columns out right to left is the caller's half of the flip.
pub fn front_pic_tiles(species: PokemonSpecies, flipped: bool) -> Vec<[u8; TILE_BYTES]> {
    pic_tiles(&front_pic_shades(species), flipped)
}

/// Any picture's shades as those 49 tiles.
pub fn pic_tiles(shades: &[u8; PIC_PX * PIC_PX], flipped: bool) -> Vec<[u8; TILE_BYTES]> {
    let mut tiles = vec![[0u8; TILE_BYTES]; PIC_TILES * PIC_TILES];
    for column in 0..PIC_TILES {
        for row in 0..PIC_TILES {
            let tile = &mut tiles[column * PIC_TILES + row];
            for y in 0..8 {
                for x in 0..8 {
                    let shade = shades[(row * 8 + y) * PIC_PX + column * 8 + x];
                    let bit = if flipped { x } else { 7 - x };
                    tile[y * 2] |= (shade & 1) << bit;
                    tile[y * 2 + 1] |= (shade >> 1) << bit;
                }
            }
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;
    use crate::fixtures::cases;
    use super::*;

    #[test]
    fn count_set_bits_as_harvested() {
        for (flags, count, _) in cases::<Vec<u8>, u8>(include_str!("../../fixtures/pokedex/count_set_bits.jsonl")) {
            assert_eq!(count_set_bits(&flags), count, "{flags:02X?}");
        }
    }

    #[test]
    fn pokedex_to_index_as_harvested() {
        for (dex, index, _) in cases::<u8, u8>(include_str!("../../fixtures/pokedex/pokedex_to_index.jsonl")) {
            assert_eq!(pokedex_to_index(dex), index, "dex {dex}");
        }
    }

    /// The two conversions invert each other, and the dex number is the one the base stats carry.
    #[test]
    fn every_species_converts_both_ways() {
        for species in PokemonSpecies::iter() {
            let dex = poke_core::base_stats::BaseStats::of(species).dex;
            assert_eq!(index_to_pokedex(species as u8), dex, "{species}");
            assert_eq!(pokedex_to_index(dex), species as u8, "{species}");
            assert_eq!(species_of(dex), Some(species));
        }
    }

    #[test]
    fn the_highest_seen_is_the_last_bit_set() {
        let mut seen = [0u8; FLAG_BYTES];
        assert_eq!(max_seen_mon(&seen), 0, "nothing seen");
        seen[0] = 1;
        assert_eq!(max_seen_mon(&seen), 1, "bit 0 is dex 1");
        seen[18] = 1 << 6;
        assert_eq!(max_seen_mon(&seen), 151, "the last real one");
        seen[18] = 1 << 7;
        assert_eq!(max_seen_mon(&seen), 152, "and the flags hold one bit more than the dex does");
    }

    /// The first entry of the table and one with a two-digit height, read out of the cartridge.
    #[test]
    fn a_dex_entry_is_a_species_line_a_height_and_a_weight() {
        let rhydon = DexEntry::of(PokemonSpecies::Rhydon as u8);
        assert_eq!(rhydon.species, poke_core::charmap::encode("DRILL").unwrap());
        assert_eq!((rhydon.feet, rhydon.inches, rhydon.weight), (6, 3, 2650));
        let onix = DexEntry::of(PokemonSpecies::Onix as u8);
        assert_eq!((onix.feet, onix.inches), (28, 10), "a height that does not fit one digit");
    }

    /// Every species' entry decodes, and every description ends where the printer stops.
    #[test]
    fn every_dex_entry_decodes() {
        // `<DEXEND>`: the decoder stops on it and keeps it in the run it hands back.
        const DEX_END: u8 = 0x5F;
        for species in PokemonSpecies::iter() {
            let entry = DexEntry::of(species as u8);
            assert!(!entry.species.is_empty(), "{species} has no species line");
            assert_eq!(entry.description.last(), Some(&DEX_END), "{species}'s description");
            assert!(entry.description.len() > 20, "{species}'s description is {} bytes", entry.description.len());
        }
    }

    /// Flipping is bit order within a byte, so the two pictures are mirror images tile by tile.
    #[test]
    fn a_flipped_picture_is_the_same_tiles_with_their_bytes_reversed() {
        let plain = front_pic_tiles(PokemonSpecies::Bulbasaur, false);
        let flipped = front_pic_tiles(PokemonSpecies::Bulbasaur, true);
        assert_eq!(plain.len(), 49);
        for (a, b) in plain.iter().zip(&flipped) {
            assert_eq!(a.map(u8::reverse_bits), *b);
        }
    }
}
