//! Every table `poke_core::tables` reads from the disassembly, decoded again from the cartridge the
//! way it was read before, and compared as values. A submodule bump that moves a table fails here.

use pokered::world::Ruleset;
use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use poke_core::tables::*;

/// `count` rows of `width` bytes from `table`.
fn rows(table: DmgPointer, width: usize, count: usize) -> Vec<&'static [u8]> {
    rom_slice(table).chunks_exact(width).take(count).collect()
}

/// Rows of `width` bytes up to the `$FF` that ends them.
fn terminated(table: DmgPointer, width: usize) -> Vec<&'static [u8]> {
    rom_slice(table).chunks_exact(width).take_while(|row| row[0] != 0xFF).collect()
}

#[test]
fn type_effects() {
    let rom: Vec<_> = terminated(sym::TypeEffects, 3).iter().map(|row| (row[0], row[1], row[2])).collect();
    assert_eq!(rom, TYPE_EFFECTS);
}

#[test]
fn high_critical_moves() {
    let rom: Vec<_> = terminated(sym::HighCriticalMoves, 1).iter().map(|row| row[0]).collect();
    assert_eq!(rom, HIGH_CRITICAL_MOVES);
}

#[test]
fn stat_modifier_ratios() {
    let rom: Vec<_> = rows(sym::StatModifierRatios, 2, 13).iter().map(|row| (row[0], row[1])).collect();
    assert_eq!(rom, STAT_MODIFIER_RATIOS);
}

/// The first byte is two nybbles and the second is sign and magnitude.
#[test]
fn growth_rates() {
    let rom: Vec<_> = rows(sym::GrowthRateTable, 4, 6).iter().map(|row| GrowthRate {
        numerator: row[0] >> 4,
        denominator: row[0] & 0xF,
        squared: if row[1] & 0x80 != 0 { -((row[1] & 0x7F) as i8) } else { row[1] as i8 },
        linear: row[2],
        constant: row[3],
    }).collect();
    assert_eq!(rom, GROWTH_RATE_TABLE);
}

#[test]
fn pokedex_order() {
    assert_eq!(rom_slice(sym::PokedexOrder)[..190], POKEDEX_ORDER);
}

/// Dex number 0 to 151.
#[test]
fn monster_palettes() {
    assert_eq!(rom_slice(sym::MonsterPalettes)[..152], MONSTER_PALETTES);
}

/// Little-endian RGB555 words, four to a palette.
#[test]
fn super_palettes() {
    let rom: Vec<[u16; 4]> = rows(sym::SuperPalettes, 8, 0x25).iter()
        .map(|row| std::array::from_fn(|i| u16::from_le_bytes([row[i * 2], row[i * 2 + 1]])))
        .collect();
    assert_eq!(rom, SUPER_PALETTES);
}

/// A list of `width`-byte rows ending at a row that starts with `end`.
fn check_list<T: PartialEq + std::fmt::Debug>(table: DmgPointer, width: usize, end: u8, source: &[T], decode: impl Fn(&[u8]) -> T) {
    let rom: Vec<T> = rom_slice(table).chunks_exact(width).take_while(|row| row[0] != end).map(decode).collect();
    assert_eq!(rom, source, "{table}");
}

fn one(row: &[u8]) -> u8 { row[0] }
fn two(row: &[u8]) -> (u8, u8) { (row[0], row[1]) }
fn three(row: &[u8]) -> (u8, u8, u8) { (row[0], row[1], row[2]) }

#[test]
fn terminated_lists() {
    check_list(sym::BikeRidingTilesets, 1, 0xFF, BIKE_RIDING_TILESETS, one);
    check_list(sym::WaterTilesets, 1, 0xFF, WATER_TILESETS, one);
    check_list(sym::EscapeRopeTilesets, 1, 0xFF, ESCAPE_ROPE_TILESETS, one);
    check_list(sym::DungeonMaps1, 1, 0xFF, DUNGEON_MAPS_1, one);
    check_list(sym::DungeonMaps2, 2, 0xFF, DUNGEON_MAPS_2, two);
    check_list(sym::SilphCoMapList, 1, 0xFF, SILPH_CO_MAP_LIST, one);
    check_list(sym::MapBadgeFlags, 2, 0xFF, MAP_BADGE_FLAGS, two);
    check_list(sym::SafariZoneRestHouses, 1, 0xFF, SAFARI_ZONE_REST_HOUSES, one);
    check_list(sym::CutTreeBlockSwaps, 2, 0xFF, CUT_TREE_BLOCK_SWAPS, two);
    check_list(sym::WarpPadAndHoleData, 3, 0xFF, WARP_PAD_AND_HOLE_DATA, three);
    check_list(sym::DungeonWarpList, 2, 0xFF, DUNGEON_WARP_LIST, two);
    check_list(sym::HMMoves, 1, 0xFF, HM_MOVES, one);
    check_list(sym::HMMoveArray, 1, 0xFF, HM_MOVES, one);
    check_list(sym::GuardDrinksList, 1, 0, GUARD_DRINKS_LIST, one);
    check_list(sym::BallMoveDistances1, 1, 0xFF, BALL_MOVE_DISTANCES_1, one);
    check_list(sym::BallMoveDistances2, 1, 0xFF, BALL_MOVE_DISTANCES_2, one);
    check_list(sym::UpwardBallsAnimXCoordinatesPlayerTurn, 1, 0xFF, UPWARD_BALLS_ANIM_X_COORDINATES_PLAYER_TURN, one);
    check_list(sym::UpwardBallsAnimXCoordinatesEnemyTurn, 1, 0xFF, UPWARD_BALLS_ANIM_X_COORDINATES_ENEMY_TURN, one);
    check_list(sym::SpiralBallAnimationCoordinates, 2, 0xFF, SPIRAL_BALL_ANIMATION_COORDINATES, two);
    check_list(sym::WavyScreenLineOffsets, 1, 0x80, WAVY_SCREEN_LINE_OFFSETS, one);
    check_list(sym::FlashScreenLongMonochrome, 1, 1, FLASH_SCREEN_LONG_MONOCHROME, one);
    check_list(sym::FlashScreenLongSGB, 1, 1, FLASH_SCREEN_LONG_SGB, one);
    check_list(sym::BattleTransition_FlashScreenPalettes, 1, 1, BATTLE_TRANSITION_FLASH_SCREEN_PALETTES, one);
}

/// Written `map, x, y` and assembled `map, y, x`.
#[test]
fn forced_bike_or_surf_maps() {
    check_list(sym::ForcedBikeOrSurfMaps, 3, 0xFF, FORCED_BIKE_OR_SURF_MAPS, |row| (row[0], row[2], row[1]));
}

/// Tables read by index, at the lengths their readers assume.
#[test]
fn indexed_lists() {
    assert_eq!(rows(sym::PrizeMonLevelDictionary, 2, 6).into_iter().map(two).collect::<Vec<_>>(), PRIZE_MON_LEVEL_DICTIONARY);
    assert_eq!(rom_slice(sym::TitleMons)[..16], *TITLE_MONS);
    assert_eq!(rom_slice(sym::CreditsMons)[..15], *CREDITS_MONS);
    assert_eq!(rom_slice(sym::TechnicalMachines)[..50 + 5], *TECHNICAL_MACHINES);
    assert_eq!(rom_slice(sym::FallingObjects_InitialXCoords)[..20], *FALLING_OBJECTS_INITIAL_X_COORDS);
    assert_eq!(rom_slice(sym::FallingObjects_DeltaXs)[..9], *FALLING_OBJECTS_DELTA_XS);
    assert_eq!(rom_slice(sym::FallingObjects_InitialMovementData)[..20], *FALLING_OBJECTS_INITIAL_MOVEMENT_DATA);
    assert_eq!(rom_slice(sym::TitleBallYTable)[..12], *TITLE_BALL_Y_TABLE);
    assert_eq!(rom_slice(sym::PlayerJumpingYScreenCoords)[..16], *PLAYER_JUMPING_Y_SCREEN_COORDS);
    assert_eq!(rom_slice(sym::SpinnerPlayerFacingDirections)[..4], *SPINNER_PLAYER_FACING_DIRECTIONS);
}

/// Every delta a falling object reads while on screen, past the table included, is the byte the
/// cartridge reads there, and the recreation carries none that no petal shows.
#[test]
fn falling_object_deltas_as_the_cartridge_reads_them() {
    use pokered::modes::battle::animation::{falling_object_delta_x, next_movement_byte, RUN_ON_DELTA_XS};
    let rom = rom_slice(sym::FallingObjects_DeltaXs);
    let mut objects: Vec<(u8, u8)> = FALLING_OBJECTS_INITIAL_MOVEMENT_DATA.iter().enumerate()
        .map(|(i, &byte)| (if i == 0 { 0 } else { 8 * (i as u8 + 1) }, byte)).collect();
    let mut furthest = 0;
    while objects[0].0 != 104 {
        for (y, byte) in &mut objects {
            *byte = next_movement_byte(*byte, Ruleset::Gen1);
            *y = if *y + 2 >= 112 { 160 } else { *y + 2 };
            if *y < 112 {
                let index = *byte & 0x7F;
                assert_eq!(falling_object_delta_x(index), rom[usize::from(index)], "the delta at index {index}");
                furthest = furthest.max(index);
            }
        }
    }
    assert_eq!(usize::from(furthest), FALLING_OBJECTS_DELTA_XS.len() + RUN_ON_DELTA_XS.len());
}

/// `rBGP`, `rOBP0` and `rOBP1` for each of `FadePal1` to `FadePal8`, which the code reads as one table.
#[test]
fn palette_registers() {
    assert_eq!(rows(sym::FadePal1, 3, 8).into_iter().map(three).collect::<Vec<_>>(), FADE_PALETTES);
    assert_eq!(sym::FadePal8.address - sym::FadePal1.address, 7 * 3);
    assert_eq!(rom_slice(sym::IntroFadePalettes)[..6], *INTRO_FADE_PALETTES);
    assert_eq!(rom_slice(sym::HoFGBPalettes)[..4], *HOF_GB_PALETTES);
}

/// A stream of commands, run to `CRED_THE_END`.
#[test]
fn credits_order() {
    const CRED_THE_END: u8 = 0xFA;
    assert_eq!(CREDITS_ORDER.last(), Some(&CRED_THE_END));
    assert_eq!(rom_slice(sym::CreditsOrder)[..CREDITS_ORDER.len()], *CREDITS_ORDER);
}

/// The accuracy is stored out of 255, which `percent` computes from the percentage written.
#[test]
fn moves() {
    let rom = rows(sym::Moves, 6, MOVES.len());
    let source: Vec<[u8; 6]> = MOVES.iter()
        .map(|m| [m.animation, m.effect, m.power, m.move_type, (m.accuracy as u16 * 0xFF / 100) as u8, m.pp])
        .collect();
    assert_eq!(rom, source.iter().map(|row| &row[..]).collect::<Vec<_>>());
}

fn encoded(names: &[&str]) -> Vec<Vec<u8>> {
    names.iter().map(|name| poke_core::charmap::encode(name).unwrap()).collect()
}

/// `li`: each name ends at its own `@`.
#[test]
fn listed_names() {
    for (table, names) in [(sym::MoveNames, &MOVE_NAMES[..]), (sym::TrainerNames, &TRAINER_NAMES[..]), (sym::ItemNames, &ITEM_NAMES[..])] {
        let rom: Vec<Vec<u8>> = rom_slice(table).split(|&byte| byte == 0x50).take(names.len()).map(<[u8]>::to_vec).collect();
        assert_eq!(rom, encoded(names), "{table}");
    }
}

/// `dname`: ten bytes each, padded with `@`.
#[test]
fn monster_names() {
    let rom: Vec<Vec<u8>> = rows(sym::MonsterNames, 10, MONSTER_NAMES.len()).iter()
        .map(|entry| entry.iter().copied().take_while(|&byte| byte != 0x50).collect())
        .collect();
    assert_eq!(rom, encoded(&MONSTER_NAMES));
}

/// Three bytes and an eleven-byte nickname padded with `@`.
#[test]
fn trade_mons() {
    let rom: Vec<(u8, u8, u8, Vec<u8>)> = rows(sym::TradeMons, 14, TRADE_MONS.len()).iter()
        .map(|row| (row[0], row[1], row[2], row[3..].iter().copied().take_while(|&byte| byte != 0x50).collect()))
        .collect();
    let source: Vec<_> = TRADE_MONS.iter()
        .map(|trade| (trade.give, trade.get, trade.dialog, poke_core::charmap::encode(trade.nickname).unwrap()))
        .collect();
    assert_eq!(rom, source);
}

/// `bcd3`: three bytes of two decimal digits, most significant first.
#[test]
fn item_prices() {
    use poke_core::item::bcd3;
    let rom: Vec<[u8; 3]> = rows(sym::ItemPrices, 3, ITEM_PRICES.len()).iter().map(|row| [row[0], row[1], row[2]]).collect();
    assert_eq!(rom, ITEM_PRICES.map(bcd3));
    let rom: Vec<(u8, [u8; 3])> = rows(sym::VendingPrices, 4, VENDING_PRICES.len()).iter().map(|row| (row[0], [row[1], row[2], row[3]])).collect();
    assert_eq!(rom, VENDING_PRICES.map(|(item, price)| (item, bcd3(price))));
}

