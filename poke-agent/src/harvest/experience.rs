//! `CalcExperience` and `CalcLevelFromExperience`.

use crate::pokemon::symbols::pokered_symbols as sym;
use super::Oracle;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

/// `d` is the level; the growth rate is read from `wMonHeader`.
fn calc_experience(oracle: &mut Oracle, growth_rate: u8, level: u8) -> u32 {
    oracle.write(sym::wMonHGrowthRate, &[growth_rate]);
    oracle.registers_mut().d = level;
    oracle.call(sym::CalcExperience);
    let experience = oracle.read(sym::hExperience, 3);
    u32::from_be_bytes([0, experience[0], experience[1], experience[2]])
}

/// The growth rate comes from the header of `wLoadedMonSpecies`, so any species that has it.
fn calc_level_from_experience(oracle: &mut Oracle, growth_rate: u8, experience: u32) -> u8 {
    oracle.write(sym::wLoadedMonSpecies, &[species_growing_at(growth_rate)]);
    oracle.write(sym::wLoadedMonExp, &experience.to_be_bytes()[1..]);
    oracle.call(sym::CalcLevelFromExperience);
    oracle.registers().d
}

fn species_growing_at(growth_rate: u8) -> u8 {
    use poke_core::base_stats::BaseStats;
    use poke_core::species::PokemonSpecies;
    use strum::IntoEnumIterator;
    PokemonSpecies::iter().find(|&species| BaseStats::of(species).growth_rate == growth_rate)
        .unwrap_or_else(|| panic!("no species grows at rate {growth_rate}")) as u8
}

#[test]
fn the_oracle_knows_the_cubic_curves() {
    let mut oracle = oracle();
    assert_eq!(calc_experience(&mut oracle, 0, 100), 1_000_000, "Medium Fast is n³");
    assert_eq!(calc_experience(&mut oracle, 5, 100), 1_250_000, "Slow is 5/4 n³");
    assert_eq!(calc_level_from_experience(&mut oracle, 0, 1_000_000), 100);
    assert_eq!(calc_level_from_experience(&mut oracle, 0, 999_999), 99);
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/experience/*.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_experience() {
    use rand::{RngExt, SeedableRng};
    use super::{write_fixture, Case};
    let mut oracle = oracle();
    let cases: Vec<Case<(u8, u8), u32>> = (0..6u8).flat_map(|rate| (0..=u8::MAX).map(move |level| (rate, level)))
        .map(|(rate, level)| Case { input: (rate, level), output: calc_experience(&mut oracle, rate, level), rng: vec![] })
        .collect();
    write_fixture("experience", "calc_experience", &cases);

    // Level 100 and either side of every level from 1 to 100, then anywhere short of the first
    // total no level exceeds, where the cartridge loops for ever.
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xE4E4);
    let mut inputs = vec![];
    // Rows 1 and 2 of `GrowthRateTable` belong to no species.
    for rate in [0, 3, 4, 5] {
        let unreachable = (0..=u8::MAX).map(|level| pokered::systems::experience::calc_experience(rate, level)).max().unwrap();
        for level in 1..=100u8 {
            let at = pokered::systems::experience::calc_experience(rate, level);
            inputs.extend([at.saturating_sub(1), at, at + 1].into_iter()
                .filter(|&experience| experience < unreachable)
                .map(|experience| (rate, experience)));
        }
        inputs.extend((0..100).map(|_| (rate, rng.random_range(0..unreachable))));
    }
    let cases: Vec<Case<(u8, u32), u8>> = inputs.into_iter()
        .map(|(rate, experience)| Case { input: (rate, experience), output: calc_level_from_experience(&mut oracle, rate, experience), rng: vec![] })
        .collect();
    write_fixture("experience", "calc_level_from_experience", &cases);
}
