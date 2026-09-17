//! The events' rules on the cartridge: `CheckForHiddenEvent`, `PrintBookshelfText`'s lookup,
//! `ApplyOutOfBattlePoisonDamage`, `IncrementDayCareMonExp`, the day care gentleman's level and
//! price, and `HealParty`.

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_local_labels::{DaycareGentlemanText, PrintBookshelfText};
use pokered::party::{BoxMon, PartyMon};
use pokered::systems::events::day_care::Collection;
use pokered::systems::events::hidden_events::HiddenEvent;
use pokered::systems::events::poison::PoisonStep;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgBank, DmgPointer};
use super::pokemon::{decode_party, encode_party};
use super::{Called, Oracle};

const PARTYMON_STRUCT_LENGTH: usize = 0x2C;
const TEXT_MON_FAINTED: u8 = 0xD0;
const TEXT_BLACKED_OUT: u8 = 0xD1;
const BIT_SCRIPTED_MOVEMENT_STATE: u8 = 7;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

fn at(address: u16) -> DmgPointer {
    DmgPointer { address, ..sym::wBuffer }
}

/// Carries on from a stop at a routine's first instruction as though that routine had returned at
/// once: `ld sp, sp + 2` and `jp` to its return address, assembled into `wBuffer`, so everything the
/// caller pushed is still on the stack beneath.
fn skip_routine(oracle: &mut Oracle, stops: &[DmgPointer]) -> (Called, Option<DmgPointer>) {
    let sp = oracle.registers().sp;
    let ret = oracle.read(at(sp), 2);
    let [low, high] = (sp + 2).to_le_bytes();
    oracle.write(sym::wBuffer, &[0x31, low, high, 0xC3, ret[0], ret[1]]);
    oracle.call_until(sym::wBuffer, stops)
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct HiddenEventInput {
    map: Map,
    x: u8,
    y: u8,
    facing: u8,
}

fn check_for_hidden_event(oracle: &mut Oracle, input: HiddenEventInput) -> Option<HiddenEvent> {
    oracle.write(sym::wCurMap, &[input.map as u8]);
    oracle.write(sym::wXCoord, &[input.x]);
    oracle.write(sym::wYCoord, &[input.y]);
    oracle.write(sym::wSpritePlayerStateData1FacingDirection, &[input.facing]);
    oracle.write(sym::wHiddenEventFunctionArgument, &[0x55; 3]);
    let called = oracle.call(sym::CheckForHiddenEvent);
    assert!(called.rng.is_empty());
    if oracle.read(sym::hDidntFindAnyHiddenEvent, 1)[0] != 0 {
        return None;
    }
    let [argument, bank, skipped] = oracle.read(sym::wHiddenEventFunctionArgument, 3).try_into().unwrap();
    let registers = oracle.registers();
    Some(HiddenEvent {
        y: oracle.read(sym::wHiddenEventY, 1)[0],
        x: oracle.read(sym::wHiddenEventX, 1)[0],
        argument,
        function: DmgPointer { bank: DmgBank::ROM { bank }, address: u16::from_be_bytes([registers.h, registers.l]) },
        skipped,
    })
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct BookshelfInput {
    tileset: TileSetId,
    tile: u8,
    facing: u8,
}

/// The text predef `PrintBookshelfText` would print, stopped before it prints.
fn print_bookshelf_text(oracle: &mut Oracle, input: BookshelfInput) -> Option<u8> {
    oracle.write(sym::wCurMapTileset, &[input.tileset as u8]);
    oracle.write(sym::wSpritePlayerStateData1FacingDirection, &[input.facing]);
    oracle.write(sym::wTileMap + (7 * 20 + 8), &[input.tile]);
    let (_, stop) = oracle.call_until(sym::PrintBookshelfText, &[sym::PrintPredefTextID, PrintBookshelfText::noMatch]);
    match stop.expect("it prints or falls through to the card key doors") {
        stop if stop == sym::PrintPredefTextID => Some(oracle.registers().a),
        _ => None,
    }
}

fn write_party(oracle: &mut Oracle, party: &[PartyMon]) {
    oracle.write(sym::wPartyCount, &[party.len() as u8]);
    let mut species: Vec<u8> = party.iter().map(|mon| mon.mon.species as u8).collect();
    species.push(0xFF);
    oracle.write(sym::wPartySpecies, &species);
    let bytes: Vec<u8> = party.iter().flat_map(encode_party).collect();
    oracle.write(sym::wPartyMons, &bytes);
}

fn read_party(oracle: &Oracle, len: usize) -> Vec<PartyMon> {
    oracle.read(sym::wPartyMons, len * PARTYMON_STRUCT_LENGTH).chunks(PARTYMON_STRUCT_LENGTH).map(decode_party).collect()
}

fn exp_bytes(exp: u32) -> [u8; 3] {
    let [_, high, mid, low] = exp.to_be_bytes();
    [high, mid, low]
}

fn read_exp(oracle: &Oracle) -> u32 {
    let bytes = oracle.read(sym::wDayCareMonExp, 3);
    u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]])
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PoisonInput {
    party: Vec<PartyMon>,
    day_care: Option<BoxMon>,
    step_counter: u8,
    simulating: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PoisonOutput {
    /// Each mon's HP and status, all the routine writes.
    party: Vec<(u16, u8)>,
    day_care_exp: Option<u32>,
    step: PoisonStep,
}

/// `ApplyOutOfBattlePoisonDamage`, with each `DisplayTextID` and `ChangeBGPalColor0_4Frames` noted
/// and returned from at once, since both wait for frames a masked machine never gives them.
fn apply_out_of_battle_poison_damage(oracle: &mut Oracle, input: &PoisonInput) -> PoisonOutput {
    write_party(oracle, &input.party);
    oracle.write(sym::wDayCareInUse, &[input.day_care.is_some() as u8]);
    if let Some(mon) = &input.day_care {
        oracle.write(sym::wDayCareMonExp, &exp_bytes(mon.exp));
    }
    oracle.write(sym::wStepCounter, &[input.step_counter]);
    oracle.write(sym::wStatusFlags5, &[(input.simulating as u8) << BIT_SCRIPTED_MOVEMENT_STATE]);
    oracle.write(sym::wOutOfBattleBlackout, &[0x55]);
    let stops = [sym::DisplayTextID, sym::ChangeBGPalColor0_4Frames];
    let mut step = PoisonStep::default();
    let (called, mut stopped) = oracle.call_until(sym::ApplyOutOfBattlePoisonDamage, &stops);
    assert!(called.rng.is_empty());
    while let Some(stop) = stopped {
        if stop == sym::DisplayTextID {
            match oracle.read(sym::hTextID, 1)[0] {
                TEXT_MON_FAINTED => step.fainted.push(oracle.read(sym::wWhichPokemon, 1)[0]),
                TEXT_BLACKED_OUT => step.blacked_out = true,
                text => panic!("text {text:#04x}"),
            }
        } else {
            assert!(!step.flash, "one flash a step");
            step.flash = true;
        }
        stopped = skip_routine(oracle, &stops).1;
    }
    let blackout = oracle.read(sym::wOutOfBattleBlackout, 1)[0];
    assert_eq!(blackout, if step.blacked_out { 0xFF } else { 0 });
    PoisonOutput {
        party: read_party(oracle, input.party.len()).iter().map(|mon| (mon.mon.hp, mon.mon.status)).collect(),
        day_care_exp: input.day_care.as_ref().map(|_| read_exp(oracle)),
        step,
    }
}

fn increment_day_care_mon_exp(oracle: &mut Oracle, exp: u32) -> u32 {
    oracle.write(sym::wDayCareInUse, &[1]);
    oracle.write(sym::wDayCareMonExp, &exp_bytes(exp));
    oracle.call(sym::IncrementDayCareMonExp);
    read_exp(oracle)
}

/// `DaycareGentlemanText` from `.daycareInUse` to the price, stepping over the two texts on the way.
fn day_care_collection(oracle: &mut Oracle, mon: &BoxMon) -> Collection {
    const CALL: u8 = 0xCD;
    assert_eq!(rom_slice(DaycareGentlemanText::next)[0], CALL, "`.next` is `call PrintText`");
    oracle.write(sym::wDayCareInUse, &[1]);
    oracle.write(sym::wDayCareMonSpecies, &[mon.species as u8]);
    oracle.write(sym::wDayCareMonBoxLevel, &[mon.box_level]);
    oracle.write(sym::wDayCareMonExp, &exp_bytes(mon.exp));
    oracle.write(sym::wPartyCount, &[1]);
    oracle.write(sym::wDayCareTotalCost, &[0x55; 2]);
    let (_, stop) = oracle.call_until(DaycareGentlemanText::daycareInUse, &[sym::PrintText]);
    assert_eq!(stop, Some(sym::PrintText), "the grown or needs-more-time text");
    let level = oracle.read(sym::wDayCareMonBoxLevel, 1)[0];
    let exp = read_exp(oracle);
    let start_level = oracle.read(sym::wDayCareStartLevel, 1)[0];
    let levels_grown = oracle.read(sym::wDayCareNumLevelsGrown, 1)[0];
    let (_, stop) = oracle.call_until(DaycareGentlemanText::next + 3, &[sym::PrintText]);
    assert_eq!(stop, Some(sym::PrintText), "the owe-money text");
    let cost = oracle.read(sym::wDayCareTotalCost, 2).try_into().unwrap();
    Collection { level, exp, start_level, levels_grown, cost }
}

/// Each mon's HP, status and PP, all the routine writes.
fn heal_party(oracle: &mut Oracle, party: &[PartyMon]) -> Vec<(u16, u8, [u8; 4])> {
    write_party(oracle, party);
    oracle.call(sym::HealParty);
    read_party(oracle, party.len()).iter().map(|mon| (mon.mon.hp, mon.mon.status, mon.mon.pp)).collect()
}

/// A mon with only what these routines read chosen, and everything else plain.
fn a_party_mon(species: PokemonSpecies, hp: u16, max_hp: u16, status: u8, moves: [Option<poke_core::move_name::PokemonMoveName>; 4], pp: [u8; 4]) -> PartyMon {
    PartyMon {
        mon: BoxMon {
            species, hp, box_level: 50, status, types: [0, 0], catch_rate: 0, moves, ot_id: 0, exp: 0,
            stat_exp: [0; 5], dvs: pokered::systems::stats::Dvs([0, 0]), pp,
        },
        level: 50,
        stats: [max_hp, 50, 50, 50, 50],
    }
}

/// The oracle's own checks, one of each.
#[test]
fn the_oracle_answers_each_event_routine() {
    use poke_core::move_name::PokemonMoveName as Move;
    use poke_core::sprite::SpriteFacing;
    let mut oracle = oracle();
    let bush = HiddenEventInput { map: Map::ViridianForest, x: 15, y: 42, facing: SpriteFacing::Right as u8 };
    let found = check_for_hidden_event(&mut oracle, bush).expect("the Antidote's bush");
    assert_eq!((found.x, found.y, found.function, found.skipped), (16, 42, sym::HiddenItems, 1));
    assert_eq!(check_for_hidden_event(&mut oracle, HiddenEventInput { facing: SpriteFacing::Left as u8, ..bush }), None);

    let shelf = BookshelfInput { tileset: TileSetId::Pokecenter, tile: 0x54, facing: SpriteFacing::Up as u8 };
    assert_eq!(print_bookshelf_text(&mut oracle, shelf), Some(0x42));
    assert_eq!(print_bookshelf_text(&mut oracle, BookshelfInput { facing: SpriteFacing::Down as u8, ..shelf }), None);

    let poisoned = a_party_mon(PokemonSpecies::Squirtle, 1, 40, 1 << 3, [Some(Move::Tackle), None, None, None], [0; 4]);
    let input = PoisonInput { party: vec![poisoned.clone()], day_care: None, step_counter: 4, simulating: false };
    let output = apply_out_of_battle_poison_damage(&mut oracle, &input);
    assert_eq!(output.step, PoisonStep { fainted: vec![0], flash: false, blacked_out: true });

    assert_eq!(increment_day_care_mon_exp(&mut oracle, 0x4F_FFFF), 0x50_0000);

    let boxed = BoxMon { box_level: 5, exp: pokered::systems::experience::calc_experience(3, 7), ..poisoned.mon.clone() };
    let collected = day_care_collection(&mut oracle, &boxed);
    assert_eq!((collected.level, collected.levels_grown, collected.cost), (7, 2, [0x03, 0x00]));

    let healed = heal_party(&mut oracle, &[poisoned]);
    assert_eq!(healed, [(40, 0, [35, 0, 0, 0])]);
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use poke_core::base_stats::BaseStats;
    use poke_core::move_name::PokemonMoveName;
    use poke_core::sprite::SpriteFacing;
    use pokered::systems::events::hidden_events::hidden_events;
    use pokered::systems::experience::calc_experience;
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};
    use strum::IntoEnumIterator;
    use super::super::{write_fixture, Case};
    use super::*;

    const FACINGS: [u8; 4] = [SpriteFacing::Down as u8, SpriteFacing::Up as u8, SpriteFacing::Left as u8, SpriteFacing::Right as u8];

    fn species(rng: &mut StdRng) -> PokemonSpecies {
        let all: Vec<_> = PokemonSpecies::iter().collect();
        all[rng.random_range(0..all.len())]
    }

    fn a_move(rng: &mut StdRng) -> PokemonMoveName {
        loop {
            if let Some(m) = PokemonMoveName::from_repr(rng.random_range(1..=0xA5)) {
                return m;
            }
        }
    }

    /// A facing, now and then one of the bytes that is none of the four and so reads as down.
    fn facing(rng: &mut StdRng) -> u8 {
        if rng.random_range(0..10) == 0 { rng.random() } else { FACINGS[rng.random_range(0..4)] }
    }

    /// Every square on and around each hidden event, facing every way, then squares anywhere on
    /// maps with and without a table, the byte-wrapping edges among them.
    fn hidden_event_inputs() -> Vec<HiddenEventInput> {
        let mut rng = StdRng::seed_from_u64(0x41DDE);
        let mut inputs = vec![];
        let maps: Vec<Map> = Map::all().collect();
        for &map in &maps {
            let Some(events) = hidden_events(map) else { continue };
            for event in events {
                for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (2, 0), (0, -2)] {
                    for facing in FACINGS {
                        let x = event.x.wrapping_add_signed(dx);
                        let y = event.y.wrapping_add_signed(dy);
                        inputs.push(HiddenEventInput { map, x, y, facing });
                    }
                }
            }
        }
        for _ in 0..1500 {
            let map = maps[rng.random_range(0..maps.len())];
            let edge = |rng: &mut StdRng| match rng.random_range(0..12) {
                0 => 0,
                1 => 0xFF,
                _ => rng.random_range(0..64),
            };
            inputs.push(HiddenEventInput { map, x: edge(&mut rng), y: edge(&mut rng), facing: facing(&mut rng) });
        }
        inputs
    }

    /// Every tile of every tileset facing up, and a sample facing any other way.
    fn bookshelf_inputs() -> Vec<BookshelfInput> {
        let mut rng = StdRng::seed_from_u64(0xB00C);
        let tilesets: Vec<TileSetId> = (0..=23).filter_map(TileSetId::from_repr).collect();
        let mut inputs: Vec<BookshelfInput> = tilesets.iter()
            .flat_map(|&tileset| (0..=255).map(move |tile| BookshelfInput { tileset, tile, facing: SpriteFacing::Up as u8 }))
            .collect();
        for _ in 0..600 {
            let tileset = tilesets[rng.random_range(0..tilesets.len())];
            let facing = loop {
                let facing = facing(&mut rng);
                if facing != SpriteFacing::Up as u8 {
                    break facing;
                }
            };
            inputs.push(BookshelfInput { tileset, tile: rng.random(), facing });
        }
        inputs
    }

    /// Moves packed or with gaps, any PP byte, PP Ups and all.
    fn moves_and_pp(rng: &mut StdRng) -> ([Option<PokemonMoveName>; 4], [u8; 4]) {
        let moves = [(); 4].map(|_| rng.random_bool(0.75).then(|| a_move(rng)));
        (moves, [(); 4].map(|_| rng.random()))
    }

    fn poisoned_party_mon(rng: &mut StdRng) -> PartyMon {
        const PSN: u8 = 1 << 3;
        let hp = match rng.random_range(0..10) {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 0x100,
            4 => 0x101,
            5 => 0x200,
            _ => rng.random_range(0..400),
        };
        let status = match rng.random_range(0..5) {
            0 => 0,
            1 => rng.random::<u8>() & !PSN,
            2 => rng.random::<u8>() | PSN,
            _ => PSN,
        };
        let (moves, pp) = moves_and_pp(rng);
        a_party_mon(species(rng), hp, rng.random(), status, moves, pp)
    }

    fn day_care_exp(rng: &mut StdRng) -> u32 {
        match rng.random_range(0..6) {
            0 => rng.random_range(0..0x100) << 16 | 0xFFFF,
            1 => 0x50_0000 + rng.random_range(0..0x1_0000),
            _ => rng.random_range(0..0x50_0000),
        }
    }

    fn poison_inputs() -> Vec<PoisonInput> {
        let mut rng = StdRng::seed_from_u64(0x9015);
        (0..800).map(|_| {
            // Short parties more often, since a blackout needs every mon down.
            let len = [0, 1, 1, 1, 2, 2, 3, 4, 5, 6][rng.random_range(0..10)];
            let party: Vec<PartyMon> = (0..len).map(|_| poisoned_party_mon(&mut rng)).collect();
            let day_care = rng.random_bool(0.5).then(|| BoxMon { exp: day_care_exp(&mut rng), ..a_party_mon(species(&mut rng), 0, 0, 0, [None; 4], [0; 4]).mon });
            // Mostly a multiple of four, since only those reach the damage.
            let step_counter = if rng.random_bool(0.7) { rng.random::<u8>() & !3 } else { rng.random() };
            PoisonInput { party, day_care, step_counter, simulating: rng.random_range(0..10) == 0 }
        }).collect()
    }

    fn exp_inputs() -> Vec<u32> {
        let mut rng = StdRng::seed_from_u64(0xDA7C);
        let mut inputs: Vec<u32> = (0..=0xFF).flat_map(|top: u32| [top << 16 | 0xFFFF, top << 16 | 0xFFFE, top << 16 | 0x00FF, top << 16]).collect();
        inputs.extend((0..400).map(|_| rng.random::<u32>() & 0xFF_FFFF));
        inputs
    }

    /// Experience on, just short of and past a level's threshold, at 100 and beyond it to the cap,
    /// against a box level below, at, above and far from the level it makes.
    fn collection_inputs() -> Vec<BoxMon> {
        let mut rng = StdRng::seed_from_u64(0xC011);
        (0..1500).map(|_| {
            let species = species(&mut rng);
            let growth = BaseStats::of(species).growth_rate;
            let level = rng.random_range(2..=100);
            let exp = match rng.random_range(0..8) {
                0 => calc_experience(growth, level) - 1,
                1 => calc_experience(growth, level),
                2 => calc_experience(growth, 100) + rng.random_range(0..200_000),
                3 => [0, 0x50_0000, 0x50_FFFF][rng.random_range(0..3)],
                _ => rng.random_range(calc_experience(growth, level)..calc_experience(growth, level + 1).max(calc_experience(growth, level) + 1)),
            };
            let box_level = match rng.random_range(0..8) {
                0 => level,
                1 => level.saturating_sub(1),
                2 => level + 1,
                3 => [0, 1, 100, 0xFF][rng.random_range(0..4)],
                _ => rng.random_range(1..=level),
            };
            BoxMon { exp, box_level, ..a_party_mon(species, 0, 0, 0, [None; 4], [0; 4]).mon }
        }).collect()
    }

    fn heal_party_inputs() -> Vec<Vec<PartyMon>> {
        let mut rng = StdRng::seed_from_u64(0x4EA1);
        (0..500).map(|_| {
            (0..rng.random_range(1..=6)).map(|_| {
                let (moves, pp) = moves_and_pp(&mut rng);
                a_party_mon(species(&mut rng), rng.random(), rng.random(), rng.random(), moves, pp)
            }).collect()
        }).collect()
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/events/*.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_events() {
        let mut oracle = oracle();

        let cases: Vec<_> = hidden_event_inputs().into_iter()
            .map(|input| Case { input, output: check_for_hidden_event(&mut oracle, input), rng: vec![] })
            .collect();
        let found = cases.iter().filter(|case| case.output.is_some()).count();
        assert!(found > 500, "only {found} found among {}", cases.len());
        write_fixture("events", "check_for_hidden_event", &cases);

        let cases: Vec<_> = bookshelf_inputs().into_iter()
            .map(|input| Case { input, output: print_bookshelf_text(&mut oracle, input), rng: vec![] })
            .collect();
        let found = cases.iter().filter(|case| case.output.is_some()).count();
        assert_eq!(found, 17, "every row of `BookshelfTileIDs`, once");
        write_fixture("events", "print_bookshelf_text", &cases);

        let cases: Vec<_> = poison_inputs().into_iter().map(|input| {
            let output = apply_out_of_battle_poison_damage(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        let fainted = cases.iter().filter(|case| !case.output.step.fainted.is_empty()).count();
        let blacked_out = cases.iter().filter(|case| case.output.step.blacked_out).count();
        assert!(fainted > 50 && blacked_out > 20, "{fainted} faints and {blacked_out} blackouts among {}", cases.len());
        write_fixture("events", "apply_out_of_battle_poison_damage", &cases);

        let cases: Vec<_> = exp_inputs().into_iter()
            .map(|input| Case { input, output: increment_day_care_mon_exp(&mut oracle, input), rng: vec![] })
            .collect();
        write_fixture("events", "increment_day_care_mon_exp", &cases);

        let cases: Vec<_> = collection_inputs().into_iter().map(|input| {
            let output = day_care_collection(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("events", "day_care_collection", &cases);

        let cases: Vec<_> = heal_party_inputs().into_iter().map(|input| {
            let output = heal_party(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("events", "heal_party", &cases);
    }
}