/// `nybble_array` packs two to a byte, high nybble first; `bit_array` eight, low bit first.
#[test]
fn packed_item_arrays() {
    let rom: Vec<u8> = rom_slice(sym::TechnicalMachinePrices).iter().flat_map(|&byte| [byte >> 4, byte & 0xF])
        .take(TECHNICAL_MACHINE_PRICES.len()).collect();
    assert_eq!(rom, TECHNICAL_MACHINE_PRICES);
    let bits = KEY_ITEM_FLAGS.len().div_ceil(8) * 8;
    let rom: Vec<bool> = rom_slice(sym::KeyItemFlags)[..bits / 8].iter().flat_map(|&byte| (0..8).map(move |bit| byte & 1 << bit != 0)).collect();
    assert_eq!(rom[..KEY_ITEM_FLAGS.len()], KEY_ITEM_FLAGS);
    assert!(rom[KEY_ITEM_FLAGS.len()..].iter().all(|&bit| !bit), "the last byte's padding is clear");
}

#[test]
fn usable_item_lists() {
    check_list(sym::UsableItems_PartyMenu, 1, 0xFF, USABLE_ITEMS_PARTY_MENU, one);
    check_list(sym::UsableItems_CloseMenu, 1, 0xFF, USABLE_ITEMS_CLOSE_MENU, one);
}

/// Dex-ordered 28-byte entries, Mew's at its own label. The `tmhm` list is packed a bit per
/// machine, in `TechnicalMachines`' order; Mew's `UNUSED` sets the 56th bit, which names no machine.
#[test]
fn base_stats() {
    const MACHINES: usize = 55;
    for (at, source) in BASE_STATS.iter().enumerate() {
        let pointer = if at == 150 { sym::MewBaseStats } else { sym::BaseStats + at as u16 * 28 };
        let e = &rom_slice(pointer)[..28];
        let rom = (e[0], [e[1], e[2], e[3], e[4], e[5]], [e[6], e[7]], e[8], e[9], [e[15], e[16], e[17], e[18]], e[19]);
        let fields = (source.dex, source.stats, source.types, source.catch_rate, source.base_exp, source.level_1_moves, source.growth_rate);
        assert_eq!(rom, fields, "dex {}", at + 1);
        let rom_machines: Vec<u8> = (0..MACHINES).filter(|&flag| e[20 + flag / 8] & 1 << (flag % 8) != 0).map(|flag| TECHNICAL_MACHINES[flag]).collect();
        let mut machines = source.tm_hm.to_vec();
        machines.sort_by_key(|mv| TECHNICAL_MACHINES.iter().position(|machine| machine == mv).expect("a machine's move"));
        assert_eq!(rom_machines, machines, "dex {}", at + 1);
    }
}

/// Each entry is the byte stream its pointer names: evolutions to a 0, then `level, move` to a 0.
#[test]
fn evos_moves() {
    let pointers = rom_slice(sym::EvosMovesPointerTable);
    for (at, source) in EVOS_MOVES.iter().enumerate() {
        let address = u16::from_le_bytes([pointers[at * 2], pointers[at * 2 + 1]]);
        let mut bytes = rom_slice(DmgPointer { address, ..sym::EvosMovesPointerTable }).iter().copied();
        let mut next = || bytes.next().unwrap();
        let mut evolutions = Vec::new();
        loop {
            evolutions.push(match next() {
                0 => break,
                1 => Evolution::Level { level: next(), into: next() },
                2 => Evolution::Item { item: next(), min_level: next(), into: next() },
                3 => Evolution::Trade { min_level: next(), into: next() },
                kind => panic!("evolution type {kind}"),
            });
        }
        let mut learnset = Vec::new();
        loop {
            match next() {
                0 => break,
                level => learnset.push((level, next())),
            }
        }
        assert_eq!((evolutions.as_slice(), learnset.as_slice()), (source.evolutions, source.learnset), "species {}", at + 1);
    }
}

/// Every tileset id's sheet and blockset are the ones its `Tilesets` row points at. The cartridge
/// copies `TILESET_TILES` from a sheet `--trim-whitespace` left short, so what it loads past the
/// end is the sheet's own blockset, up to the end of the bank, and for `RedsHouse` the start of
/// `House`'s sheet after it.
#[test]
fn tileset_sheets_and_blocksets() {
    use poke_core::map_gfx::{blockset, tileset_sheet, TILESET_TILES};
    use poke_core::map_header::TileSetId;
    use poke_core::rom_gfx::TILE_BYTES;
    use poke_core::symbols::DmgBank;
    for tileset in (0..=23).map(|id| TileSetId::from_repr(id).unwrap()) {
        let row = rom_tileset(tileset as u8);
        let at = |offset: usize| rom_slice(DmgPointer { bank: DmgBank::ROM { bank: row[0] }, address: u16::from_le_bytes([row[offset], row[offset + 1]]) });
        let (sheet, blocks) = (tileset_sheet(tileset), blockset(tileset));
        assert_eq!(at(3)[..sheet.len()], *sheet, "{tileset} sheet");
        assert_eq!(at(1)[..blocks.len()], *blocks, "{tileset} blockset");

        let copied = &at(3)[sheet.len()..];
        let tail = &copied[..copied.len().min(TILESET_TILES * TILE_BYTES - sheet.len())];
        let (own, next) = tail.split_at(tail.len().min(blocks.len()));
        assert_eq!(*own, blocks[..own.len()], "{tileset}: past its sheet is its blockset");
        if !next.is_empty() {
            assert!(matches!(tileset, TileSetId::RedsHouse1 | TileSetId::RedsHouse2), "{tileset} copies past its blockset");
            assert_eq!(*next, tileset_sheet(TileSetId::House)[..next.len()]);
        }
    }
}

/// Every `SpriteSheetPointerTable` row's sheet is the one it points at, loading the same count. A
/// walking picture's frames are read from past its standing ones, which for a sheet with none is
/// whatever sheet follows it in the bank.
#[test]
fn sprite_sheets() {
    use poke_core::map_objects::{sprite_sheet, FIRST_STILL_SPRITE};
    use poke_core::symbols::DmgBank;
    let rows = rows(sym::SpriteSheetPointerTable, 4, poke_core::gfx::SPRITE_SHEETS.len());
    for (picture, row) in (1..).zip(rows) {
        let sheet = sprite_sheet(picture).unwrap();
        assert_eq!(sheet.bytes, row[2] as usize, "picture {picture}");
        let rom = rom_slice(DmgPointer { bank: DmgBank::ROM { bank: row[3] }, address: u16::from_le_bytes([row[0], row[1]]) });
        assert_eq!(rom[..sheet.tiles.len()], *sheet.tiles, "picture {picture}");
        if picture < FIRST_STILL_SPRITE && sheet.tiles.len() < 2 * sheet.bytes {
            let walking = &rom[sheet.tiles.len()..][..sheet.bytes];
            assert!(poke_core::gfx::SPRITE_SHEETS.iter().any(|(next, _)| {
                let n = next.len().min(walking.len());
                next[..n] == walking[..n]
            }), "picture {picture}");
        }
    }
    assert_eq!(poke_core::gfx::SPRITE_SHEETS.len(), 0x48);
    assert!(sprite_sheet(0).is_none() && sprite_sheet(0x49).is_none());
}

/// Each map's entry is where its pointer says: a grass rate and ten `level, species` slots when it is
/// not 0, then the same for water.
#[test]
fn wild_data() {
    let pointers = rom_slice(sym::WildDataPointers);
    assert_eq!(pointers[WILD_DATA.len() * 2..][..2], [0xFF, 0xFF], "the table ends at -1");
    for (map, source) in WILD_DATA.iter().enumerate() {
        let address = u16::from_le_bytes([pointers[map * 2], pointers[map * 2 + 1]]);
        let mut bytes = rom_slice(DmgPointer { address, ..sym::WildDataPointers }).iter().copied();
        let mut list = || {
            let rate = bytes.next().unwrap();
            let slots: Vec<(u8, u8)> = if rate == 0 { vec![] } else { (0..10).map(|_| (bytes.next().unwrap(), bytes.next().unwrap())).collect() };
            (rate, slots)
        };
        let rom = [list(), list()];
        assert_eq!(rom, [(source.grass_rate, source.grass.to_vec()), (source.water_rate, source.water.to_vec())], "map {map}");
    }
}

/// `wild_chance` writes the running total less one, and the slot's offset into the list.
#[test]
fn wild_mon_encounter_slot_chances() {
    let mut total = 0;
    let source: Vec<(u8, u8)> = WILD_MON_ENCOUNTER_SLOT_CHANCES.iter().enumerate()
        .map(|(slot, &chance)| { total += chance as u16; ((total - 1) as u8, slot as u8 * 2) })
        .collect();
    assert_eq!(total, 256);
    assert_eq!(rows(sym::WildMonEncounterSlotChances, 2, 10).into_iter().map(two).collect::<Vec<_>>(), source);
}

/// The index is `map, pointer` to an `$FF`; a group is its count and then `level, species` pairs.
#[test]
fn fishing() {
    assert_eq!(rows(sym::GoodRodMons, 2, 2).into_iter().map(two).collect::<Vec<_>>(), GOOD_ROD_MONS);
    let rom: Vec<(u8, Vec<(u8, u8)>)> = rom_slice(sym::SuperRodData).chunks_exact(3).take_while(|row| row[0] != 0xFF).map(|row| {
        let group = rom_slice(DmgPointer { address: u16::from_le_bytes([row[1], row[2]]), ..sym::SuperRodData });
        (row[0], group[1..][..group[0] as usize * 2].chunks_exact(2).map(two).collect())
    }).collect();
    let source: Vec<(u8, Vec<(u8, u8)>)> = SUPER_ROD_DATA.iter().map(|&(map, mons)| (map, mons.to_vec())).collect();
    assert_eq!(rom, source);
}

/// A class's parties run from its pointer to the next class's; the last class's to `TrainerAI`, the
/// routine the linker puts after it. A party is `level, species…, 0`, or `$FF, level, species…, 0`.
#[test]
fn trainer_parties() {
    let pointers = rom_slice(sym::TrainerDataPointers);
    let at = |class: usize| u16::from_le_bytes([pointers[class * 2], pointers[class * 2 + 1]]);
    assert_eq!(sym::TrainerAI.bank, sym::TrainerDataPointers.bank);
    for (class, source) in TRAINER_PARTIES.iter().enumerate() {
        let end = if class + 1 < TRAINER_PARTIES.len() { at(class + 1) } else { sym::TrainerAI.address };
        let data = &rom_slice(DmgPointer { address: at(class), ..sym::TrainerDataPointers })[..(end - at(class)) as usize];
        let rom: Vec<TrainerParty> = data.split_inclusive(|&byte| byte == 0).map(|party| {
            let (&first, rest) = party[..party.len() - 1].split_first().unwrap();
            let mons: Vec<(u8, u8)> = if first == 0xFF { rest.chunks_exact(2).map(two).collect() } else { rest.iter().map(|&species| (first, species)).collect() };
            TrainerParty { per_mon_levels: first == 0xFF, mons: mons.leak() }
        }).collect();
        assert_eq!(rom, *source, "class {}", class + 1);
    }
}

/// `pic_money`: the pic's pointer, then the base reward in `bcd3`.
#[test]
fn trainer_base_money() {
    use poke_core::item::bcd3;
    let rom: Vec<[u8; 3]> = rows(sym::TrainerPicAndMoneyPointers, 5, TRAINER_BASE_MONEY.len()).iter().map(|row| [row[2], row[3], row[4]]).collect();
    assert_eq!(rom, TRAINER_BASE_MONEY.map(bcd3));
}

