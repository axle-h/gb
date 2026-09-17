//! Random battle states, weighted towards the values the arithmetic breaks on.

use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::species::PokemonSpecies;
use pokered::systems::battle::{status, Arena, Side};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use strum::IntoEnumIterator;

pub fn seeded(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

pub fn pick<T: Copy>(rng: &mut StdRng, from: &[T]) -> T {
    from[rng.random_range(0..from.len())]
}

pub fn a_side(rng: &mut StdRng) -> Side {
    if rng.random_bool(0.5) { Side::Player } else { Side::Enemy }
}

pub fn a_species(rng: &mut StdRng) -> PokemonSpecies {
    let all: Vec<_> = PokemonSpecies::iter().collect();
    pick(rng, &all)
}

pub fn a_move(rng: &mut StdRng) -> PokemonMoveName {
    PokemonMoveName::from_repr(rng.random_range(1..=PokemonMoveName::Struggle as u8)).unwrap()
}

/// The fifteen types a mon or a move can have.
pub const TYPES: [u8; 15] = [0, 1, 2, 3, 4, 5, 7, 8, 20, 21, 22, 23, 24, 25, 26];

pub fn a_type(rng: &mut StdRng) -> u8 {
    pick(rng, &TYPES)
}

/// A stat, often one at a boundary the scaling or a cap turns on.
pub fn a_stat(rng: &mut StdRng) -> u16 {
    match rng.random_range(0..3) {
        0 => pick(rng, &[1, 2, 3, 4, 5, 7, 8, 127, 128, 254, 255, 256, 257, 511, 512, 513, 887, 888, 998, 999, 1000, 1023,
            1024, 1027, 1028, 2047, 4096, 32768, 65535]),
        _ => rng.random_range(1..=999),
    }
}

/// A stat no defense can scale to 0 from, with or without a screen: `Divide` never returns from 0.
pub fn a_safe_stat(rng: &mut StdRng) -> u16 {
    loop {
        let stat = match rng.random_range(0..4) {
            0 => pick(rng, &[4, 5, 255, 256, 511, 514, 998, 999]),
            _ => rng.random_range(4..=999),
        };
        if !matches!(stat, 512 | 513) {
            return stat;
        }
    }
}

pub fn a_level(rng: &mut StdRng) -> u8 {
    match rng.random_range(0..4) {
        0 => pick(rng, &[1, 2, 3, 99, 100, 127, 128, 129, 255]),
        _ => rng.random_range(1..=100),
    }
}

pub fn a_stat_mod(rng: &mut StdRng) -> u8 {
    match rng.random_range(0..3) {
        0 => pick(rng, &[1, 7, 13]),
        _ => rng.random_range(1..=13),
    }
}

pub fn a_damage(rng: &mut StdRng) -> u16 {
    match rng.random_range(0..4) {
        0 => pick(rng, &[0, 1, 2, 3, 255, 256, 996, 997, 998, 999, 0x7FFF, 0x8000, 0xFFFF]),
        1 => rng.random(),
        _ => rng.random_range(0..=400),
    }
}

pub fn a_status(rng: &mut StdRng) -> u8 {
    match rng.random_range(0..6) {
        0 | 1 => 0,
        2 => rng.random_range(1..=7),
        3 => pick(rng, &[status::PSN, status::BRN, status::FRZ, status::PAR]),
        4 => rng.random(),
        _ => status::PAR | status::BRN,
    }
}

/// The move `side` is using, as `GetCurrentMove` would have loaded it.
pub fn using(arena: &mut Arena, side: Side, name: PokemonMoveName) {
    let combatant = arena.battle.side_mut(side);
    combatant.current_move = MoveData::of_move(name);
    combatant.selected_move = name as u8;
}
