use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use pokered::gfx::sgb::OverworldPalette;
use crate::pokemon::symbols::pokered_symbols;
use super::Oracle;

/// `SetPal_Overworld` builds its packet from `PalPacket_Empty` and patches one byte of it, so what
/// it chose is `wPalPacket + 1` when it returns.
fn set_pal_overworld(oracle: &mut Oracle, input: OverworldPalette) -> u8 {
    oracle.write(pokered_symbols::wCurMap, &[input.map as u8]);
    oracle.write(pokered_symbols::wCurMapTileset, &[input.tileset as u8]);
    oracle.write(pokered_symbols::wLastMap, &[input.last_map as u8]);
    let called = oracle.call(pokered_symbols::SetPal_Overworld);
    assert!(called.rng.is_empty());
    oracle.read(pokered_symbols::wPalPacket, 2)[1]
}

/// `DeterminePaletteIDOutOfBattle` takes its species index in `a` and answers there too.
#[cfg(feature = "slow-tests")]
fn determine_palette_id(oracle: &mut Oracle, species: u8) -> u8 {
    oracle.registers_mut().a = species;
    oracle.call(pokered_symbols::DeterminePaletteIDOutOfBattle);
    oracle.registers().a
}

/// Every map, in the three tilesets that change the answer, against a `wLastMap` that cycles
/// through a town, a route, a city and the plateau.
#[cfg(feature = "slow-tests")]
fn overworld_inputs() -> Vec<OverworldPalette> {
    use strum::IntoEnumIterator;
    const LAST_MAPS: [Map; 4] = [Map::PalletTown, Map::Route1, Map::CeladonCity, Map::IndigoPlateau];
    const TILESETS: [TileSetId; 3] = [TileSetId::Overworld, TileSetId::Cavern, TileSetId::Cemetery];
    Map::iter()
        .enumerate()
        .flat_map(|(i, map)| TILESETS.map(move |tileset| OverworldPalette {
            map,
            tileset,
            last_map: LAST_MAPS[i % LAST_MAPS.len()],
        }))
        .collect()
}

/// `NUM_POKEMON_INDEXES`, which is where `PokedexOrder` ends.
#[cfg(feature = "slow-tests")]
const NUM_POKEMON_INDEXES: u8 = 190;

/// The oracle's own check, on the two cases the routine's shape turns on: a building takes the
/// palette of the map it is in, and a cave takes its own whatever map that is.
#[test]
fn an_indoor_map_answers_for_the_map_it_is_in() {
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let input = OverworldPalette {
        map: Map::RedsHouse1F,
        tileset: TileSetId::RedsHouse1,
        last_map: Map::PalletTown,
    };
    assert_eq!(set_pal_overworld(&mut oracle, input), 0x01, "PAL_PALLET");
    let cave = OverworldPalette { tileset: TileSetId::Cavern, ..input };
    assert_eq!(set_pal_overworld(&mut oracle, cave), 0x23, "PAL_CAVE");
}

#[test]
#[cfg(feature = "slow-tests")]
fn the_port_matches_the_cartridge() {
    use pokered::gfx::sgb::{determine_palette_id_out_of_battle, overworld_palette};
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    for input in overworld_inputs() {
        assert_eq!(overworld_palette(input), set_pal_overworld(&mut oracle, input), "{input:?}");
    }
    for species in 0..=NUM_POKEMON_INDEXES {
        assert_eq!(determine_palette_id_out_of_battle(species), determine_palette_id(&mut oracle, species),
                   "species {species:#04X}");
    }
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/palettes/ under GB_REGEN_FIXTURES=1"]
fn harvest_palettes() {
    use super::{write_fixture, Case};
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let overworld: Vec<Case<OverworldPalette, u8>> = overworld_inputs().into_iter()
        .map(|input| Case { input, output: set_pal_overworld(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("palettes", "set_pal_overworld", &overworld);
    let species: Vec<Case<u8, u8>> = (0..=NUM_POKEMON_INDEXES)
        .map(|input| Case { input, output: determine_palette_id(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("palettes", "determine_palette_id", &species);
}