/// `dbw`: the uses, then the routine's address, compared by the name at it.
#[test]
fn trainer_ai() {
    let rom: Vec<(u8, u16)> = rows(sym::TrainerAIPointers, 3, TRAINER_AI_POINTERS.len()).iter().map(|row| (row[0], u16::from_le_bytes([row[1], row[2]]))).collect();
    let routines = [
        ("GenericAI", sym::GenericAI), ("JugglerAI", sym::JugglerAI), ("BlackbeltAI", sym::BlackbeltAI),
        ("GiovanniAI", sym::GiovanniAI), ("CooltrainerMAI", sym::CooltrainerMAI), ("CooltrainerFAI", sym::CooltrainerFAI),
        ("BrockAI", sym::BrockAI), ("MistyAI", sym::MistyAI), ("LtSurgeAI", sym::LtSurgeAI), ("ErikaAI", sym::ErikaAI),
        ("KogaAI", sym::KogaAI), ("BlaineAI", sym::BlaineAI), ("SabrinaAI", sym::SabrinaAI), ("Rival2AI", sym::Rival2AI),
        ("Rival3AI", sym::Rival3AI), ("LoreleiAI", sym::LoreleiAI), ("BrunoAI", sym::BrunoAI), ("AgathaAI", sym::AgathaAI),
        ("LanceAI", sym::LanceAI),
    ];
    let source: Vec<(u8, u16)> = TRAINER_AI_POINTERS.iter().map(|&(count, name)| {
        let (_, label) = routines.iter().find(|(known, _)| *known == name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(label.bank, sym::TrainerAIPointers.bank, "{name}");
        (count, label.address)
    }).collect();
    assert_eq!(rom, source);
}

/// `move_choices`: each class's layers and a 0; the special moves are `db` pairs, `TeamMoves` to `$FF`.
#[test]
fn trainer_move_choices() {
    let rom: Vec<&[u8]> = rom_slice(sym::TrainerClassMoveChoiceModifications).split(|&byte| byte == 0).take(TRAINER_CLASS_MOVE_CHOICE_MODIFICATIONS.len()).collect();
    assert_eq!(rom, TRAINER_CLASS_MOVE_CHOICE_MODIFICATIONS);
    assert_eq!(rows(sym::LoneMoves, 2, 8).into_iter().map(two).collect::<Vec<_>>(), LONE_MOVES);
    check_list(sym::TeamMoves, 2, 0xFF, TEAM_MOVES, two);
}

use crate::harvest::Oracle;
use poke_core::mon_gfx;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::DmgBank;
use strum::IntoEnumIterator;

/// The two 1bpp planes of the sprite buffers, low then high, as the cartridge lays them out:
/// column-major, 7 columns of 56 rows, unlike every other tile.
fn sprite_planes(shades: &[u8; mon_gfx::PIC_PX * mon_gfx::PIC_PX]) -> [Vec<u8>; 2] {
    use mon_gfx::PIC_PX;
    let mut planes = [vec![0u8; PIC_PX * PIC_PX / 8], vec![0u8; PIC_PX * PIC_PX / 8]];
    for y in 0..PIC_PX {
        for x in 0..PIC_PX {
            let (byte, bit) = ((x / 8) * PIC_PX + y, 7 - x % 8);
            let shade = shades[y * PIC_PX + x];
            planes[0][byte] |= (shade & 1) << bit;
            planes[1][byte] |= (shade >> 1) << bit;
        }
    }
    planes
}

/// What the cartridge leaves in the sprite buffers for `InterlaceMergeSpriteBuffers` to draw: the
/// low plane in `sSpriteBuffer0` and the high in `sSpriteBuffer1`.
fn sprite_buffers(oracle: &Oracle) -> [Vec<u8>; 2] {
    let len = mon_gfx::PIC_PX * mon_gfx::PIC_PX / 8;
    [oracle.read(sym::sSpriteBuffer0, len), oracle.read(sym::sSpriteBuffer1, len)]
}

/// `UncompressSpriteData` of the pic at `pic`, then `LoadUncompressedSpriteData`'s alignment by its
/// dimension byte.
fn placed_by_cartridge(oracle: &mut Oracle, pic: DmgPointer) -> [Vec<u8>; 2] {
    let DmgBank::ROM { bank } = pic.bank else { panic!("{pic} is not ROM") };
    oracle.write(sym::wSpriteInputPtr, &pic.address.to_le_bytes());
    oracle.registers_mut().a = bank;
    oracle.call(sym::UncompressSpriteData);
    align_by_cartridge(oracle, rom_slice(pic)[0])
}

fn align_by_cartridge(oracle: &mut Oracle, dimensions: u8) -> [Vec<u8>; 2] {
    let registers = oracle.registers_mut();
    (registers.a, registers.c) = (dimensions, dimensions);
    registers.set_de(sym::vFrontPic.address);
    let (_, stop) = oracle.call_until(sym::LoadUncompressedSpriteData, &[sym::InterlaceMergeSpriteBuffers]);
    assert_eq!(stop, Some(sym::InterlaceMergeSpriteBuffers));
    sprite_buffers(oracle)
}

/// A pic's dimension byte, as `pkmncompress` writes it.
fn dimensions(tiles: &[u8]) -> u8 {
    let side = mon_gfx::pic_side(tiles) as u8;
    side << 4 | side
}

/// Every pic, decompressed and placed by the cartridge on the emulator from the cartridge's own
/// pointers, against the generated tiles placed the recreation's way; a back pic scaled as both
/// halves draw it.
#[test]
fn pics() {
    let mut oracle = Oracle::from_state(include_bytes!("pokemon/data/at-celadon.bin"));
    for species in PokemonSpecies::iter() {
        oracle.write(sym::wCurSpecies, &[species as u8]);
        oracle.call(sym::GetMonHeader);
        oracle.write(sym::wCurPartySpecies, &[species as u8]);
        let front = mon_gfx::front_pic(species);
        assert_eq!(oracle.read(sym::wMonHSpriteDim, 1)[0], dimensions(front), "{species}");
        oracle.registers_mut().set_de(sym::vFrontPic.address);
        let (_, stop) = oracle.call_until(sym::LoadMonFrontSprite, &[sym::InterlaceMergeSpriteBuffers]);
        assert_eq!(stop, Some(sym::InterlaceMergeSpriteBuffers));
        assert!(sprite_buffers(&oracle) == sprite_planes(&mon_gfx::pic_shades(front)), "{species}'s front");

        oracle.registers_mut().set_hl((sym::wMonHBackSprite.address - sym::wMonHeader.address) as u16);
        oracle.call(sym::UncompressMonSprite);
        oracle.call(sym::ScaleSpriteByTwo);
        let back = mon_gfx::back_pic(species);
        assert!(sprite_buffers(&oracle) == sprite_planes(&mon_gfx::scaled_back_pic_shades(back)), "{species}'s back");
    }

    let named: [(DmgPointer, &[u8]); 10] = [
        (sym::RedPicFront, poke_core::gfx::player::RED),
        (sym::RedPicBack, poke_core::gfx::player::REDB),
        (sym::OldManPicBack, poke_core::gfx::battle::OLDMANB),
        (sym::ShrinkPic1, poke_core::gfx::player::SHRINK1),
        (sym::ShrinkPic2, poke_core::gfx::player::SHRINK2),
        (sym::GhostPic, poke_core::gfx::battle::GHOST),
        (sym::FossilKabutopsPic, poke_core::gfx::pokemon::front::FOSSILKABUTOPS),
        (sym::FossilAerodactylPic, poke_core::gfx::pokemon::front::FOSSILAERODACTYL),
        (sym::ProfOakPic, poke_core::gfx::trainers::PROF_OAK),
        (sym::Rival1Pic, poke_core::gfx::trainers::RIVAL1),
    ];
    let trainers = rows(sym::TrainerPicAndMoneyPointers, 5, poke_core::trainers::NUM_TRAINERS as usize).into_iter().enumerate().map(|(index, row)| {
        let pic = DmgPointer { address: u16::from_le_bytes([row[0], row[1]]), ..sym::YoungsterPic };
        (pic, poke_core::trainers::pic(index as u8 + 1))
    });
    for (pic, tiles) in named.into_iter().chain(trainers) {
        assert_eq!(rom_slice(pic)[0], dimensions(tiles), "{pic}");
        assert!(placed_by_cartridge(&mut oracle, pic) == sprite_planes(&mon_gfx::pic_shades(tiles)), "{pic}");
    }
    let back = [(sym::RedPicBack, poke_core::gfx::player::REDB), (sym::OldManPicBack, poke_core::gfx::battle::OLDMANB)];
    for (pic, tiles) in back {
        oracle.write(sym::wSpriteInputPtr, &pic.address.to_le_bytes());
        let DmgBank::ROM { bank } = pic.bank else { unreachable!() };
        oracle.registers_mut().a = bank;
        oracle.call(sym::UncompressSpriteData);
        oracle.call(sym::ScaleSpriteByTwo);
        assert!(sprite_buffers(&oracle) == sprite_planes(&mon_gfx::scaled_back_pic_shades(tiles)), "{pic} scaled");
    }
}

/// The bytes from `start` up to `end`, two labels of one bank.
fn between(start: DmgPointer, end: DmgPointer) -> &'static [u8] {
    &rom_slice(start)[..(end.address - start.address) as usize]
}

/// `tile_ids`: the list's pointer, then its height and width as nybbles.
#[test]
fn tile_id_lists() {
    use poke_core::gfx::TILE_ID_LISTS;
    for (entry, row) in rows(sym::TileIDListPointerTable, 3, TILE_ID_LISTS.len()).into_iter().enumerate() {
        let (ids, width, height) = TILE_ID_LISTS[entry];
        let list = DmgPointer { address: u16::from_le_bytes([row[0], row[1]]), ..sym::TileIDListPointerTable };
        assert_eq!(((row[2] >> 4) as usize, (row[2] & 0xF) as usize), (height, width), "entry {entry}");
        assert_eq!(&rom_slice(list)[..width * height], ids, "entry {entry}");
    }
}

/// Each blob a screen loads across more than one `INCBIN`: the files in order, and a blank tile
/// where the source pads one on.
#[test]
fn intro_and_title_graphics_run_on_across_their_labels() {
    use poke_core::gfx::{intro, splash, title};
    let blank = [0u8; 16];
    assert_eq!(between(sym::GameFreakIntro, sym::GameFreakIntroEnd), [&splash::GAMEFREAK_PRESENTS[..], splash::GAMEFREAK_LOGO, &blank].concat());
    assert_eq!(between(sym::FightIntroBackMon, sym::FightIntroBackMonEnd), [&intro::GENGAR[..], &blank].concat());
    assert_eq!(between(sym::FightIntroFrontMon, sym::FightIntroFrontMonEnd),
        [&intro::RED_NIDORINO_1[..], intro::RED_NIDORINO_2, intro::RED_NIDORINO_3].concat());
    assert_eq!(between(sym::NintendoCopyrightLogoGraphics, sym::GameFreakLogoGraphicsEnd), [&splash::COPYRIGHT[..], title::GAMEFREAK_INC].concat());
    assert_eq!(between(sym::Version_GFX, sym::Version_GFXEnd), title::RED_VERSION);
}

/// The trainer card copies `$17` tiles from `BlankLeaderNames`, the last of them `CircleTile`.
#[test]
fn the_trainer_cards_leader_names_run_into_the_circle() {
    use poke_core::gfx::trainer_card::{BLANK_LEADER_NAMES, CIRCLE_TILE};
    assert_eq!(&rom_slice(sym::BlankLeaderNames)[..0x17 * 16], [&BLANK_LEADER_NAMES[..], CIRCLE_TILE].concat());
}

/// `BorderPalettes`: the tilemap, padding to `$800`, then three palettes `$20` apart.
#[test]
fn sgb_border_palettes() {
    use poke_core::gfx::SGB_BORDER_PALETTES;
    let data = rom_slice(sym::BorderPalettes);
    for (i, palette) in SGB_BORDER_PALETTES.iter().enumerate() {
        let at = 0x800 + i * 0x20;
        let rom: Vec<u16> = data[at..at + 8].chunks_exact(2).map(|w| u16::from_le_bytes([w[0], w[1]])).collect();
        assert_eq!(rom, palette, "palette {i}");
    }
}

#[test]
fn sgb_packets() {
    use poke_core::gfx::sgb_packets::*;
    let packets = [
        (sym::BlkPacket_WholeScreen, BLK_PACKET_WHOLE_SCREEN), (sym::BlkPacket_Battle, BLK_PACKET_BATTLE),
        (sym::BlkPacket_StatusScreen, BLK_PACKET_STATUS_SCREEN), (sym::BlkPacket_Pokedex, BLK_PACKET_POKEDEX),
        (sym::BlkPacket_Slots, BLK_PACKET_SLOTS), (sym::BlkPacket_Titlescreen, BLK_PACKET_TITLESCREEN),
        (sym::BlkPacket_NidorinoIntro, BLK_PACKET_NIDORINO_INTRO), (sym::BlkPacket_PartyMenu, BLK_PACKET_PARTY_MENU),
        (sym::BlkPacket_TrainerCard, BLK_PACKET_TRAINER_CARD), (sym::BlkPacket_GameFreakIntro, BLK_PACKET_GAME_FREAK_INTRO),
        (sym::PalPacket_Empty, PAL_PACKET_EMPTY), (sym::PalPacket_PartyMenu, PAL_PACKET_PARTY_MENU),
        (sym::PalPacket_Black, PAL_PACKET_BLACK), (sym::PalPacket_TownMap, PAL_PACKET_TOWN_MAP),
        (sym::PalPacket_Pokedex, PAL_PACKET_POKEDEX), (sym::PalPacket_Slots, PAL_PACKET_SLOTS),
        (sym::PalPacket_Titlescreen, PAL_PACKET_TITLESCREEN), (sym::PalPacket_TrainerCard, PAL_PACKET_TRAINER_CARD),
        (sym::PalPacket_Generic, PAL_PACKET_GENERIC), (sym::PalPacket_NidorinoIntro, PAL_PACKET_NIDORINO_INTRO),
        (sym::PalPacket_GameFreakIntro, PAL_PACKET_GAME_FREAK_INTRO),
    ];
    for (label, packet) in packets {
        let rom = rom_slice(label);
        assert_eq!(&rom[..(rom[0] & 7) as usize * 16], packet, "{label}");
    }
}

/// `fishing_gfx`: the tiles' pointer, their count, their bank and the `vNPCSprites` tile they go to.
#[test]
fn red_fishing_tiles() {
    use poke_core::gfx::RED_FISHING_TILES;
    for (row, &(tiles, tile)) in rows(sym::RedFishingTiles, 6, RED_FISHING_TILES.len()).into_iter().zip(RED_FISHING_TILES) {
        let source = DmgPointer { bank: poke_core::symbols::DmgBank::ROM { bank: row[3] }, address: u16::from_le_bytes([row[0], row[1]]) };
        assert_eq!(row[2] as usize * 16, tiles.len());
        assert_eq!(&rom_slice(source)[..tiles.len()], tiles);
        assert_eq!(u16::from_le_bytes([row[4], row[5]]), 0x8000 + tile as u16 * 16);
    }
}

