//! `CheckForHiddenEvent` and the lookups the hidden objects make: `HiddenEventMaps`, the hidden
//! items and coins, the bookshelves, the bench guys, the gym statues and the Silph Co card key
//! doors.

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::{pokered_symbols, DmgBank, DmgPointer};
use serde::{Deserialize, Serialize};

const END: u8 = 0xFF;

/// A row of a map's `HiddenEventsFor_*`: where it is, the byte its function is handed, and the
/// function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiddenEvent {
    pub y: u8,
    pub x: u8,
    /// `wHiddenEventFunctionArgument`: an item, a facing, a text id, a trash can.
    pub argument: u8,
    /// `wHiddenEventFunctionRomBank` and the address `hl` holds.
    pub function: DmgPointer,
    /// How far `wHiddenEventIndex` moved: the rows passed over before this one.
    pub skipped: u8,
}

/// Every hidden event on `map`, in the table's order, or `None` for a map not in `HiddenEventMaps`.
pub fn hidden_events(map: Map) -> Option<Vec<HiddenEvent>> {
    let maps = rom_slice(pokered_symbols::HiddenEventMaps);
    let index = maps.iter().take_while(|&&m| m != END).position(|&m| m == map as u8)?;
    let pointers = rom_slice(pokered_symbols::HiddenEventPointers + 2 * index as u16);
    let table = DmgPointer { bank: pokered_symbols::HiddenEventPointers.bank, address: u16::from_le_bytes([pointers[0], pointers[1]]) };
    let rows = rom_slice(table);
    Some(rows.chunks(6).take_while(|row| row[0] != END).enumerate().map(|(i, row)| HiddenEvent {
        y: row[0],
        x: row[1],
        argument: row[2],
        function: DmgPointer { bank: DmgBank::ROM { bank: row[3] }, address: u16::from_le_bytes([row[4], row[5]]) },
        skipped: i as u8,
    }).collect())
}

/// `CheckIfCoordsInFrontOfPlayerMatch`'s square. Any facing that is not up, left or right is down.
pub fn in_front(x: u8, y: u8, facing: u8) -> (u8, u8) {
    match SpriteFacing::from_repr(facing) {
        Some(SpriteFacing::Up) => (x, y.wrapping_sub(1)),
        Some(SpriteFacing::Left) => (x.wrapping_sub(1), y),
        Some(SpriteFacing::Right) => (x.wrapping_add(1), y),
        _ => (x, y.wrapping_add(1)),
    }
}

/// `CheckForHiddenEvent`: the first row on the square in front of the player, whichever way they
/// face; the facing a row names is its function's to check, if it checks at all.
pub fn check_for_hidden_event(map: Map, x: u8, y: u8, facing: u8) -> Option<HiddenEvent> {
    let (fx, fy) = in_front(x, y, facing);
    hidden_events(map)?.into_iter().find(|event| event.y == fy && event.x == fx)
}

/// `FindHiddenItemOrCoinsIndex` over `HiddenItemCoords` or `HiddenCoinCoords`: the row of the map
/// and square, or `$ff` when there is none.
fn find_hidden_item_or_coins_index(table: DmgPointer, map: Map, x: u8, y: u8) -> u8 {
    rom_slice(table).chunks(3).take_while(|row| row[0] != END)
        .position(|row| row[0] == map as u8 && row[1] == y && row[2] == x)
        .map_or(END, |i| i as u8)
}

pub fn hidden_item_index(map: Map, x: u8, y: u8) -> u8 {
    find_hidden_item_or_coins_index(pokered_symbols::HiddenItemCoords, map, x, y)
}

pub fn hidden_coin_index(map: Map, x: u8, y: u8) -> u8 {
    find_hidden_item_or_coins_index(pokered_symbols::HiddenCoinCoords, map, x, y)
}

/// `HiddenCoins`' amount, BCD, from its argument less `COIN`. Forty is a typo in the cartridge that
/// gives twenty, and anything it does not name is a hundred.
pub fn hidden_coins_amount(argument: u8) -> [u8; 2] {
    match argument.wrapping_sub(poke_core::item::ItemId::Coin as u8) {
        10 => [0x00, 0x10],
        20 | 40 => [0x00, 0x20],
        _ => [0x01, 0x00],
    }
}

/// `PrintBookshelfText`'s lookup: the text predef of the bookshelf, statue or poster whose tile is
/// in front of a player facing up, by tileset.
pub fn bookshelf_text(tileset: TileSetId, tile: u8) -> Option<u8> {
    rom_slice(pokered_symbols::BookshelfTileIDs).chunks(3).take_while(|row| row[0] != END)
        .find(|row| row[0] == tileset as u8 && row[1] == tile)
        .map(|row| row[2])
}

/// `PrintBenchGuyText`'s lookup. A facing that does not match is not stepped past, so the scan goes
/// on a byte out of step and reads every third byte after it as a map, into whatever follows the
/// table. It reads on to the end of the bank here; the cartridge, running out of ROM, reads VRAM.
pub fn bench_guy_text(map: Map, facing: u8) -> Option<u8> {
    let bytes = rom_slice(pokered_symbols::BenchGuyTextPointers);
    let mut i = 0;
    while let Some(&m) = bytes.get(i) {
        i += 1;
        if m == END {
            return None;
        }
        if m != map as u8 {
            i += 2;
            continue;
        }
        let &wanted = bytes.get(i)?;
        i += 1;
        if wanted == facing {
            return bytes.get(i).copied();
        }
    }
    None
}

