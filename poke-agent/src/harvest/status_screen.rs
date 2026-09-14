//! `CalcExpToLevelUp`, `FormatMovesString` and `PrintMonType`: the parts of the status screen and
//! `LearnMove` that do not wait on a frame.

use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use pokered::systems::learn_move::MovesString;
use crate::pokemon::symbols::pokered_symbols as sym;
use super::Oracle;

/// Whatever `wNumMovesMinusOne` held before the call, so a call that never writes it shows.
const UNWRITTEN: u8 = 0xAA;
const TERMINATOR: u8 = 0x50;
const BLANK: u8 = 0x7F;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

/// The growth rate is read from `wMonHeader`, and the answer is left in `wLoadedMonExp`.
fn calc_exp_to_level_up(oracle: &mut Oracle, growth_rate: u8, level: u8, exp: u32) -> u32 {
    oracle.write(sym::wMonHGrowthRate, &[growth_rate]);
    oracle.write(sym::wLoadedMonLevel, &[level]);
    oracle.write(sym::wLoadedMonExp, &exp.to_be_bytes()[1..]);
    oracle.call(sym::CalcExpToLevelUp);
    let left = oracle.read(sym::wLoadedMonExp, 3);
    u32::from_be_bytes([0, left[0], left[1], left[2]])
}

fn format_moves_string(oracle: &mut Oracle, moves: [Option<PokemonMoveName>; 4]) -> MovesString {
    oracle.write(sym::wMoves, &moves.map(|known| known.map_or(0, |known| known as u8)));
    oracle.write(sym::wNumMovesMinusOne, &[UNWRITTEN]);
    oracle.call(sym::FormatMovesString);
    let bytes = oracle.read(sym::wMovesString, 4 * 14);
    let end = bytes.iter().position(|&byte| byte == TERMINATOR).expect("the string ends");
    let count = oracle.read(sym::wNumMovesMinusOne, 1)[0];
    MovesString { string: bytes[..=end].to_vec(), num_moves_minus_one: (count != UNWRITTEN).then_some(count) }
}

/// `hl` arrives through `wPredefHL`, since the routine opens with `GetPredefRegisters`. The rows
/// are blanked and given the status screen's `TYPE1/` and `TYPE2/` first, which is what the
/// single-type case rubs out.
fn print_mon_type(oracle: &mut Oracle, species: PokemonSpecies) -> Vec<Vec<u8>> {
    let tile_map = sym::wTileMap.address;
    let row = |y: u16| crate::pokemon::symbols::DmgPointer { address: tile_map + y * 20, ..sym::wTileMap };
    for y in 9..13 {
        oracle.write(row(y), &[BLANK; 20]);
    }
    let encode = |text| poke_core::charmap::encode(text).unwrap();
    oracle.write(crate::pokemon::symbols::DmgPointer { address: tile_map + 9 * 20 + 10, ..sym::wTileMap }, &encode("TYPE1/"));
    oracle.write(crate::pokemon::symbols::DmgPointer { address: tile_map + 11 * 20 + 10, ..sym::wTileMap }, &encode("TYPE2/"));
    oracle.write(sym::wCurSpecies, &[species as u8]);
    oracle.write(sym::wPredefHL, &(tile_map + 10 * 20 + 11).to_be_bytes());
    oracle.call(sym::PrintMonType);
    (9..13).map(|y| oracle.read(row(y), 20)).collect()
}

#[test]
fn the_oracle_agrees_on_a_level_up_and_a_full_move_list() {
    let mut oracle = oracle();
    assert_eq!(calc_exp_to_level_up(&mut oracle, 0, 99, 970_299), 1_000_000 - 970_299);
    assert_eq!(calc_exp_to_level_up(&mut oracle, 0, 100, 1_000_000), 0);
    use PokemonMoveName::*;
    let moves = [Some(Tackle), Some(Growl), None, None];
    assert_eq!(format_moves_string(&mut oracle, moves), pokered::systems::learn_move::format_moves_string(&moves));
    let rows = print_mon_type(&mut oracle, PokemonSpecies::Pidgey);
    assert_eq!(rows[3][11..17], poke_core::charmap::encode("FLYING").unwrap()[..], "the second type two rows down");
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/status_screen/*.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_status_screen() {
    use rand::{RngExt, SeedableRng};
    use strum::IntoEnumIterator;
    use super::{write_fixture, Case};

    #[derive(serde::Serialize)]
    struct ExpInput {
        growth_rate: u8,
        level: u8,
        exp: u32,
    }
    #[derive(serde::Serialize)]
    struct TypeInput {
        species: PokemonSpecies,
    }

    let mut oracle = oracle();
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x5747);

    // Either side of every threshold, an experience past the next level's, which wraps, and the
    // levels a byte can hold that no mon reaches.
    let mut inputs = vec![];
    for growth_rate in [0, 3, 4, 5] {
        for level in 1..=100u8 {
            let next = pokered::systems::experience::calc_experience(growth_rate, level.wrapping_add(1));
            for exp in [next.saturating_sub(1), next, next + 1, rng.random_range(0..0x100_0000)] {
                inputs.push(ExpInput { growth_rate, level, exp });
            }
        }
        for level in [0, 101, 254, 255] {
            inputs.push(ExpInput { growth_rate, level, exp: rng.random_range(0..0x100_0000) });
        }
    }
    let cases: Vec<Case<ExpInput, u32>> = inputs.into_iter().map(|input| {
        let output = calc_exp_to_level_up(&mut oracle, input.growth_rate, input.level, input.exp);
        Case { input, output, rng: vec![] }
    }).collect();
    write_fixture("status_screen", "calc_exp_to_level_up", &cases);

    // Full lists, short ones, empty ones and ones with a gap in them.
    let all: Vec<PokemonMoveName> = (1..=u8::MAX).filter_map(PokemonMoveName::from_repr).collect();
    let cases: Vec<Case<[Option<PokemonMoveName>; 4], MovesString>> = (0..400).map(|i| {
        let mut moves = [0; 4].map(|_| Some(all[rng.random_range(0..all.len())]));
        match i % 5 {
            0 => {}
            1 => moves[rng.random_range(0..4)..].fill(None),
            2 => moves[rng.random_range(0..4)] = None,
            3 => moves = [None; 4],
            _ => moves[3] = None,
        }
        Case { input: moves, output: format_moves_string(&mut oracle, moves), rng: vec![] }
    }).collect();
    write_fixture("status_screen", "format_moves_string", &cases);

    let cases: Vec<Case<TypeInput, Vec<Vec<u8>>>> = PokemonSpecies::iter()
        .map(|species| Case { input: TypeInput { species }, output: print_mon_type(&mut oracle, species), rng: vec![] })
        .collect();
    write_fixture("status_screen", "print_mon_type", &cases);
}