#[test]
fn object_tables() {
    use poke_core::gfx::*;
    let objects = |table, end: DmgPointer| between(table, end).chunks_exact(4).map(|row| <[u8; 4]>::try_from(row).unwrap()).collect::<Vec<_>>();
    assert_eq!(objects(sym::FishingRodOAM, sym::RedFishingTiles), FISHING_ROD_OAM);
    assert_eq!(objects(sym::PokeCenterOAMData, sym::FlashSprite8Times), POKE_CENTER_OAM);
    assert_eq!(objects(sym::SmallStarsOAM, sym::SmallStarsOAMEnd), SMALL_STARS_OAM);
    assert_eq!(objects(sym::GameFreakLogoOAMData, sym::GameFreakLogoOAMDataEnd), GAME_FREAK_LOGO_OAM);
    assert_eq!(objects(sym::GameFreakShootingStarOAMData, sym::GameFreakShootingStarOAMDataEnd), GAME_FREAK_SHOOTING_STAR_OAM);
}

/// `AnimateHealingMachine` copies three tiles from a two-tile picture, the third being the first
/// four objects of `PokeCenterOAMData`; the recreation copies two.
#[test]
fn the_healing_machines_third_tile_is_object_data() {
    use poke_core::gfx::{overworld::HEAL_MACHINE, POKE_CENTER_OAM};
    let rom = &rom_slice(sym::PokeCenterFlashingMonitorAndHealBall)[..3 * 16];
    assert_eq!(&rom[..32], HEAL_MACHINE);
    assert_eq!(rom[32..], *POKE_CENTER_OAM[..4].concat());
}

/// The four tiles `LoadSlotMachineTiles` copies past the symbols' picture are `MoveAnimation`'s
/// code; the recreation copies the picture.
#[test]
fn the_slot_symbols_run_into_move_animation() {
    use poke_core::gfx::slots::RED_SLOTS_2;
    assert_eq!(between(sym::SlotMachineTiles2, sym::SlotMachineTiles2End), RED_SLOTS_2);
    assert_eq!(sym::SlotMachineTiles2End, sym::MoveAnimation);
}

/// `SlotRewardPointers`: a routine and its payout's text for each symbol, the text a `@`-ended
/// string shorter than the four bytes the cartridge copies.
#[test]
fn slot_reward_texts() {
    for (row, text) in rows(sym::SlotRewardPointers, 4, SLOT_REWARD_TEXTS.len()).into_iter().zip(SLOT_REWARD_TEXTS) {
        let at = DmgPointer { address: u16::from_le_bytes([row[2], row[3]]), ..sym::SlotRewardPointers };
        let string: Vec<u8> = rom_slice(at).iter().copied().take_while(|&b| b != 0x50).collect();
        assert_eq!(string, poke_core::charmap::encode(text).unwrap(), "{text}");
    }
}

/// The price the native agent offers the Bicycle at is the one the clerk's menu draws.
#[test]
fn bike_shop_menu_price() {
    let string: Vec<u8> = rom_slice(sym::BikeShopMenuPrice).iter().copied().take_while(|&b| b != 0x50).collect();
    assert_eq!(string, poke_core::charmap::encode(&pokered::scripts::bike_shop::menu_price_text()).unwrap());
}

/// Each bank's copy of the pointers and the five waves; the pointers past them name the bytes that
/// follow, which in the first bank are the wave the recreation plays for all three.
#[test]
fn wave_samples() {
    use pokered::audio::data::{AudioBank, LAVENDER_WAVE};
    let waves = WAVE_SAMPLES.concat();
    for (bank, pointers) in AudioBank::ALL.into_iter().zip([sym::Audio1_WavePointers, sym::Audio2_WavePointers, sym::Audio3_WavePointers]) {
        let first = pointers.address + 2 * WAVE_POINTERS.len() as u16;
        assert_eq!(&rom_slice(DmgPointer { address: first, ..pointers })[..waves.len()], waves, "{bank:?}");
        for (row, &wave) in rows(pointers, 2, WAVE_POINTERS.len()).into_iter().zip(&WAVE_POINTERS) {
            assert_eq!(u16::from_le_bytes([row[0], row[1]]), first + 16 * wave as u16, "{bank:?}");
        }
    }
    let past = DmgPointer { address: sym::Audio1_WavePointers.address + 2 * WAVE_POINTERS.len() as u16 + waves.len() as u16, ..sym::Audio1_WavePointers };
    assert_eq!(rom_slice(past)[..16], LAVENDER_WAVE);
}

/// Five bytes a can; the cartridge's wrapped offset reads 256 bytes past a can's mask, which for
/// every can is the bank's zero padding.
#[test]
fn gym_trash_cans() {
    for (row, &(mask, neighbours)) in rows(sym::GymTrashCans, 5, GYM_TRASH_CANS.len()).into_iter().zip(&GYM_TRASH_CANS) {
        assert_eq!(row, [&[mask][..], &neighbours].concat());
    }
    let data = rom_slice(sym::GymTrashCans);
    assert!((0..GYM_TRASH_CANS.len()).all(|can| data[5 * can + 1 + 0xFF] == 0));
}

/// `map_coord_movement`: `y, x` and the movement's pointer.
#[test]
fn pokemon_tower_7f_npc_coord_movement_table() {
    let table = sym::PokemonTower7FNPCCoordMovementTable;
    for (row, &((x, y), movement)) in rows(table, 4, POKEMON_TOWER_7F_NPC_COORD_MOVEMENT_TABLE.len()).into_iter().zip(&POKEMON_TOWER_7F_NPC_COORD_MOVEMENT_TABLE) {
        assert_eq!((row[0], row[1]), (y, x));
        let at = DmgPointer { address: u16::from_le_bytes([row[2], row[3]]), ..table };
        assert_eq!(&rom_slice(at)[..movement.len()], movement, "({x}, {y})");
    }
}

/// `MapHeaderPointers` and `MapHeaderBanks`: where map `id`'s header is in the cartridge.
fn rom_map_header(id: u8) -> DmgPointer {
    let address = rom_slice(sym::MapHeaderPointers + id as u16 * 2);
    let bank = rom_slice(sym::MapHeaderBanks + id as u16)[0];
    DmgPointer { bank: poke_core::symbols::DmgBank::ROM { bank }, address: u16::from_le_bytes([address[0], address[1]]) }
}

/// `map_header` as assembled: tileset, height, width, three pointers, the connection flags, one
/// 11-byte `connection` per flag in the order north, south, west, east, then the objects pointer.
/// Each connection's pointers are compared with the label base they carry taken off: the connected
/// map's blocks for the strip's source, `wOverworldMap` for its destination and the view.
#[test]
fn map_headers_and_connections() {
    use poke_core::map::Map;
    use poke_core::map_header::MapHeader;
    let word = |bytes: &[u8], at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let overworld = sym::wOverworldMap.address;
    let own: Vec<u8> = (0..MAP_HEADERS.len() as u8).filter(|&id| MAP_HEADERS[id as usize].is_some()).collect();
    for id in 0..MAP_HEADERS.len() as u8 {
        let pointer = rom_map_header(id);
        let map = Map::from_repr(id).unwrap();
        let Ok(header) = MapHeader::read(map) else {
            // Its bank is a placeholder: nothing loads a map with no header of its own.
            assert!(own.iter().any(|&other| rom_map_header(other).address == pointer.address), "{map} borrows no header");
            continue;
        };
        let rom = rom_slice(pointer);
        assert_eq!((rom[0], rom[1], rom[2]), (header.tileset as u8, header.height, header.width), "{map}");
        let blocks = rom_slice(DmgPointer { bank: pointer.bank, address: word(rom, 3) });
        assert_eq!(&blocks[..header.blocks.len()], header.blocks, "{map}'s blocks");
        let connections = [header.north_connection, header.south_connection, header.west_connection, header.east_connection];
        let flags = [8, 4, 2, 1].iter().zip(&connections).filter(|(_, it)| it.is_some()).map(|(flag, _)| flag).sum::<u8>();
        assert_eq!(rom[9], flags, "{map}'s connections");
        for (i, connection) in connections.iter().flatten().enumerate() {
            let row = &rom[10 + i * 11..];
            let connected = rom_slice(rom_map_header(connection.map as u8));
            assert_eq!(
                (row[0], word(row, 1) - word(connected, 3), word(row, 3) - overworld, row[5], row[6], row[7] as i8, row[8] as i8, word(row, 9) - overworld),
                (connection.map as u8, connection.strip_src_block, connection.strip_dest_block, connection.strip_length,
                 connection.connected_map_width, connection.y_alignment, connection.x_alignment, connection.view_block),
                "{map}'s {:?} connection", connection.direction,
            );
        }
    }
}

/// `<Map>_Object` as assembled: the border block, then counted warps (`y, x, warp - 1, map`), signs
/// (`y, x, text`) and objects (`picture, y + 4, x + 4, movement, range, text`, a trainer's text with
/// bit 6 and its class and number after it, an item's with bit 7 and its id), then one
/// `event_displacement` per warp.
#[test]
fn map_objects() {
    use poke_core::map::Map;
    use poke_core::map_objects::*;
    for id in 0..MAP_HEADERS.len() as u8 {
        let map = Map::from_repr(id).unwrap();
        let Ok(source) = MapObjects::read(map) else { continue };
        let header = rom_slice(rom_map_header(id));
        let connections = header[9].count_ones() as usize;
        let at = 10 + connections * 11;
        let mut bytes = rom_slice(DmgPointer { bank: rom_map_header(id).bank, address: u16::from_le_bytes([header[at], header[at + 1]]) }).iter().copied();
        let mut next = || bytes.next().unwrap();
        let border_block = next();
        let warps: Vec<Warp> = (0..next()).map(|_| Warp { y: next(), x: next(), destination_warp: next(), destination_map: next() }).collect();
        let signs: Vec<Sign> = (0..next()).map(|_| Sign { y: next(), x: next(), text_id: next() }).collect();
        let objects: Vec<ObjectEvent> = (0..next()).map(|_| {
            let (picture, map_y, map_x, movement1, movement2, text) = (next(), next(), next(), next(), next(), next());
            let kind = match text {
                text if text & 0x40 != 0 => ObjectKind::Trainer { class: next(), number: next() },
                text if text & 0x80 != 0 => ObjectKind::Item(next()),
                _ => ObjectKind::Person,
            };
            ObjectEvent { picture, map_y, map_x, movement1, movement2, text_id: text & 0x3F, kind }
        }).collect();
        let warp_to: Vec<WarpTo> = warps.iter().map(|_| {
            let view = u16::from_le_bytes([next(), next()]).wrapping_sub(sym::wOverworldMap.address);
            WarpTo { view, y: next(), x: next() }
        }).collect();
        assert_eq!(MapObjects { border_block, warps, signs, objects, warp_to }, source, "{map}");
    }
}

/// `Tilesets`' row for tileset `id`: the graphics' bank, then the blocks, graphics and collision
/// list pointers, the three counter tiles, the grass tile and the animation.
fn rom_tileset(id: u8) -> &'static [u8] {
    &rom_slice(sym::Tilesets + id as u16 * 12)[..12]
}

/// Each `tileset` row, and the collision list its pointer names in bank 0.
#[test]
fn tilesets() {
    for (id, source) in TILESETS.iter().enumerate() {
        let row = rom_tileset(id as u8);
        let collision = rom_slice(DmgPointer { bank: poke_core::symbols::DmgBank::ROM { bank: 0 }, address: u16::from_le_bytes([row[5], row[6]]) });
        let collision: Vec<u8> = collision.iter().copied().take_while(|&tile| tile != 0xFF).collect();
        assert_eq!((&row[7..10], row[10], row[11], collision.as_slice()),
            (source.counter_tiles.as_slice(), source.grass_tile, source.animation, source.collision), "{}", source.name);
    }
    check_list(sym::DungeonTilesets, 1, 0xFF, DUNGEON_TILESETS, |row| row[0]);
    check_list(sym::LedgeTiles, 4, 0xFF, LEDGE_TILES, |row| (row[0], row[1], row[2], row[3]));
}

/// A pointer table's lists, each read from where its pointer lands to its terminator, which is how
/// a list with no terminator of its own shares the next one's.
#[test]
fn warp_and_door_tile_ids() {
    let list = |bank, address: u16, end: u8| -> Vec<u8> {
        rom_slice(DmgPointer { bank, address }).iter().copied().take_while(|&tile| tile != end).collect()
    };
    let pointed = |table: DmgPointer, count: usize| -> Vec<Vec<u8>> {
        rows(table, 2, count).iter().map(|row| list(table.bank, u16::from_le_bytes([row[0], row[1]]), 0xFF)).collect()
    };
    assert_eq!(pointed(sym::WarpTileIDPointers, WARP_TILE_IDS.len()), WARP_TILE_IDS);
    assert_eq!(pointed(sym::WarpTileListPointers, WARP_CARPET_TILE_IDS.len()), WARP_CARPET_TILE_IDS);
    let doors: Vec<(u8, Vec<u8>)> = terminated(sym::DoorTileIDPointers, 3).iter()
        .map(|row| (row[0], list(sym::DoorTileIDPointers.bank, u16::from_le_bytes([row[1], row[2]]), 0)))
        .collect();
    assert_eq!(doors, DOOR_TILE_IDS.iter().map(|&(tileset, tiles)| (tileset, tiles.to_vec())).collect::<Vec<_>>());
}