/// `GymStatues`' `MapBadgeFlags`: the badge bit of a gym, which `wBeatGymFlags` is compared against.
pub fn gym_badge(map: Map) -> Option<u8> {
    rom_slice(pokered_symbols::MapBadgeFlags).chunks(2).take_while(|row| row[0] != END)
        .find(|row| row[0] == map as u8).map(|row| row[1])
}

/// `PrintCardKeyText`'s test: a Silph Co floor and a card key door in front. The door is tile `$18`
/// or `$24`, or `$5e` on the eleventh floor.
pub fn card_key_door(map: Map, tile_in_front: u8) -> bool {
    let silph = rom_slice(pokered_symbols::SilphCoMapList).iter().take_while(|&&m| m != END).any(|&m| m == map as u8);
    silph && (matches!(tile_in_front, 0x18 | 0x24) || map == Map::SilphCo11F && tile_in_front == 0x5E)
}

/// The block an opened card key door becomes.
pub fn card_key_door_block(map: Map) -> u8 {
    if map == Map::SilphCo11F { 0x03 } else { 0x0E }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use super::*;

    #[derive(Deserialize)]
    struct HiddenEventInput {
        map: Map,
        x: u8,
        y: u8,
        facing: u8,
    }

    #[test]
    fn every_harvested_case_of_check_for_hidden_event() {
        let jsonl = include_str!("../../../fixtures/events/check_for_hidden_event.jsonl");
        for (i, expected, _) in cases::<HiddenEventInput, Option<HiddenEvent>>(jsonl) {
            assert_eq!(check_for_hidden_event(i.map, i.x, i.y, i.facing), expected, "{:?} ({}, {}) facing {:#04x}", i.map, i.x, i.y, i.facing);
        }
    }

    #[derive(Deserialize)]
    struct BookshelfInput {
        tileset: TileSetId,
        tile: u8,
        facing: u8,
    }

    /// `PrintBookshelfText` answers only a player facing up, as the overworld asks it.
    #[test]
    fn every_harvested_case_of_print_bookshelf_text() {
        let jsonl = include_str!("../../../fixtures/events/print_bookshelf_text.jsonl");
        for (i, expected, _) in cases::<BookshelfInput, Option<u8>>(jsonl) {
            let text = (i.facing == SpriteFacing::Up as u8).then(|| bookshelf_text(i.tileset, i.tile)).flatten();
            assert_eq!(text, expected, "{:?} tile {:#04x} facing {:#04x}", i.tileset, i.tile, i.facing);
        }
    }

    #[test]
    fn a_hidden_item_row_names_its_function_and_item() {
        let events = hidden_events(Map::ViridianForest).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!((events[1].x, events[1].y, events[1].argument), (16, 42, poke_core::item::ItemId::Antidote as u8));
        assert_eq!(events[1].function, pokered_symbols::HiddenItems);
        assert_eq!(hidden_item_index(Map::ViridianForest, 16, 42), 1);
        assert_eq!(hidden_item_index(Map::ViridianForest, 16, 41), 0xFF);
    }

    #[test]
    fn a_map_with_an_empty_table_is_in_the_list_and_one_without_is_not() {
        assert_eq!(hidden_events(Map::ViridianMart), Some(vec![]));
        assert_eq!(hidden_events(Map::PalletTown), None);
    }

    #[test]
    fn every_hidden_event_function_is_a_label() {
        let named = [
            pokered_symbols::HiddenItems, pokered_symbols::HiddenCoins, pokered_symbols::OpenPokemonCenterPC,
            pokered_symbols::PrintBenchGuyText, pokered_symbols::GymStatues, pokered_symbols::OpenRedsPC,
        ];
        let all: Vec<_> = Map::all().filter_map(hidden_events).flatten().collect();
        assert_eq!(all.len(), 217);
        for function in named {
            assert!(all.iter().any(|event| event.function == function), "{function}");
        }
    }

    #[test]
    fn a_bookshelf_is_a_text_predef_by_tileset() {
        assert_eq!(bookshelf_text(TileSetId::Pokecenter, 0x54), Some(0x42));
        assert_eq!(bookshelf_text(TileSetId::House, 0x3D), Some(0x3F));
        assert_eq!(bookshelf_text(TileSetId::Overworld, 0x54), None);
    }

    #[test]
    fn the_bench_guy_answers_the_left_facing_and_the_coins_forty_are_twenty() {
        assert_eq!(bench_guy_text(Map::ViridianPokecenter, SpriteFacing::Left as u8), Some(0x0F));
        assert_eq!(hidden_coins_amount(poke_core::item::ItemId::Coin as u8 + 40), [0, 0x20]);
        assert_eq!(hidden_coins_amount(poke_core::item::ItemId::Coin as u8 + 100), [1, 0]);
    }
}
