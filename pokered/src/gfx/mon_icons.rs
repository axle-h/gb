//! `engine/gfx/mon_icons.asm`: the party menu's mon icons, four OAM objects a mon out of their own
//! tile patterns, and `AnimatePartyMon`, which flips the selected one between two frames.
//!
//! The icon a species gets is `MonPartyData`, a nybble a dex number; its patterns are
//! `MonPartySpritePointers`, both read out of the cartridge rather than transcribed. Every icon but
//! one kind is left-right symmetric and drawn from two patterns with the second column X-flipped;
//! `ICON_HELIX` is drawn from four. Frame two is the same objects `ICONOFFSET` tiles on, except for
//! `ICON_BALL` and `ICON_HELIX`, which instead drop a pixel.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_symbols, DmgBank, DmgPointer};
use crate::gfx::layers::Object;
use crate::gfx::tiles::{TileData, V_CHARS0};
use crate::systems::hp_bar::HpBarColour;

/// `ICON_BALL << 2` and `ICON_HELIX << 2`, the base tiles of the two icons that bob.
const ICON_BALL: u8 = 1 << 2;
const ICON_HELIX: u8 = 2 << 2;
/// `ICONOFFSET`: how far frame two's patterns sit from frame one's.
const ICONOFFSET: u8 = 0x40;
/// `MonPartySpritePointers`' entries, each six bytes.
const ICON_POINTERS: usize = 0x1C;
const V_SPRITES: u16 = 0x8000;
/// `OBJ_SIZE * 4 * PARTY_LENGTH`, in objects: what `wMonPartySpritesSavedOAM` holds.
pub const PARTY_OBJECTS: usize = 4 * 6;
/// `wShadowOAM`'s forty objects.
const OAM_OBJECTS: usize = 40;
/// `WriteMonPartySpriteOAM`'s top-left, with the hardware's offsets: `x = 16`, `y = 16 + 16n`.
const ICON_X: u8 = 16;
const ICON_Y: u8 = 16;

/// `LoadMonPartySpriteGfx`, and the LCD-off twin whose frames are loading.
pub fn load_mon_party_sprite_gfx(tiles: &mut TileData) {
    let table = rom_slice(pokered_symbols::MonPartySpritePointers);
    for entry in table.chunks_exact(6).take(ICON_POINTERS) {
        let source = DmgPointer { bank: DmgBank::ROM { bank: entry[3] }, address: u16::from_le_bytes([entry[0], entry[1]]) };
        let count = entry[2] as usize;
        let destination = (u16::from_le_bytes([entry[4], entry[5]]) - V_SPRITES) as usize / TILE_BYTES;
        tiles.load(V_CHARS0 + destination, &rom_slice(source)[..count * TILE_BYTES]);
    }
}

/// `GetPartyMonSpriteID`: the icon's base tile, `ICON_* << 2`. An odd dex number reads the high
/// nybble of its byte and an even one the low.
pub fn icon_tile(species: PokemonSpecies) -> u8 {
    let dex = species.metadata().pokedex_number;
    let byte = rom_slice(pokered_symbols::MonPartyData)[(dex as usize - 1) / 2];
    let nybble = if dex & 1 != 0 { byte >> 4 } else { byte & 0x0F };
    nybble << 2
}

/// `ClearSprites`.
pub fn clear_sprites(sprites: &mut Vec<Object>) {
    sprites.clear();
    sprites.resize(OAM_OBJECTS, Object::default());
}

/// `WriteMonPartySpriteOAMByPartyIndex`: slot `n`'s four objects, frame one, at OAM `4n`.
pub fn write_mon_party_sprite_oam(sprites: &mut Vec<Object>, slot: usize, species: PokemonSpecies) {
    if sprites.len() < OAM_OBJECTS {
        sprites.resize(OAM_OBJECTS, Object::default());
    }
    let base = icon_tile(species);
    let top = ICON_Y + 16 * slot as u8;
    for (i, object) in sprites[4 * slot..4 * slot + 4].iter_mut().enumerate() {
        let (row, column) = ((i / 2) as u8, (i % 2) as u8);
        *object = if base == ICON_HELIX {
            // `WriteAsymmetricMonPartySpriteOAM`: four patterns, nothing flipped.
            Object { y: top + 8 * row, x: ICON_X + 8 * column, tile: base + i as u8, attributes: 0 }
        } else {
            // `WriteSymmetricMonPartySpriteOAM`: a pattern a row, the right column its mirror.
            let attributes = if column == 1 { Object::X_FLIP } else { 0 };
            Object { y: top + 8 * row, x: ICON_X + 8 * column, tile: base + 2 * row, attributes }
        };
    }
}