#[test]
fn sprite_sets() {
    assert_eq!(rom_slice(sym::MapSpriteSets)[..MAP_SPRITE_SETS.len()], MAP_SPRITE_SETS);
    let split: Vec<_> = rows(sym::SplitMapSpriteSets, 4, SPLIT_MAP_SPRITE_SETS.len()).iter().map(|row| (row[0], row[1], row[2], row[3])).collect();
    assert_eq!(split, SPLIT_MAP_SPRITE_SETS);
    assert_eq!(rows(sym::SpriteSets, 11, SPRITE_SETS.len()), SPRITE_SETS.iter().map(|set| set.as_slice()).collect::<Vec<_>>());
}

/// `toggle_object_state` rows in flag order, to the `$FF` after them, and each map's pointer into
/// them, which is where the map's first row now sits.
#[test]
fn toggleable_objects() {
    const ON: u8 = 0x15;
    const OFF: u8 = 0x11;
    let rom: Vec<(u8, u8, bool)> = terminated(sym::ToggleableObjectStates, 3).iter().map(|row| {
        assert!(row[2] == ON || row[2] == OFF, "{row:?}");
        (row[0], row[1], row[2] == ON)
    }).collect();
    assert_eq!(rom, TOGGLEABLE_OBJECT_STATES);
    for (map, row) in rows(sym::ToggleableObjectMapPointers, 2, MAP_HEADERS.len()).iter().enumerate() {
        let address = u16::from_le_bytes([row[0], row[1]]);
        let first = TOGGLEABLE_OBJECT_STATES.iter().position(|&(of, _, _)| of as usize == map);
        match first {
            Some(first) => assert_eq!(address, sym::ToggleableObjectStates.address + first as u16 * 3, "map {map}"),
            // `NoToggleData`, a lone `$FF` row.
            None => assert_eq!(rom_slice(DmgPointer { bank: sym::ToggleableObjectMapPointers.bank, address })[0], 0xFF, "map {map}"),
        }
    }
}

use std::collections::HashMap;

/// Every label and variable `pokered.sym` names, as `(bank, address)`: where the assembler put what
/// the source names.
fn symbol_table() -> HashMap<String, (u8, u16)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/pokered/pokered.sym");
    std::fs::read_to_string(path).unwrap().lines().filter_map(|line| {
        let (at, name) = line.split_once(' ')?;
        let (bank, address) = at.split_once(':')?;
        Some((name.to_string(), (u8::from_str_radix(bank, 16).ok()?, u16::from_str_radix(address, 16).ok()?)))
    }).collect()
}

/// A script's bytes as `macros/scripts/text.asm` lays them out, up to where the cartridge stops
/// reading it.
fn assemble_text(script: &[TextMacro], symbols: &HashMap<String, (u8, u16)>) -> Vec<u8> {
    let address = |name: &str| symbols.get(name).unwrap_or_else(|| panic!("{name} is not in the .sym")).1.to_le_bytes();
    let mut bytes = Vec::new();
    for command in script {
        match *command {
            TextMacro::Run(text) => {
                bytes.push(0x00);
                bytes.extend(poke_core::charmap::encode(text).unwrap());
                if !ends_the_script(text) {
                    bytes.push(0x50);
                }
            }
            TextMacro::Ram(at) => bytes.extend([0x01].into_iter().chain(address(at))),
            TextMacro::Bcd { at, flags } => bytes.extend([0x02].into_iter().chain(address(at)).chain([flags])),
            TextMacro::Low => bytes.push(0x05),
            TextMacro::PromptButton => bytes.push(0x06),
            TextMacro::Scroll => bytes.push(0x07),
            TextMacro::Asm(_) => bytes.push(0x08),
            TextMacro::Decimal { at, bytes: size, digits } => bytes.extend([0x09].into_iter().chain(address(at)).chain([size << 4 | digits])),
            TextMacro::Pause => bytes.push(0x0A),
            TextMacro::Dots(count) => bytes.extend([0x0C, count]),
            TextMacro::WaitButton => bytes.push(0x0D),
            TextMacro::Sound(name) => bytes.push(match name {
                "sound_get_item_1" => 0x0B,
                "sound_pokedex_rating" => 0x0E,
                "sound_get_item_1_duplicate" => 0x0F,
                "sound_get_item_2" => 0x10,
                "sound_get_key_item" => 0x11,
                "sound_caught_mon" => 0x12,
                "sound_dex_page_added" => 0x13,
                "sound_cry_nidorina" => 0x14,
                "sound_cry_pidgeot" => 0x15,
                "sound_cry_dewgong" => 0x16,
                _ => panic!("{name}"),
            }),
            TextMacro::Far(label) => {
                let (bank, at) = symbols[label];
                bytes.extend([0x17].into_iter().chain(at.to_le_bytes()).chain([bank]));
            }
        }
    }
    let ends_itself = match script.last() {
        Some(TextMacro::Asm(_)) => true,
        Some(TextMacro::Run(text)) => ends_the_script(text),
        _ => false,
    };
    if !ends_itself {
        bytes.push(0x50);
    }
    bytes
}

/// A run ending in `<DONE>`, `<PROMPT>` or `<DEXEND>` has no `@`, and nothing after it is read.
fn ends_the_script(run: &str) -> bool {
    ["<DONE>", "<PROMPT>", "<DEXEND>"].iter().any(|end| run.ends_with(end))
}

/// Every text script assembles to the bytes at its label, and every far-text body the cartridge
/// labels is one of them.
#[test]
fn texts() {
    let symbols = symbol_table();
    for &(label, script) in TEXTS {
        let (bank, address) = symbols[label];
        let at = DmgPointer { bank: DmgBank::ROM { bank }, address };
        let bytes = assemble_text(script, &symbols);
        assert_eq!(bytes, rom_slice(at)[..bytes.len()], "{label}");
    }
    let far_bodies = symbols.iter().filter(|&(name, &(_, address))| {
        name.starts_with('_') && name.contains("Text") && !name.contains('.') && address >= 0x4000
    });
    for (name, _) in far_bodies {
        assert!(TEXTS.binary_search_by(|(label, _)| (*label).cmp(name)).is_ok(), "{name} is not in TEXTS");
    }
}

/// Every map's text pointer table holds, id by id, where the labels its source lists are.
#[test]
fn text_pointers() {
    let symbols = symbol_table();
    for &(table, entries) in TEXT_POINTERS {
        let (bank, address) = symbols[table];
        let rom = rom_slice(DmgPointer { bank: DmgBank::ROM { bank }, address });
        for (id, entry) in entries.iter().enumerate() {
            assert_eq!(u16::from_le_bytes([rom[id * 2], rom[id * 2 + 1]]), symbols[*entry].1, "{table} {}: {entry}", id + 1);
        }
    }
}

/// Every `def_trainers` table is the headers `CheckForEngagingTrainers` walks, to its end, and
/// every constant naming a table or a header names where it is.
#[test]
fn trainer_headers() {
    use poke_core::trainer_headers::TrainerRef;
    const TRAINER_STRUCT_SIZE: usize = 12;
    let symbols = symbol_table();
    let at = |label: &str| symbols[label].1;
    for &(map, table, trainers) in TRAINER_HEADERS {
        let (bank, address) = symbols[table];
        let rom: Vec<&[u8]> = rom_slice(DmgPointer { bank: DmgBank::ROM { bank }, address })
            .chunks(TRAINER_STRUCT_SIZE).take_while(|row| row[0] != 0xFF).collect();
        assert_eq!(rom.len(), trainers.len(), "{map}: {table}");
        assert_eq!(TrainerRef::named(table).map(|first| first.header().label), trainers.first().map(|first| first.label), "{table}");
        for (i, (row, trainer)) in rom.iter().zip(trainers).enumerate() {
            let word = |at: usize| u16::from_le_bytes([row[at], row[at + 1]]);
            let event = (word(2) - sym::wEventFlags.address) * 8 + row[0] as u16;
            assert_eq!(
                (address + (i * TRAINER_STRUCT_SIZE) as u16, row[0], row[1] >> 4, event),
                (at(trainer.label), trainer.sprite, trainer.range, trainer.event),
                "{map}: {}", trainer.label,
            );
            assert_eq!(
                [word(4), word(6), word(8), word(10)],
                [trainer.before_battle, trainer.after_battle, trainer.end_battle, trainer.end_battle].map(at),
                "{map}: {}", trainer.label,
            );
            assert_eq!(TrainerRef::named(trainer.label).map(|header| header.header()), Some(trainer));
        }
    }
}

/// Every `script_*` text is its `TX_SCRIPT_*` byte, and a mart's the stock after it.
#[test]
fn text_dispatches() {
    let symbols = symbol_table();
    for &(label, dispatch) in TEXT_DISPATCHES {
        let (bank, address) = symbols[label];
        let rom = rom_slice(DmgPointer { bank: DmgBank::ROM { bank }, address });
        let expected: Vec<u8> = match dispatch {
            TextDispatch::PokecenterNurse => vec![0xFF],
            TextDispatch::Mart(stock) => [0xFE, stock.len() as u8].into_iter().chain(stock.iter().copied()).chain([0xFF]).collect(),
            TextDispatch::BillsPc => vec![0xFD],
            TextDispatch::PlayersPc => vec![0xFC],
            TextDispatch::PokecenterPc => vec![0xF9],
            TextDispatch::PrizeVendor => vec![0xF7],
            TextDispatch::CableClubReceptionist => vec![0xF6],
            TextDispatch::VendingMachine => vec![0xF5],
        };
        assert_eq!(rom[..expected.len()], expected[..], "{label}");
    }
}

/// `TextPredefs` points at each text in the enum's order, and the `db_tx_pre` tables hold the ids
/// the enum gives.
#[test]
fn text_predefs() {
    let symbols = symbol_table();
    let rom = rows(sym::TextPredefs, 2, TextPredef::ALL.len());
    for (row, predef) in rom.iter().zip(TextPredef::ALL) {
        assert_eq!(u16::from_le_bytes([row[0], row[1]]), symbols[predef.label()].1, "{predef:?}");
    }
    let rows_of = |table: &[(u8, u8, TextPredef)]| table.iter().map(|&(a, b, text)| (a, b, text as u8)).collect::<Vec<_>>();
    assert_eq!(terminated(sym::BookshelfTileIDs, 3).iter().map(|row| three(row)).collect::<Vec<_>>(), rows_of(&BOOKSHELF_TILE_IDS));
    assert_eq!(terminated(sym::BenchGuyTextPointers, 3).iter().map(|row| three(row)).collect::<Vec<_>>(), rows_of(&BENCH_GUY_TEXTS));
}

/// Past a wrong facing the cartridge's scan of `BenchGuyTextPointers` goes on a byte out of step
/// and off the table's end; for every map and facing, what it finds is what a scan of the table
/// alone finds.
#[test]
fn bench_guy_scan_finds_nothing_past_the_table() {
    use pokered::systems::events::hidden_events::bench_guy_text;
    let rom = rom_slice(sym::BenchGuyTextPointers);
    for map in poke_core::map::Map::all() {
        for facing in [0, 4, 8, 12] {
            let (mut i, mut found) = (0, None);
            while let Some(&m) = rom.get(i) {
                i += 1;
                if m == 0xFF {
                    break;
                }
                if m != map as u8 {
                    i += 2;
                    continue;
                }
                let wanted = rom[i];
                i += 1;
                if wanted == facing {
                    found = Some(rom[i]);
                    break;
                }
            }
            assert_eq!(bench_guy_text(map, facing, Ruleset::Gen1).map(|text| text as u8), found, "{map:?} facing {facing}");
        }
    }
}

/// `HiddenEventMaps`, `HiddenEventPointers` and every map's rows, each routine where its label is.
#[test]
fn hidden_events() {
    let symbols = symbol_table();
    let maps: Vec<u8> = rom_slice(sym::HiddenEventMaps).iter().copied().take_while(|&map| map != 0xFF).collect();
    assert_eq!(maps, HIDDEN_EVENTS.iter().map(|&(map, _)| map).collect::<Vec<_>>());
    for (i, &(map, events)) in HIDDEN_EVENTS.iter().enumerate() {
        let pointer = rows(sym::HiddenEventPointers + 2 * i as u16, 2, 1)[0];
        let table = DmgPointer { bank: sym::HiddenEventPointers.bank, address: u16::from_le_bytes([pointer[0], pointer[1]]) };
        let rom = terminated(table, 6);
        assert_eq!(rom.len(), events.len(), "map {map}");
        for (row, event) in rom.iter().zip(events) {
            let (bank, address) = symbols[event.routine.label()];
            assert_eq!(row, &[event.y, event.x, event.argument, bank, address as u8, (address >> 8) as u8], "map {map}: {event:?}");
        }
    }
}

/// A `dw` in `table`'s bank, as a pointer.
fn word_at(table: DmgPointer, at: usize) -> DmgPointer {
    let bytes = &rom_slice(table)[at..at + 2];
    DmgPointer { address: u16::from_le_bytes([bytes[0], bytes[1]]), ..table }
}

/// Three pointers to five text pointers each.
#[test]
fn in_game_trade_texts() {
    let symbols = symbol_table();
    for (set, texts) in IN_GAME_TRADE_TEXTS.iter().enumerate() {
        let pointers = word_at(sym::InGameTradeTextPointers, 2 * set);
        for (i, text) in texts.iter().enumerate() {
            assert_eq!(word_at(pointers, 2 * i).address, symbols[*text].1, "{text}");
        }
    }
}

