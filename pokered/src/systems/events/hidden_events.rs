//! `CheckForHiddenEvent` and the lookups the hidden objects make: `HiddenEventMaps`, the hidden
//! items and coins, the bookshelves, the bench guys, the gym statues and the Silph Co card key
//! doors.

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::sprite::SpriteFacing;
use poke_core::tables::{HiddenRoutine, TextPredef, BENCH_GUY_TEXTS, BOOKSHELF_TILE_IDS, HIDDEN_EVENTS, MAP_BADGE_FLAGS, SILPH_CO_MAP_LIST};
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
    /// The routine `hl` holds.
    pub function: HiddenRoutine,
    /// How far `wHiddenEventIndex` moved: the rows passed over before this one.
    pub skipped: u8,
}

/// Every hidden event on `map`, in the table's order, or `None` for a map not in `HiddenEventMaps`.
pub fn hidden_events(map: Map) -> Option<Vec<HiddenEvent>> {
    let &(_, rows) = HIDDEN_EVENTS.iter().find(|&&(m, _)| m == map as u8)?;
    Some(rows.iter().enumerate().map(|(i, row)| HiddenEvent {
        y: row.y,
        x: row.x,
        argument: row.argument,
        function: row.routine,
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
fn find_hidden_item_or_coins_index(table: &[(u8, u8, u8)], map: Map, x: u8, y: u8) -> u8 {
    table.iter().position(|&row| row == (map as u8, x, y)).map_or(END, |i| i as u8)
}

pub fn hidden_item_index(map: Map, x: u8, y: u8) -> u8 {
    find_hidden_item_or_coins_index(poke_core::tables::HIDDEN_ITEM_COORDS, map, x, y)
}

pub fn hidden_coin_index(map: Map, x: u8, y: u8) -> u8 {
    find_hidden_item_or_coins_index(poke_core::tables::HIDDEN_COIN_COORDS, map, x, y)
}

/// `HiddenCoins`' amount, BCD, from its argument less `COIN`: anything it does not name is a
/// hundred.
pub fn hidden_coins_amount(argument: u8, cartridge_bugs: bool) -> [u8; 2] {
    match argument.wrapping_sub(poke_core::item::ItemId::Coin as u8) {
        10 => [0x00, 0x10],
        20 => [0x00, 0x20],
        // The cartridge jumps to the twenty for forty too.
        40 if cartridge_bugs => [0x00, 0x20],
        40 => [0x00, 0x40],
        _ => [0x01, 0x00],
    }
}

/// `PrintBookshelfText`'s lookup: the text predef of the bookshelf, statue or poster whose tile is
/// in front of a player facing up, by tileset.
pub fn bookshelf_text(tileset: TileSetId, tile: u8) -> Option<TextPredef> {
    BOOKSHELF_TILE_IDS.iter().find(|&&(of, at, _)| of == tileset as u8 && at == tile).map(|&(_, _, text)| text)
}

/// `PrintBenchGuyText`'s lookup: the text predef for a player facing the bench guy on `map`.
pub fn bench_guy_text(map: Map, facing: u8, cartridge_bugs: bool) -> Option<TextPredef> {
    let bytes: Vec<u8> = BENCH_GUY_TEXTS.iter().flat_map(|&(map, facing, text)| [map, facing, text as u8]).chain([END]).collect();
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
            return bytes.get(i).and_then(|&id| TextPredef::from_id(id));
        }
        // The cartridge does not step past the text on a wrong facing, so its scan goes on out of
        // step, and past the table into VRAM; `rom_equality` pins that it finds nothing there.
        if !cartridge_bugs {
            i += 1;
        }
    }
    None
}

/// `GymStatues`' `MapBadgeFlags`: the badge bit of a gym, which `wBeatGymFlags` is compared against.
pub fn gym_badge(map: Map) -> Option<u8> {
    MAP_BADGE_FLAGS.iter().find(|&&(gym, _)| gym == map as u8).map(|&(_, badge)| badge)
}

/// `PrintCardKeyText`'s test: a Silph Co floor and a card key door in front. The door is tile `$18`
/// or `$24`, or `$5e` on the eleventh floor.
pub fn card_key_door(map: Map, tile_in_front: u8) -> bool {
    let silph = SILPH_CO_MAP_LIST.contains(&(map as u8));
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
            let text = (i.facing == SpriteFacing::Up as u8).then(|| bookshelf_text(i.tileset, i.tile)).flatten().map(|text| text as u8);
            assert_eq!(text, expected, "{:?} tile {:#04x} facing {:#04x}", i.tileset, i.tile, i.facing);
        }
    }

    #[test]
    fn a_hidden_item_row_names_its_function_and_item() {
        let events = hidden_events(Map::ViridianForest).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!((events[1].x, events[1].y, events[1].argument), (16, 42, poke_core::item::ItemId::Antidote as u8));
        assert_eq!(events[1].function, HiddenRoutine::HiddenItems);
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
        use HiddenRoutine::*;
        let named = [HiddenItems, HiddenCoins, OpenPokemonCenterPC, PrintBenchGuyText, GymStatues, OpenRedsPC];
        let all: Vec<_> = Map::all().filter_map(hidden_events).flatten().collect();
        assert_eq!(all.len(), 217);
        for function in named {
            assert!(all.iter().any(|event| event.function == function), "{function:?}");
        }
    }

    #[test]
    fn a_bookshelf_is_a_text_predef_by_tileset() {
        assert_eq!(bookshelf_text(TileSetId::Pokecenter, 0x54), Some(TextPredef::PokemonStuffText));
        assert_eq!(bookshelf_text(TileSetId::House, 0x3D), Some(TextPredef::TownMapText));
        assert_eq!(bookshelf_text(TileSetId::Overworld, 0x54), None);
    }

    #[test]
    fn the_bench_guy_answers_the_left_facing() {
        let text = Some(TextPredef::ViridianCityPokecenterBenchGuyText);
        assert_eq!(bench_guy_text(Map::ViridianPokecenter, SpriteFacing::Left as u8, false), text);
        assert_eq!(bench_guy_text(Map::ViridianPokecenter, SpriteFacing::Left as u8, true), text);
    }

    #[test]
    fn the_bench_guy_is_silent_to_any_other_facing() {
        for map in Map::all() {
            for facing in [SpriteFacing::Down, SpriteFacing::Up, SpriteFacing::Right] {
                assert_eq!(bench_guy_text(map, facing as u8, false), None, "{map:?} {facing:?}");
            }
        }
    }

    #[test]
    fn the_forty_coin_spot_gives_forty() {
        let coin = poke_core::item::ItemId::Coin as u8;
        assert_eq!(hidden_coins_amount(coin + 40, false), [0, 0x40]);
        assert_eq!(hidden_coins_amount(coin + 40, true), [0, 0x20], "the cartridge gives twenty");
        assert_eq!(hidden_coins_amount(coin + 20, false), [0, 0x20]);
        assert_eq!(hidden_coins_amount(coin + 100, false), [1, 0]);
    }
}