/// `AnimatePartyMon`, one pass: `wAnimCounter` counts to twice the speed and the selected mon
/// shows frame two for its second half. At zero every icon goes back to frame one, which is
/// `wMonPartySpritesSavedOAM` copied back; the icons the party would write are the same objects.
///
/// `sgb` is `wOnSGB`, which the speed is one frame shorter for.
pub fn animate_party_mon(sprites: &mut Vec<Object>, counter: &mut u8, current: u8, colour: HpBarColour,
                         party: &[PokemonSpecies], sgb: bool) {
    let speed = colour.animation_speed() + !sgb as u8;
    if *counter == 0 {
        for (slot, &species) in party.iter().enumerate() {
            write_mon_party_sprite_oam(sprites, slot, species);
        }
    } else if *counter == speed && (current as usize) < party.len() {
        let objects = &mut sprites[4 * current as usize..4 * current as usize + 4];
        if matches!(objects[0].tile, ICON_BALL | ICON_HELIX) {
            objects.iter_mut().for_each(|object| object.y += 1);
        } else {
            objects.iter_mut().for_each(|object| object.tile += ICONOFFSET);
        }
    }
    *counter += 1;
    if *counter == 2 * speed {
        *counter = 0;
    }
}

#[cfg(test)]
mod tests {
    use crate::gfx::tiles::TileData;
    use super::*;

    #[test]
    fn a_species_gets_the_icon_its_nybble_names() {
        assert_eq!(icon_tile(PokemonSpecies::Bulbasaur), 7 << 2, "ICON_GRASS, an odd number's high nybble");
        assert_eq!(icon_tile(PokemonSpecies::Ivysaur), 7 << 2, "and an even number's low one");
        assert_eq!(icon_tile(PokemonSpecies::Charmander), 0, "ICON_MON");
        assert_eq!(icon_tile(PokemonSpecies::Pidgey), 4 << 2, "ICON_BIRD");
        assert_eq!(icon_tile(PokemonSpecies::Voltorb), ICON_BALL);
        assert_eq!(icon_tile(PokemonSpecies::Omanyte), ICON_HELIX);
        assert_eq!(icon_tile(PokemonSpecies::Mew), 0, "ICON_MON, the last entry");
    }

    #[test]
    fn a_symmetric_icon_mirrors_its_right_column() {
        let mut sprites = Vec::new();
        write_mon_party_sprite_oam(&mut sprites, 2, PokemonSpecies::Pidgey);
        assert_eq!(sprites.len(), 40);
        let bird = 4 << 2;
        assert_eq!(sprites[8..12], [
            Object { y: 48, x: 16, tile: bird, attributes: 0 },
            Object { y: 48, x: 24, tile: bird, attributes: Object::X_FLIP },
            Object { y: 56, x: 16, tile: bird + 2, attributes: 0 },
            Object { y: 56, x: 24, tile: bird + 2, attributes: Object::X_FLIP },
        ]);
    }

    #[test]
    fn the_helix_is_four_patterns_and_bobs_rather_than_changing() {
        let mut sprites = Vec::new();
        write_mon_party_sprite_oam(&mut sprites, 0, PokemonSpecies::Kabuto);
        assert_eq!(sprites[..4].iter().map(|o| o.tile).collect::<Vec<_>>(), [8, 9, 10, 11]);
        assert!(sprites[..4].iter().all(|o| o.attributes == 0));
        let mut counter = 6;
        animate_party_mon(&mut sprites, &mut counter, 0, HpBarColour::Green, &[PokemonSpecies::Kabuto], false);
        assert_eq!(sprites[..4].iter().map(|o| (o.y, o.tile)).collect::<Vec<_>>(), [(17, 8), (17, 9), (25, 10), (25, 11)]);
    }

    /// Green is five passes a frame, and a DMG adds one.
    #[test]
    fn the_selected_mon_flips_for_the_second_half_of_its_count() {
        let party = [PokemonSpecies::Pidgey, PokemonSpecies::Rattata];
        let mut sprites = Vec::new();
        let mut counter = 0;
        let mut tiles = Vec::new();
        for _ in 0..24 {
            animate_party_mon(&mut sprites, &mut counter, 1, HpBarColour::Green, &party, false);
            tiles.push(sprites[4].tile);
        }
        let quadruped = 9 << 2;
        let frame = |second: bool| if second { quadruped + ICONOFFSET } else { quadruped };
        let expected: Vec<u8> = (0..24).map(|pass| frame(pass % 12 >= 6)).collect();
        assert_eq!(tiles, expected);
        assert_eq!(sprites[0].tile, 4 << 2, "the other mon never moves");
    }

    #[test]
    fn every_icon_pattern_lands_in_the_sprite_tiles() {
        let mut tiles = TileData::default();
        load_mon_party_sprite_gfx(&mut tiles);
        let blank = [0; TILE_BYTES];
        for base in [0u8, ICON_BALL, 3 << 2, 4 << 2, 5 << 2, 6 << 2, 7 << 2, 8 << 2, 9 << 2] {
            assert_ne!(*tiles.obj(base), blank, "frame one at ${base:02X}");
            assert_ne!(*tiles.obj(base + ICONOFFSET), blank, "frame two at ${:02X}", base + ICONOFFSET);
        }
    }
}