/// A guide's squares, `y, x` and a pointer to presses ending in `$FF`, as many as the cartridge
/// reads of each.
#[test]
fn pewter_guys_coords() {
    for (guy, squares) in PEWTER_GUYS_COORDS.iter().enumerate() {
        let table = word_at(sym::PewterGuysCoordsTable, 2 * guy);
        for (i, &((x, y), presses)) in squares.iter().enumerate() {
            let row = &rom_slice(table)[4 * i..4 * i + 4];
            assert_eq!((row[0], row[1]), (y, x));
            let moves = rom_slice(word_at(table, 4 * i + 2));
            assert_eq!(moves[..presses.len() + 1], [presses, &[0xFF]].concat(), "({x}, {y})");
        }
    }
}

/// A window's prizes to a `@`, and its costs as two BCD bytes each.
#[test]
fn prize_windows() {
    for (window, (prizes, costs)) in PRIZE_WINDOWS.iter().enumerate() {
        let [entries, cost_bytes] = [0, 2].map(|at| rom_slice(word_at(sym::PrizeDifferentMenuPtrs, 4 * window + at)));
        assert_eq!(entries[..4], [&prizes[..], &[0x50]].concat());
        let bcd = |bytes: &[u8]| bytes.iter().fold(0u16, |n, &b| n * 100 + (b >> 4) as u16 * 10 + (b & 0xF) as u16);
        assert_eq!(cost_bytes.chunks(2).take(3).map(bcd).collect::<Vec<_>>(), costs);
        assert_eq!(cost_bytes[6], 0x50);
    }
}

/// `half_circle`: the quadrant, the circle data's pointer and a `wTileMap` address.
#[test]
fn battle_transition_half_circles() {
    for (table, half) in [sym::BattleTransition_HalfCircle1, sym::BattleTransition_HalfCircle2].into_iter().zip(&BATTLE_TRANSITION_HALF_CIRCLES) {
        for (i, (row, step)) in rows(table, 5, 10).into_iter().zip(half).enumerate() {
            assert_eq!(row[0] != 0, step.right);
            let data = rom_slice(word_at(table, 5 * i + 1));
            assert_eq!(data[..step.runs.len() + 1], [step.runs, &[0xFF]].concat());
            let at = u16::from_le_bytes([row[3], row[4]]) - sym::wTileMap.address;
            assert_eq!((at % 20, at / 20), (step.x as u16, step.y as u16));
        }
    }
}

/// `map_coord_movement` rows to a `$FF`, each pointing at its `(press, count)` pairs and a `$FF`.
#[test]
fn arrow_tile_player_movement() {
    for (table, arrows) in [
        (sym::RocketHideout2ArrowTilePlayerMovement, ROCKET_HIDEOUT_B2F_ARROWS),
        (sym::RocketHideout3ArrowTilePlayerMovement, ROCKET_HIDEOUT_B3F_ARROWS),
        (sym::ViridianGymArrowTilePlayerMovement, VIRIDIAN_GYM_ARROWS),
    ] {
        let rom = terminated(table, 4);
        assert_eq!(rom.len(), arrows.len(), "{table}");
        for (i, (row, &((x, y), list))) in rom.into_iter().zip(arrows).enumerate() {
            assert_eq!((row[0], row[1]), (y, x));
            let pairs = rom_slice(word_at(table, 4 * i + 2));
            assert_eq!(pairs[..2 * list.len() + 1], [&list.iter().flat_map(|&(press, count)| [press, count]).collect::<Vec<_>>()[..], &[0xFF]].concat());
        }
    }
}

/// Every run-length list by the label the cartridge reads it at.
#[test]
fn rle_lists() {
    use crate::pokemon::symbols::pokered_local_labels::{SeafoamIslandsB3FMoveObjectScript as B3F, SeafoamIslandsB4FMoveObjectScript as B4F, ViridianMartDefaultScript};
    use poke_core::tables::rle_lists::*;
    for (at, list) in [
        (sym::RLEList_ProfOakWalkToLab, PROF_OAK_WALK_TO_LAB),
        (sym::RLEList_PlayerWalkToLab, PLAYER_WALK_TO_LAB),
        (sym::RLEList_PewterMuseumPlayer, PEWTER_MUSEUM_PLAYER),
        (sym::RLEList_PewterMuseumGuy, PEWTER_MUSEUM_GUY),
        (sym::RLEList_PewterGymPlayer, PEWTER_GYM_PLAYER),
        (sym::RLEList_PewterGymGuy, PEWTER_GYM_GUY),
        (sym::PlayerEntryMovementRLE, OAKS_LAB_PLAYER_ENTRY),
        (ViridianMartDefaultScript::PlayerMovement, VIRIDIAN_MART_PLAYER),
        (sym::RLEList_ForcedSurfingStrongCurrentNearSteps, SEAFOAM_ISLANDS_B3F_NEAR_STEPS),
        (B3F::RLEList_StrongCurrentNearRightBoulder, SEAFOAM_ISLANDS_B3F_NEAR_RIGHT_BOULDER),
        (B3F::RLEList_StrongCurrentNearLeftBoulder, SEAFOAM_ISLANDS_B3F_NEAR_LEFT_BOULDER),
        (B4F::RLEList_StrongCurrentNearRightBoulder, SEAFOAM_ISLANDS_B4F_NEAR_RIGHT_BOULDER),
        (B4F::RLEList_StrongCurrentNearLeftBoulder, SEAFOAM_ISLANDS_B4F_NEAR_LEFT_BOULDER),
    ] {
        let pairs: Vec<u8> = list.iter().flat_map(|&(value, count)| [value, count]).collect();
        assert_eq!(rom_slice(at)[..pairs.len() + 1], [&pairs[..], &[0xFF]].concat(), "{at}");
    }
}

/// Four-byte rows of two pointers, a frame's tile ids and its OAM layout, `y, x, attributes`.
#[test]
fn sprite_facing_and_animation_table() {
    let table = sym::SpriteFacingAndAnimationTable;
    for (row, (tiles, oam)) in SPRITE_FACING_AND_ANIMATION_TABLE.iter().enumerate() {
        assert_eq!(rom_slice(word_at(table, 4 * row))[..4], **tiles, "row {row}");
        assert_eq!(rom_slice(word_at(table, 4 * row + 2))[..12], *oam.concat(), "row {row}");
    }
}

#[test]
fn title_screen_pokemon_logo_y_scrolls() {
    use crate::pokemon::symbols::pokered_local_labels::DisplayTitleScreen;
    let pairs: Vec<u8> = TITLE_SCREEN_POKEMON_LOGO_Y_SCROLLS.iter().flat_map(|&(d, times)| [d as u8, times]).collect();
    assert_eq!(rom_slice(DisplayTitleScreen::TitleScreenPokemonLogoYScrolls)[..pairs.len() + 1], [&pairs[..], &[0]].concat());
}

/// `diploma_text`: the string's pointer and a `wTileMap` address.
#[test]
fn diploma_texts() {
    let table = sym::DiplomaTextPointersAndCoords;
    for (i, &((x, y), text)) in DIPLOMA_TEXTS.iter().enumerate() {
        let bytes = Chars::encode(text);
        assert_eq!(rom_slice(word_at(table, 4 * i))[..bytes.len()], bytes, "({x}, {y})");
        let at = u16::from_le_bytes(rom_slice(table)[4 * i + 2..4 * i + 4].try_into().unwrap()) - sym::wTileMap.address;
        assert_eq!((at % 20, at / 20), (x as u16, y as u16));
    }
}

/// `dbw`: the count below which a rating applies and its text; and the sound's counts to a `$FF`.
#[test]
fn dex_ratings() {
    let symbols = symbol_table();
    for (row, &(below, text)) in rows(sym::DexRatingsTable, 3, DEX_RATINGS.len()).into_iter().zip(&DEX_RATINGS) {
        assert_eq!((row[0], u16::from_le_bytes([row[1], row[2]])), (below, symbols[text].1), "{text}");
    }
    check_list(sym::OwnedMonValues, 1, 0xFF, OWNED_MON_VALUES, one);
}

/// `MonPartyData`, a nybble a dex number, the odd ones high.
#[test]
fn mon_party_data() {
    let rom = rom_slice(sym::MonPartyData);
    let nybbles: Vec<u8> = (0..MON_PARTY_DATA.len()).map(|i| if i % 2 == 0 { rom[i / 2] >> 4 } else { rom[i / 2] & 0xF }).collect();
    assert_eq!(nybbles, MON_PARTY_DATA);
}

/// `mon_icon_header`: the tiles' pointer, their count, their bank and the `vSprites` address they go
/// to; a row whose read runs on into the next picture is more than one of the recreation's.
#[test]
fn mon_party_sprites() {
    use poke_core::gfx::MON_PARTY_SPRITES;
    let mut ours = MON_PARTY_SPRITES.iter().peekable();
    for row in rows(sym::MonPartySpritePointers, 6, 28) {
        let source = DmgPointer { bank: DmgBank::ROM { bank: row[3] }, address: u16::from_le_bytes([row[0], row[1]]) };
        let mut expected = Vec::new();
        let mut destination = None;
        while expected.len() < row[2] as usize * 16 {
            let &(picture, first, count, to) = ours.next().expect("a piece for every row");
            destination.get_or_insert(to);
            expected.extend_from_slice(&picture[first * 16..(first + count) * 16]);
        }
        assert_eq!(&rom_slice(source)[..expected.len()], expected);
        assert_eq!(u16::from_le_bytes([row[4], row[5]]), 0x8000 + destination.unwrap() as u16 * 16);
    }
    assert!(ours.next().is_none());
}

/// Each step a byte, speed and frames as nybbles, to a 0.
#[test]
fn title_scrolls() {
    check_list(sym::TitleScroll_Out, 1, 0, TITLE_SCROLL_OUT, one);
    check_list(sym::TitleScroll_In, 1, 0, TITLE_SCROLL_IN, one);
    check_list(sym::TitleScroll_WaitBall, 1, 0, TITLE_SCROLL_WAIT_BALL, one);
}

/// `TypeNames`: a pointer a type, the unused ids' at `NORMAL`, each to a `@`-ended name.
#[test]
fn type_names() {
    for (id, name) in TYPE_NAMES.iter().enumerate() {
        let bytes = poke_core::charmap::encode(&format!("{name}@")).unwrap();
        assert_eq!(rom_slice(word_at(sym::TypeNames, 2 * id))[..bytes.len()], bytes, "{name}");
    }
}

#[test]
fn title_screen_strings() {
    use crate::pokemon::symbols::pokered_local_labels::DisplayTitleScreen;
    assert_eq!(between(DisplayTitleScreen::tileScreenCopyrightTiles, DisplayTitleScreen::tileScreenCopyrightTilesEnd), TITLE_SCREEN_COPYRIGHT_TILES);
    for (at, string) in [(sym::VersionOnTitleScreenText, VERSION_ON_TITLE_SCREEN_TEXT), (sym::CopyrightTextString, COPYRIGHT_TEXT_STRING)] {
        let bytes = Chars::encode(string);
        assert_eq!(rom_slice(at)[..bytes.len()], bytes, "{at}");
    }
}

/// Six pointers, each to four `y, x` rows or to a lone `$FF`.
#[test]
fn small_stars_wave_coords() {
    for (wave, coords) in SMALL_STARS_WAVE_COORDS.iter().enumerate() {
        let rom = rom_slice(word_at(sym::SmallStarsWaveCoordsPointerTable, 2 * wave));
        match coords.len() {
            0 => assert_eq!(rom[0], 0xFF),
            _ => assert_eq!(rom[..8], *coords.concat(), "wave {wave}"),
        }
    }
}

/// `dy, dx` pairs to `ANIMATION_END`.
#[test]
fn intro_nidorino_animations() {
    let tables = [sym::IntroNidorinoAnimation1, sym::IntroNidorinoAnimation2, sym::IntroNidorinoAnimation3, sym::IntroNidorinoAnimation4,
        sym::IntroNidorinoAnimation5, sym::IntroNidorinoAnimation6, sym::IntroNidorinoAnimation7];
    for (table, pairs) in tables.into_iter().zip(INTRO_NIDORINO_ANIMATIONS) {
        let bytes: Vec<u8> = pairs.iter().flat_map(|&(dy, dx)| [dy as u8, dx as u8]).collect();
        assert_eq!(rom_slice(table)[..bytes.len() + 1], [&bytes[..], &[80]].concat(), "{table}");
    }
}

/// `outdoor_map` and `indoor_map`: `y, x` as nybbles and the name's pointer, the indoor rows after
/// the group each ends before.
#[test]
fn town_map_entries() {
    assert_eq!(between(sym::TownMapOrder, sym::TownMapOrderEnd), TOWN_MAP_ORDER);
    let name = |at: DmgPointer, name: &str| {
        let bytes = poke_core::charmap::encode(&format!("{name}@")).unwrap();
        assert_eq!(rom_slice(at)[..bytes.len()], bytes, "{name}");
    };
    for (row, &((x, y), text)) in rows(sym::ExternalMapEntries, 3, EXTERNAL_MAP_ENTRIES.len()).into_iter().zip(&EXTERNAL_MAP_ENTRIES) {
        assert_eq!(row[0], y << 4 | x, "{text}");
        name(DmgPointer { address: u16::from_le_bytes([row[1], row[2]]), ..sym::ExternalMapEntries }, text);
    }
    let rom = terminated(sym::InternalMapEntries, 4);
    assert_eq!(rom.len(), INTERNAL_MAP_ENTRIES.len());
    for (row, &(group, (x, y), text)) in rom.into_iter().zip(&INTERNAL_MAP_ENTRIES) {
        assert_eq!((row[0], row[1]), (group, y << 4 | x), "{text}");
        name(DmgPointer { address: u16::from_le_bytes([row[2], row[3]]), ..sym::InternalMapEntries }, text);
    }
}

