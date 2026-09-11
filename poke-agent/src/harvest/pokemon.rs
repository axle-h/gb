//! `WriteMonMoves`, `AddPartyMon` and `MoveMon`, over `box_struct` and `party_struct` as WRAM
//! lays them out.

use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use pokered::party::{BoxMon, PartyMon};
use pokered::systems::add_mon::Origin;
use pokered::systems::evos_moves::Learner;
use pokered::systems::stats::Dvs;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::Oracle;

const BOXMON_STRUCT_LENGTH: usize = 0x21;
const PARTYMON_STRUCT_LENGTH: usize = 0x2C;
const PARTY_TO_BOX: u8 = 1;
const DAYCARE_TO_PARTY: u8 = 2;
const PARTY_TO_DAYCARE: u8 = 3;
/// `_AddPartyMon` names a mon only when all of `wMonDataLocation` is 0; the player's party is
/// the low nybble alone.
const PLAYER_PARTY_UNNAMED: u8 = 0x10;
const ENEMY_PARTY_DATA: u8 = 1;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

fn be16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn move_bytes(moves: [Option<PokemonMoveName>; 4]) -> [u8; 4] {
    moves.map(|m| m.map_or(0, |m| m as u8))
}

fn moves_of(bytes: &[u8]) -> [Option<PokemonMoveName>; 4] {
    [0, 1, 2, 3].map(|i| (bytes[i] != 0).then(|| PokemonMoveName::from_repr(bytes[i]).expect("a move")))
}

fn encode_box(mon: &BoxMon) -> Vec<u8> {
    let mut bytes = vec![mon.species as u8];
    bytes.extend(mon.hp.to_be_bytes());
    bytes.extend([mon.box_level, mon.status, mon.types[0], mon.types[1], mon.catch_rate]);
    bytes.extend(move_bytes(mon.moves));
    bytes.extend(mon.ot_id.to_be_bytes());
    bytes.extend(&mon.exp.to_be_bytes()[1..]);
    bytes.extend(mon.stat_exp.iter().flat_map(|exp| exp.to_be_bytes()));
    bytes.extend(mon.dvs.0);
    bytes.extend(mon.pp);
    assert_eq!(bytes.len(), BOXMON_STRUCT_LENGTH);
    bytes
}

fn encode_party(mon: &PartyMon) -> Vec<u8> {
    let mut bytes = encode_box(&mon.mon);
    bytes.push(mon.level);
    bytes.extend(mon.stats.iter().flat_map(|stat| stat.to_be_bytes()));
    bytes
}

fn decode_box(b: &[u8]) -> BoxMon {
    BoxMon {
        species: PokemonSpecies::from_repr(b[0]).expect("a species"),
        hp: be16(&b[1..]),
        box_level: b[3],
        status: b[4],
        types: [b[5], b[6]],
        catch_rate: b[7],
        moves: moves_of(&b[8..12]),
        ot_id: be16(&b[12..]),
        exp: u32::from_be_bytes([0, b[14], b[15], b[16]]),
        stat_exp: [0, 1, 2, 3, 4].map(|i| be16(&b[17 + 2 * i..])),
        dvs: Dvs([b[27], b[28]]),
        pp: b[29..33].try_into().unwrap(),
    }
}

fn decode_party(b: &[u8]) -> PartyMon {
    PartyMon { mon: decode_box(b), level: b[33], stats: [0, 1, 2, 3, 4].map(|i| be16(&b[34 + 2 * i..])) }
}

fn read_party_mon(oracle: &Oracle, first: DmgPointer, slot: usize) -> PartyMon {
    decode_party(&oracle.read(first + (slot * PARTYMON_STRUCT_LENGTH) as u16, PARTYMON_STRUCT_LENGTH))
}