/// `fly_warp`: `event_displacement`, a `wOverworldMap` address then `y, x`, and the sub-block bits.
fn check_fly_warp(row: &[u8], warp: (u8, u8, u8)) {
    let to = poke_core::map_objects::WarpTo::fly_warp(warp);
    assert_eq!(u16::from_le_bytes([row[0], row[1]]), sym::wOverworldMap.address + to.view, "{warp:?}");
    assert_eq!(row[2..6], [warp.2, warp.1, warp.2 & 1, warp.1 & 1], "{warp:?}");
}

#[test]
fn fly_and_dungeon_warps() {
    for (row, &warp) in rows(sym::DungeonWarpData, 6, DUNGEON_WARP_DATA.len()).into_iter().zip(&DUNGEON_WARP_DATA) {
        check_fly_warp(row, warp);
    }
    for (i, &warp) in FLY_WARP_DATA.iter().enumerate() {
        assert_eq!(rom_slice(sym::FlyWarpDataPtr)[4 * i..4 * i + 2], [warp.0, 0]);
        check_fly_warp(rom_slice(word_at(sym::FlyWarpDataPtr, 4 * i + 2)), warp);
    }
}

#[test]
fn slot_machine_wheels() {
    for (table, words) in [sym::SlotMachineWheel1, sym::SlotMachineWheel2, sym::SlotMachineWheel3].into_iter().zip(&SLOT_MACHINE_WHEELS) {
        let rom: Vec<u16> = rom_slice(table).chunks(2).take(18).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        assert_eq!(rom, words, "{table}");
    }
}

/// The battle animation tables through their pointers, each entry as long as its own count says.
#[test]
fn battle_animations() {
    use poke_core::battle_anims::AnimCommand;
    let bank = |address: u16| rom_slice(DmgPointer { address, ..sym::AttackAnimationPointers });
    for (id, commands) in ATTACK_ANIMATIONS.iter().enumerate() {
        let bytes: Vec<u8> = commands.iter().flat_map(|command| match *command {
            AnimCommand::SpecialEffect { id, sound } => vec![id, sound],
            AnimCommand::Subanimation { tileset, delay, sound, id } => vec![tileset << 6 | delay, sound, id],
        }).collect();
        assert_eq!(bank(word_at(sym::AttackAnimationPointers, 2 * id).address)[..bytes.len() + 1], [&bytes[..], &[0xFF]].concat(), "animation {}", id + 1);
    }
    for (id, &(kind, entries)) in SUBANIMATIONS.iter().enumerate() {
        let rom = bank(word_at(sym::SubanimationPointers, 2 * id).address);
        assert_eq!((rom[0] >> 5, (rom[0] & 0x1F) as usize), (kind, entries.len()), "subanimation {id}");
        assert_eq!(rom[1..1 + 3 * entries.len()], *entries.concat(), "subanimation {id}");
    }
    for (id, tiles) in FRAME_BLOCKS.iter().enumerate() {
        let rom = bank(word_at(sym::FrameBlockPointers, 2 * id).address);
        assert_eq!(rom[0] as usize, tiles.len(), "frame block {id}");
        assert_eq!(rom[1..1 + 4 * tiles.len()], *tiles.concat(), "frame block {id}");
    }
    assert_eq!(rows(sym::FrameBlockBaseCoords, 2, FRAME_BLOCK_BASE_COORDS.len()).into_iter().map(two).collect::<Vec<_>>(), FRAME_BLOCK_BASE_COORDS);
}

/// Every label `pokered.sym` puts in ROM bank `bank`, by name.
fn bank_symbols(bank: u8) -> std::collections::HashMap<&'static str, u16> {
    include_str!("../../vendor/pokered/pokered.sym").lines().filter_map(|line| {
        let (at, name) = line.split_once(' ')?;
        let (in_bank, address) = at.split_once(':')?;
        let address = u16::from_str_radix(address, 16).ok()?;
        (u8::from_str_radix(in_bank, 16).ok()? == bank).then_some((name, address))
    }).collect()
}

/// Each copy of the sound data is in the bank that holds the cartridge's, and every label in it is
/// one of the cartridge's: at the same address up to the engine's code, and moved after it.
#[test]
fn audio_labels() {
    use poke_core::audio::AUDIO_BANKS;
    for (n, data) in AUDIO_BANKS.iter().enumerate() {
        let symbols = bank_symbols(data.rom_bank);
        assert_eq!(symbols.get(format!("SFX_Headers_{}", n + 1).as_str()), Some(&0x4000), "audio {} is in bank {:02X}", n + 1, data.rom_bank);
        let cry_ret = data.label(&format!("Audio{}_CryRet", n + 1)).expect("the engine's one stream");
        for &(name, at) in data.labels {
            let theirs = *symbols.get(name).unwrap_or_else(|| panic!("the cartridge has no {name} in bank {:02X}", data.rom_bank));
            if at < cry_ret {
                assert_eq!(at, theirs, "{name}");
            }
        }
        assert_eq!(data.labels[0].1, 0x4000, "the first byte is labelled");
    }
}

/// Every byte of each copy against the cartridge's, a label's run at a time from where the
/// cartridge has that label; a pointer is compared by the label it names.
#[test]
fn audio_bytes() {
    use poke_core::audio::AUDIO_BANKS;
    use poke_core::symbols::DmgBank;
    for data in &AUDIO_BANKS {
        let symbols = bank_symbols(data.rom_bank);
        let window = rom_slice(DmgPointer { bank: DmgBank::ROM { bank: data.rom_bank }, address: 0x4000 });
        let named: std::collections::HashMap<u16, &str> = data.labels.iter().map(|&(name, at)| (at, name)).collect();
        let pointers: std::collections::HashSet<u16> = data.pointers.iter().copied().collect();
        let end = 0x4000 + data.bytes.len() as u16;
        let mut compared = 0;
        for (i, &(name, at)) in data.labels.iter().enumerate() {
            if i > 0 && data.labels[i - 1].1 == at {
                continue;
            }
            let next = data.labels[i + 1..].iter().map(|&(_, next)| next).find(|&next| next > at).unwrap_or(end);
            let theirs = symbols[name];
            let mut k = 0;
            while at + k < next {
                let (ours, rom) = (&data.bytes[(at + k - 0x4000) as usize..], &window[(theirs + k - 0x4000) as usize..]);
                if pointers.contains(&(at + k)) {
                    let target = named[&u16::from_le_bytes([ours[0], ours[1]])];
                    assert_eq!(u16::from_le_bytes([rom[0], rom[1]]), symbols[target], "{name}+{k}: a pointer to {target}");
                    k += 2;
                } else {
                    assert_eq!(ours[0], rom[0], "{name}+{k} in bank {:02X}", data.rom_bank);
                    k += 1;
                }
            }
            compared += (next - at) as usize;
        }
        assert_eq!(compared, data.bytes.len());
    }
}

/// The pitch table, the cries, and the three tables that name a sound with its bank.
#[test]
fn sound_tables() {
    use poke_core::audio::{sound, CRY_DATA, MAP_SONG_BANKS, MOVE_SOUND_TABLE, PITCHES, POKEDEX_RATING_SFX};
    for pitches in [sym::Audio1_Pitches, sym::Audio2_Pitches, sym::Audio3_Pitches] {
        let rom: Vec<u16> = rows(pitches, 2, PITCHES.len()).iter().map(|row| u16::from_le_bytes([row[0], row[1]])).collect();
        assert_eq!(rom, PITCHES);
    }
    let rom: Vec<_> = rows(sym::CryData, 3, CRY_DATA.len()).iter().map(|row| (sound::CRY_SFX_START + 3 * row[0], row[1], row[2])).collect();
    assert_eq!(rom, CRY_DATA);
    assert_eq!(rows(sym::MapSongBanks, 2, MAP_SONG_BANKS.len()).into_iter().map(two).collect::<Vec<_>>(), MAP_SONG_BANKS);
    assert_eq!(rows(sym::PokedexRatingSfxPointers, 2, POKEDEX_RATING_SFX.len()).into_iter().map(two).collect::<Vec<_>>(), POKEDEX_RATING_SFX);
    assert_eq!(rows(sym::MoveSoundTable, 3, MOVE_SOUND_TABLE.len()).into_iter().map(three).collect::<Vec<_>>(), MOVE_SOUND_TABLE);
    let id = |header: DmgPointer| ((header.address - 0x4000) / 3) as u8;
    for (source, header) in [
        (sound::NOISE_INSTRUMENTS_END, id(sym::SFX_Noise_Instrument19_1) + 1),
        (sound::CRY_SFX_START, id(sym::SFX_Cry00_1)),
        (sound::CRY_SFX_END, id(sym::SFX_Cry25_1) + 3),
        (sound::BATTLE_SFX_START, id(sym::SFX_Peck)),
        (sound::BATTLE_SFX_END, id(sym::SFX_Trainer_Appeared) + 1),
        (sound::MAX_SFX_ID_1, id(sym::SFX_Safari_Zone_PA)),
        (sound::MAX_SFX_ID_2, id(sym::SFX_Trainer_Appeared)),
        (sound::MAX_SFX_ID_3, id(sym::SFX_Shooting_Star)),
        (sound::MUSIC_PALLET_TOWN, id(sym::Music_PalletTown)),
        (sound::MUSIC_MEET_MALE_TRAINER, id(sym::Music_MeetMaleTrainer)),
    ] {
        assert_eq!(source, header);
    }
}

/// Where the `.sym` puts `label`, as a ROM pointer.
fn labelled(symbols: &HashMap<String, (u8, u16)>, label: &str) -> DmgPointer {
    let &(bank, address) = symbols.get(label).unwrap_or_else(|| panic!("{label} is not in the .sym"));
    DmgPointer { bank: poke_core::symbols::DmgBank::ROM { bank }, address }
}

/// Every `db` string by the label the cartridge places it from.
#[test]
fn db_strings() {
    let symbols = symbol_table();
    for &(label, chars) in &DB_STRINGS {
        let bytes = Chars::encode(chars);
        assert_eq!(rom_slice(labelled(&symbols, label))[..bytes.len()], bytes, "{label}");
    }
}

/// `NEW NAME` and the three names, each ended by a `@`.
#[test]
fn default_names_lists() {
    for (table, names) in [(sym::DefaultNamesPlayerList, DEFAULT_NAMES_PLAYER_LIST), (sym::DefaultNamesRivalList, DEFAULT_NAMES_RIVAL_LIST)] {
        let bytes: Vec<u8> = names.iter().flat_map(|name| poke_core::charmap::encode(&format!("{name}@")).unwrap()).collect();
        assert_eq!(rom_slice(table)[..bytes.len()], bytes, "{table}");
    }
}

/// A pointer a credit, each to its column offset and a `@`-ended string.
#[test]
fn credits_texts() {
    for (i, &(offset, text)) in CREDITS_TEXTS.iter().enumerate() {
        let bytes = [&[offset as u8][..], &poke_core::charmap::encode(&format!("{text}@")).unwrap()].concat();
        assert_eq!(rom_slice(word_at(sym::CreditsTextPointers, 2 * i))[..bytes.len()], bytes, "{text}");
    }
}

/// `dw` tables of texts, each pointer where the `.sym` puts its label.
#[test]
fn text_pointer_lists() {
    let symbols = symbol_table();
    for (table, labels) in [
        (sym::CinnabarQuizQuestions, &CINNABAR_QUIZ_QUESTIONS[..]),
        (sym::LinkCableInfoTexts, &LINK_CABLE_INFO_TEXTS[..]),
        (sym::ViridianBlackboardStatusPointers, &VIRIDIAN_BLACKBOARD_STATUS_TEXTS[..]),
    ] {
        for (i, label) in labels.iter().enumerate() {
            assert_eq!(word_at(table, 2 * i), labelled(&symbols, label), "{table} row {i}");
        }
    }
}

/// `gym_gate_coord`: `x, y, block` and a padding 0.
#[test]
fn cinnabar_gym_gate_coords() {
    let rom: Vec<_> = rows(sym::CinnabarGymGateCoords, 4, 6).into_iter().map(|row| {
        assert_eq!(row[3], 0);
        (row[0], row[1], row[2])
    }).collect();
    assert_eq!(rom, CINNABAR_GYM_GATE_COORDS);
}

#[test]
fn animation_coordinate_rows() {
    for (table, source) in [
        (sym::CutAnimationOffsets, CUT_ANIMATION_OFFSETS),
        (sym::BoulderDustAnimationOffsets, BOULDER_DUST_ANIMATION_OFFSETS),
        (sym::FlyAnimationScreenCoords1, FLY_ANIMATION_SCREEN_COORDS_1),
        (sym::FlyAnimationScreenCoords2, FLY_ANIMATION_SCREEN_COORDS_2),
        (sym::FlyAnimationEnterScreenCoords, FLY_ANIMATION_ENTER_SCREEN_COORDS),
    ] {
        assert_eq!(rows(table, 2, source.len()).into_iter().map(two).collect::<Vec<_>>(), source, "{table}");
    }
}

/// `hidden_item` and `hidden_coin` store `map, y, x`.
#[test]
fn terminated_lists_read_last() {
    check_list(sym::TilePairCollisionsLand, 3, 0xFF, TILE_PAIR_COLLISIONS_LAND, three);
    check_list(sym::TilePairCollisionsWater, 3, 0xFF, TILE_PAIR_COLLISIONS_WATER, three);
    check_list(sym::HiddenItemCoords, 3, 0xFF, HIDDEN_ITEM_COORDS, |row| (row[0], row[2], row[1]));
    check_list(sym::HiddenCoinCoords, 3, 0xFF, HIDDEN_COIN_COORDS, |row| (row[0], row[2], row[1]));
    check_list(sym::FemaleTrainerList, 1, 0xFF, FEMALE_TRAINER_LIST, one);
    check_list(sym::EvilTrainerList, 1, 0xFF, EVIL_TRAINER_LIST, one);
}

#[test]
fn minimized_mon_sprite() {
    assert_eq!(between(sym::MinimizedMonSprite, sym::MinimizedMonSpriteEnd), MINIMIZED_MON_SPRITE);
}

#[test]
fn walk_to_lance() {
    let pairs: Vec<u8> = rle_lists::WALK_TO_LANCE.iter().flat_map(|&(value, count)| [value, count]).collect();
    assert_eq!(rom_slice(sym::WalkToLance_RLEList)[..pairs.len() + 1], [&pairs[..], &[0xFF]].concat());
}

/// The three bubbles in `EmotionBubblesPointerTable`'s order, one picture after another.
#[test]
fn emotion_bubbles() {
    use poke_core::gfx::emotes::{HAPPY, QUESTION, SHOCK};
    let bubbles = [&SHOCK[..], &QUESTION[..], &HAPPY[..]].concat();
    assert_eq!(rom_slice(sym::EmotionBubbles)[..bubbles.len()], bubbles);
    for (i, bubble) in [SHOCK, QUESTION, HAPPY].iter().enumerate() {
        assert_eq!(rom_slice(word_at(sym::EmotionBubblesPointerTable, 2 * i))[..bubble.len()], bubble[..], "bubble {i}");
    }
}

/// The species line to a `@`, feet and inches, the weight as a word, then `text_far` and
/// `text_end`. MissingNo's entry is written another way and is left out.
#[test]
fn dex_entries() {
    let symbols = symbol_table();
    for (i, entry) in DEX_ENTRIES.iter().enumerate() {
        let rom = rom_slice(word_at(sym::PokedexEntryPointers, 2 * i));
        let Some(entry) = entry else {
            assert_eq!(word_at(sym::PokedexEntryPointers, 2 * i), sym::MissingNoDexEntry);
            continue;
        };
        let text = labelled(&symbols, entry.text);
        let poke_core::symbols::DmgBank::ROM { bank } = text.bank else { unreachable!() };
        let bytes = [
            &poke_core::charmap::encode(&format!("{}@", entry.species)).unwrap()[..],
            &[entry.feet, entry.inches], &entry.weight.to_le_bytes(),
            &[0x17], &text.address.to_le_bytes(), &[bank, 0x50],
        ].concat();
        assert_eq!(rom[..bytes.len()], bytes, "{}", entry.species);
    }
}

/// `ItemUseCardKey` compares the first byte of `GetTileAndCoordsInFrontOfPlayer`'s own code with
/// the door tiles, and it is none of them, which is why the recreation never opens a door with it.
#[test]
fn the_card_key_compares_a_byte_of_code_that_is_no_door() {
    assert!(![0x18, 0x24, 0x5E].contains(&rom_slice(sym::GetTileAndCoordsInFrontOfPlayer)[0]));
}

/// Every map object id `const_export` counts out is the value the `.sym` exports.
#[test]
fn map_object_ids() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/pokered/pokered.sym");
    let exported: HashMap<String, u8> = std::fs::read_to_string(path).unwrap().lines().filter_map(|line| {
        let (value, name) = line.split_once(' ')?;
        (value.len() == 2).then(|| Some((name.to_string(), u8::from_str_radix(value, 16).ok()?)))?
    }).collect();
    use poke_core::symbols::pokered_map_scripts as ours;
    for (name, value) in [
        ("PALLETTOWN_OAK", ours::PALLETTOWN_OAK), ("VIRIDIANCITY_YOUNGSTER2", ours::VIRIDIANCITY_YOUNGSTER2),
        ("PEWTERCITY_YOUNGSTER", ours::PEWTERCITY_YOUNGSTER), ("CERULEANCITY_RIVAL", ours::CERULEANCITY_RIVAL),
        ("CERULEANCITY_ROCKET", ours::CERULEANCITY_ROCKET), ("ROUTE22_RIVAL1", ours::ROUTE22_RIVAL1),
        ("ROUTE22_RIVAL2", ours::ROUTE22_RIVAL2), ("BILLSHOUSE_BILL1", ours::BILLSHOUSE_BILL1),
        ("BILLSHOUSE_BILL_POKEMON", ours::BILLSHOUSE_BILL_POKEMON), ("SSANNE2F_RIVAL", ours::SSANNE2F_RIVAL),
        ("MTMOONB2F_SUPER_NERD", ours::MTMOONB2F_SUPER_NERD), ("POKEMONTOWER2F_RIVAL", ours::POKEMONTOWER2F_RIVAL),
        ("GAMECORNER_ROCKET", ours::GAMECORNER_ROCKET), ("FIGHTINGDOJO_KARATE_MASTER", ours::FIGHTINGDOJO_KARATE_MASTER),
        ("SILPHCO7F_RIVAL", ours::SILPHCO7F_RIVAL), ("SILPHCO11F_GIOVANNI", ours::SILPHCO11F_GIOVANNI),
        ("HALLOFFAME_OAK", ours::HALLOFFAME_OAK), ("PEWTERCITY_SUPER_NERD1", ours::PEWTERCITY_SUPER_NERD1),
        ("CHAMPIONSROOM_OAK", ours::CHAMPIONSROOM_OAK), ("CHAMPIONSROOM_RIVAL", ours::CHAMPIONSROOM_RIVAL),
        ("OAKSLAB_BULBASAUR_POKE_BALL", ours::OAKSLAB_BULBASAUR_POKE_BALL), ("OAKSLAB_CHARMANDER_POKE_BALL", ours::OAKSLAB_CHARMANDER_POKE_BALL),
        ("OAKSLAB_SQUIRTLE_POKE_BALL", ours::OAKSLAB_SQUIRTLE_POKE_BALL), ("OAKSLAB_OAK1", ours::OAKSLAB_OAK1),
        ("OAKSLAB_OAK2", ours::OAKSLAB_OAK2), ("OAKSLAB_RIVAL", ours::OAKSLAB_RIVAL),
        ("CINNABARGYM_BLAINE", ours::CINNABARGYM_BLAINE), ("CINNABARGYM_SUPER_NERD3", ours::CINNABARGYM_SUPER_NERD3),
    ] {
        assert_eq!(exported[name], value, "{name}");
    }
}

/// The assets `poke_core::gfx` builds, found in the cartridge where the source `INCBIN`s them.
mod incbins {
    use crate::pokemon::roms::{POKERED, ROM_BANK_SIZE};
    use poke_core::gfx::ALL;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/pokered");

    /// Every `INCBIN` of an asset the red cartridge assembles is the ROM's bytes where its label
    /// puts it, or where the `INCBIN` before it ends.
    #[test]
    fn every_included_asset_is_the_roms_bytes() {
        let symbols = symbols();
        let assets: HashMap<&str, &[u8]> = ALL.iter().copied().collect();
        let mut files = Vec::new();
        for dir in ["data", "engine", "gfx", "home"] {
            walk(&Path::new(ROOT).join(dir), &mut files);
        }
        files.retain(|f| f.extension().is_some_and(|e| e == "asm" || e == "inc"));

        let (mut checked, mut unplaced) = (0, Vec::new());
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            let mut equs = HashMap::new();
            let (mut conditions, mut global) = (Vec::<bool>::new(), String::new());
            // Where the next byte lands, while it is known.
            let mut cursor: Option<usize> = None;
            for line in source.lines() {
                let code = line.split(';').next().unwrap().trim_end();
                if let Some(rest) = code.trim().strip_prefix("DEF ") {
                    if let Some((name, value)) = rest.split_once(" EQUS ") {
                        equs.insert(name.trim().to_string(), value.trim().trim_matches('"').to_string());
                    }
                    continue;
                }
                match code.trim() {
                    "IF DEF(_BLUE)" => { conditions.push(false); continue }
                    c if c.starts_with("IF ") => { conditions.push(true); continue }
                    "ELSE" => { if let Some(c) = conditions.last_mut() { *c = !*c } continue }
                    "ENDC" => { conditions.pop(); continue }
                    "" => continue,
                    _ => {}
                }
                let active = conditions.iter().all(|&c| c);
                let mut statement = code;
                // Labels open a line; a global one scopes the local ones after it.
                while let Some((label, rest)) = statement.split_once(':').filter(|(l, _)| is_label(l)) {
                    let name = match label.strip_prefix('.') {
                        Some(local) => format!("{global}.{local}"),
                        None => { global = label.to_string(); label.to_string() }
                    };
                    if active {
                        cursor = symbols.get(&name).copied();
                    }
                    statement = rest.trim_start_matches(':').trim();
                }
                let statement = statement.trim();
                if statement.is_empty() || !active {
                    continue;
                }
                let Some((path, operands)) = gfx_incbin(statement) else {
                    if statement.contains("INCBIN") && statement.contains("bpp\"") {
                        unplaced.push(format!("{}: {line}", file.display()));
                    }
                    cursor = None;
                    continue;
                };
                // A `.pic` is compressed in the ROM, and the rest of `gfx/` is committed verbatim.
                let Some(&asset) = assets.get(path) else {
                    cursor = None;
                    continue;
                };
                let operands = equs.get(operands).map(String::as_str).unwrap_or(operands);
                let (offset, len) = match operands.split_once(',') {
                    Some((offset, len)) => (number(offset.trim()), number(len.trim())),
                    None => (0, asset.len()),
                };
                let Some(at) = cursor else {
                    unplaced.push(format!("{}: {line}", file.display()));
                    continue;
                };
                assert!(POKERED[at..at + len] == asset[offset..offset + len], "{path} is not the ROM's bytes at {at:#x}");
                checked += 1;
                cursor = Some(at + len);
            }
        }
        assert!(unplaced.is_empty(), "INCBINs not checked:\n{}", unplaced.join("\n"));
        println!("{checked} INCBINs match the ROM");
    }

    fn is_label(text: &str) -> bool {
        let name = text.strip_prefix('.').unwrap_or(text);
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    /// `INCBIN "gfx/..."` as its path and whatever follows the path's comma.
    fn gfx_incbin(statement: &str) -> Option<(&str, &str)> {
        let quoted = statement.strip_prefix("INCBIN")?.trim_start().strip_prefix('"')?;
        let (path, rest) = quoted.split_once('"')?;
        path.starts_with("gfx/").then(|| (path, rest.trim().trim_start_matches(',').trim()))
    }

    fn number(text: &str) -> usize {
        match text.strip_prefix('$') {
            Some(hex) => usize::from_str_radix(hex, 16).unwrap(),
            None => text.parse().unwrap_or_else(|_| panic!("INCBIN operand `{text}`")),
        }
    }

    /// `pokered.sym` as label to file offset, ROM labels only.
    fn symbols() -> HashMap<String, usize> {
        let sym = std::fs::read_to_string(Path::new(ROOT).join("pokered.sym")).unwrap();
        sym.lines()
            .filter_map(|line| {
                let (place, name) = line.split_once(' ')?;
                let (bank, address) = place.split_once(':')?;
                let (bank, address) = (usize::from_str_radix(bank, 16).ok()?, usize::from_str_radix(address, 16).ok()?);
                (address < 0x8000).then(|| {
                    let window = if bank == 0 { 0 } else { ROM_BANK_SIZE };
                    (name.to_string(), bank * ROM_BANK_SIZE + address - window)
                })
            })
            .collect()
    }

    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { walk(&path, files) } else { files.push(path) }
        }
    }
}

/// Every address `SavedLabel` maps to a label is where the cartridge's symbols put that label.
#[test]
fn saved_addresses_are_the_cartridge_s() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/pokered/pokered.sym");
    let symbols: HashMap<String, DmgPointer> = std::fs::read_to_string(path).unwrap().lines().filter_map(|line| {
        let (place, name) = line.split_once(' ')?;
        let (bank, address) = place.split_once(':')?;
        let (bank, address) = (u8::from_str_radix(bank, 16).ok()?, u16::from_str_radix(address, 16).ok()?);
        (address < 0x8000).then(|| (name.to_string(), DmgPointer { bank: DmgBank::ROM { bank }, address }))
    }).collect();
    for &(at, label) in poke_core::saved_addresses::SAVED_ADDRESSES {
        assert_eq!(symbols.get(label), Some(&at), "{label}");
    }
}