/// A species list of `count` Rhydons, as the count and list a routine appends to.
fn fill_list(oracle: &mut Oracle, count: DmgPointer, n: usize) {
    oracle.write(count, &[n as u8]);
    let mut list = vec![PokemonSpecies::Rhydon as u8; n];
    list.push(0xFF);
    oracle.write(count + 1, &list);
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WriteMonMovesInput {
    species: PokemonSpecies,
    level: u8,
    moves: [Option<PokemonMoveName>; 4],
    pp: [u8; 4],
    learner: Learner,
}

/// Called as `predef` leaves it: `de`, the moves, in `wPredefDE`, and the PP after them as in
/// `party_struct`.
fn write_mon_moves(oracle: &mut Oracle, i: &WriteMonMovesInput) -> ([Option<PokemonMoveName>; 4], [u8; 4]) {
    oracle.write(sym::wPartyMon1Moves, &move_bytes(i.moves));
    oracle.write(sym::wPartyMon1PP, &i.pp);
    oracle.write(sym::wPredefDE, &sym::wPartyMon1Moves.address.to_be_bytes());
    oracle.write(sym::wCurPartySpecies, &[i.species as u8]);
    oracle.write(sym::wCurEnemyLevel, &[i.level]);
    let (day_care, start) = match i.learner {
        Learner::New => (0, 0),
        Learner::DayCare { start_level } => (1, start_level),
    };
    oracle.write(sym::wLearningMovesFromDayCare, &[day_care]);
    oracle.write(sym::wDayCareStartLevel, &[start]);
    oracle.call(sym::WriteMonMoves);
    (moves_of(&oracle.read(sym::wPartyMon1Moves, 4)), oracle.read(sym::wPartyMon1PP, 4).try_into().unwrap())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NewPartyMonInput {
    species: PokemonSpecies,
    level: u8,
    ot_id: u16,
    origin: Origin,
}

/// `AddPartyMon` behind `party_len` others, to the player's party unnamed or a trainer's.
fn add_party_mon(oracle: &mut Oracle, i: &NewPartyMonInput, party_len: usize) -> (PartyMon, Vec<u8>) {
    oracle.write(sym::wCurPartySpecies, &[i.species as u8]);
    oracle.write(sym::wCurEnemyLevel, &[i.level]);
    oracle.write(sym::wPlayerID, &i.ot_id.to_be_bytes());
    let (location, in_battle, count, mons) = match i.origin {
        Origin::Given => (PLAYER_PARTY_UNNAMED, 0, sym::wPartyCount, sym::wPartyMons),
        Origin::Caught { .. } => (PLAYER_PARTY_UNNAMED, 1, sym::wPartyCount, sym::wPartyMons),
        Origin::Trainer => (ENEMY_PARTY_DATA, 2, sym::wEnemyPartyCount, sym::wEnemyMons),
    };
    if let Origin::Caught { dvs, hp, status, stats } = i.origin {
        oracle.write(sym::wEnemyMonDVs, &dvs.0);
        oracle.write(sym::wEnemyMonHP, &hp.to_be_bytes());
        oracle.write(sym::wEnemyMonStatus, &[status]);
        oracle.write(sym::wEnemyMonMaxHP, &stats.iter().flat_map(|s| s.to_be_bytes()).collect::<Vec<_>>());
    }
    oracle.write(sym::wMonDataLocation, &[location]);
    oracle.write(sym::wIsInBattle, &[in_battle]);
    fill_list(oracle, count, party_len);
    let called = oracle.call(sym::AddPartyMon);
    assert!(oracle.registers().flags.c, "added");
    (read_party_mon(oracle, mons, party_len), called.rng)
}

/// `MoveMon` from box slot 0 or the day care into a party of `party_len`.
fn withdraw(oracle: &mut Oracle, mon: &BoxMon, from_day_care: bool, party_len: usize) -> PartyMon {
    fill_list(oracle, sym::wPartyCount, party_len);
    if from_day_care {
        oracle.write(sym::wDayCareMon, &encode_box(mon));
        oracle.write(sym::wMoveMonType, &[DAYCARE_TO_PARTY]);
    } else {
        fill_list(oracle, sym::wBoxCount, 1);
        oracle.write(sym::wBoxSpecies, &[mon.species as u8, 0xFF]);
        oracle.write(sym::wBoxMons, &encode_box(mon));
        oracle.write(sym::wMoveMonType, &[0]);
    }
    oracle.write(sym::wCurPartySpecies, &[mon.species as u8]);
    oracle.write(sym::wWhichPokemon, &[0]);
    oracle.call(sym::MoveMon);
    assert!(!oracle.registers().flags.c, "moved");
    read_party_mon(oracle, sym::wPartyMons, party_len)
}

/// `MoveMon` from party slot 0 into a box of `box_len` or the day care.
fn deposit(oracle: &mut Oracle, mon: &PartyMon, to_day_care: bool, box_len: usize) -> BoxMon {
    fill_list(oracle, sym::wPartyCount, 1);
    oracle.write(sym::wPartySpecies, &[mon.mon.species as u8, 0xFF]);
    oracle.write(sym::wPartyMons, &encode_party(mon));
    fill_list(oracle, sym::wBoxCount, box_len);
    oracle.write(sym::wMoveMonType, &[if to_day_care { PARTY_TO_DAYCARE } else { PARTY_TO_BOX }]);
    oracle.write(sym::wCurPartySpecies, &[mon.mon.species as u8]);
    oracle.write(sym::wWhichPokemon, &[0]);
    oracle.call(sym::MoveMon);
    assert!(!oracle.registers().flags.c, "moved");
    let at = if to_day_care { sym::wDayCareMon } else { sym::wBoxMons + (box_len * BOXMON_STRUCT_LENGTH) as u16 };
    decode_box(&oracle.read(at, BOXMON_STRUCT_LENGTH))
}

#[test]
fn a_new_bulbasaur_is_level_1_moves_and_full_hp() {
    use PokemonMoveName::*;
    let mut oracle = oracle();
    let input = NewPartyMonInput { species: PokemonSpecies::Bulbasaur, level: 5, ot_id: 0x1234, origin: Origin::Trainer };
    let (mon, rng) = add_party_mon(&mut oracle, &input, 0);
    assert!(rng.is_empty());
    assert_eq!(mon.mon.moves, [Some(Tackle), Some(Growl), None, None]);
    assert_eq!((mon.mon.dvs, mon.mon.hp, mon.level), (Dvs([0x98, 0x88]), mon.stats[0], 5));
    assert_eq!(mon.mon.ot_id, 0x1234);
}

#[test]
fn the_oracle_moves_a_mon_and_teaches_it() {
    use PokemonMoveName::*;
    let mut oracle = oracle();
    let (mon, _) = add_party_mon(&mut oracle, &NewPartyMonInput {
        species: PokemonSpecies::Bulbasaur, level: 20, ot_id: 1, origin: Origin::Trainer }, 0);
    let boxed = deposit(&mut oracle, &mon, false, 3);
    assert_eq!(boxed.box_level, 20);
    assert_eq!(withdraw(&mut oracle, &boxed, true, 5), PartyMon { mon: boxed, ..mon }, "the box level stays");
    let learner = WriteMonMovesInput { species: PokemonSpecies::Bulbasaur, level: 7, moves: [Some(Tackle), Some(Growl), None, None],
        pp: [35, 40, 0, 0], learner: Learner::DayCare { start_level: 5 } };
    assert_eq!(write_mon_moves(&mut oracle, &learner), ([Some(Tackle), Some(Growl), Some(LeechSeed), None], [35, 40, 10, 0]));
}

#[test]
fn a_mon_round_trips_through_its_bytes() {
    let mut oracle = oracle();
    let (mon, _) = add_party_mon(&mut oracle, &NewPartyMonInput {
        species: PokemonSpecies::Pikachu, level: 30, ot_id: 7, origin: Origin::Given }, 2);
    assert_eq!(decode_party(&encode_party(&mon)), mon);
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use poke_core::base_stats::BaseStats;
    use poke_core::evos_moves::EvosMoves;
    use pokered::systems::experience::calc_experience;
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};
    use strum::IntoEnumIterator;
    use super::super::{write_fixture, Case};
    use super::*;

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

    /// Empty, the level-1 moves, some of its own learnset, or anything, packed from the front as
    /// the cartridge keeps them.
    fn moves(rng: &mut StdRng, species: PokemonSpecies) -> [Option<PokemonMoveName>; 4] {
        let learnset = EvosMoves::of(species).learnset;
        let mut moves: Vec<PokemonMoveName> = match rng.random_range(0..4) {
            0 => vec![],
            1 => BaseStats::of(species).level_1_moves().collect(),
            2 if !learnset.is_empty() => (0..rng.random_range(1..=4)).map(|_| learnset[rng.random_range(0..learnset.len())].1).collect(),
            _ => (0..rng.random_range(1..=4)).map(|_| a_move(rng)).collect(),
        };
        moves.dedup();
        moves.truncate(4);
        [0, 1, 2, 3].map(|i| moves.get(i).copied())
    }

    fn a_box_mon(rng: &mut StdRng) -> BoxMon {
        let species = species(rng);
        let rate = BaseStats::of(species).growth_rate;
        let exp_cap = if rng.random_range(0..8) == 0 { calc_experience(rate, 120) } else { calc_experience(rate, 100) };
        BoxMon {
            species,
            hp: rng.random(),
            box_level: rng.random(),
            status: rng.random(),
            types: [rng.random(), rng.random()],
            catch_rate: rng.random(),
            moves: moves(rng, species),
            ot_id: rng.random(),
            exp: rng.random_range(0..=exp_cap),
            stat_exp: [(); 5].map(|_| rng.random()),
            dvs: Dvs([rng.random(), rng.random()]),
            pp: [(); 4].map(|_| rng.random()),
        }
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/pokemon/*.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_pokemon() {
        let mut oracle = oracle();
        let mut rng = StdRng::seed_from_u64(0x90CE);

        let cases: Vec<Case<WriteMonMovesInput, _>> = (0..800).map(|_| {
            let species = species(&mut rng);
            let level = rng.random_range(1..=100);
            let learner = if rng.random_bool(0.5) { Learner::New } else { Learner::DayCare { start_level: rng.random_range(1..=level) } };
            let input = WriteMonMovesInput { species, level, moves: moves(&mut rng, species), pp: [(); 4].map(|_| rng.random()), learner };
            let output = write_mon_moves(&mut oracle, &input);
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("pokemon", "write_mon_moves", &cases);

        let cases: Vec<Case<NewPartyMonInput, PartyMon>> = (0..600).map(|_| {
            let origin = match rng.random_range(0..3) {
                0 => Origin::Given,
                1 => Origin::Caught { dvs: Dvs([rng.random(), rng.random()]), hp: rng.random(), status: rng.random(), stats: [(); 5].map(|_| rng.random()) },
                _ => Origin::Trainer,
            };
            let input = NewPartyMonInput { species: species(&mut rng), level: rng.random_range(1..=100), ot_id: rng.random(), origin };
            let (output, tape) = add_party_mon(&mut oracle, &input, rng.random_range(0..6));
            Case { input, output, rng: tape }
        }).collect();
        write_fixture("pokemon", "add_party_mon", &cases);

        let cases: Vec<Case<BoxMon, PartyMon>> = (0..400).map(|_| {
            let input = a_box_mon(&mut rng);
            let output = withdraw(&mut oracle, &input, rng.random_bool(0.25), rng.random_range(0..6));
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("pokemon", "withdraw", &cases);

        let cases: Vec<Case<PartyMon, BoxMon>> = (0..200).map(|_| {
            let input = PartyMon { mon: a_box_mon(&mut rng), level: rng.random(), stats: [(); 5].map(|_| rng.random()) };
            let output = deposit(&mut oracle, &input, rng.random_bool(0.25), rng.random_range(0..20));
            Case { input, output, rng: vec![] }
        }).collect();
        write_fixture("pokemon", "deposit", &cases);
    }
}
