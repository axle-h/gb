//! `engine/pokemon/experience.asm`: the experience a growth rate needs for a level, and the level a
//! total of experience makes. Experience is 24 bits, as `hExperience` holds it, and every wrap the
//! cartridge's arithmetic makes is kept: Medium Slow needs 16,777,162 at level 1.

use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_symbols;
use super::math::{divide, multiply};

const EXPERIENCE_MASK: u32 = 0xFF_FFFF;

/// `CalcExperience`: `a/b·n³ + c·n² + e·n − f` for `GrowthRateTable` row `growth_rate`, where the
/// cube is divided before anything is added and each term is cut to the three bytes it is kept in.
pub fn calc_experience(growth_rate: u8, level: u8) -> u32 {
    let row = &rom_slice(pokered_symbols::GrowthRateTable)[growth_rate as usize * 4..][..4];
    let (numerator, denominator) = (row[0] >> 4, row[0] & 0xF);
    let (squared, linear, constant) = (row[1], row[2], row[3]);
    let d_squared = || multiply(level as u32, level);

    let cubed = multiply(multiply(d_squared(), level), numerator);
    let (quotient, _) = divide(cubed.to_be_bytes(), denominator, 4);
    let cubed_term = u32::from_be_bytes(quotient) & EXPERIENCE_MASK;
    let squared_term = multiply(d_squared(), squared & 0x7F) & EXPERIENCE_MASK;
    let mut experience = multiply(level as u32, linear).wrapping_sub(constant as u32) & EXPERIENCE_MASK;
    experience = if squared & 0x80 != 0 {
        experience.wrapping_sub(squared_term)
    } else {
        experience + squared_term
    };
    (experience + cubed_term) & EXPERIENCE_MASK
}

/// `CalcLevelFromExperience`: one below the first level from 2 up that needs more than
/// `experience`. The level is a byte and wraps, so experience past every level's never returns.
pub fn calc_level_from_experience(growth_rate: u8, experience: u32) -> u8 {
    let mut level = 1u8;
    for _ in 0..=u8::MAX as u32 * 2 {
        level = level.wrapping_add(1);
        if calc_experience(growth_rate, level) > experience {
            return level.wrapping_sub(1);
        }
    }
    panic!("CalcLevelFromExperience never returns for {experience} at growth rate {growth_rate}")
}

#[cfg(test)]
mod tests {
    use poke_core::base_stats::BaseStats;
    use poke_core::species::PokemonSpecies;
    use strum::IntoEnumIterator;
    use crate::fixtures::cases;
    use super::*;

    #[test]
    fn every_harvested_case_of_calc_experience() {
        let cases = cases::<(u8, u8), u32>(include_str!("../../fixtures/experience/calc_experience.jsonl"));
        for ((growth_rate, level), output, _) in cases {
            assert_eq!(calc_experience(growth_rate, level), output, "growth rate {growth_rate}, level {level}");
        }
    }

    #[test]
    fn every_harvested_case_of_calc_level_from_experience() {
        let cases = cases::<(u8, u32), u8>(include_str!("../../fixtures/experience/calc_level_from_experience.jsonl"));
        for ((growth_rate, experience), output, _) in cases {
            assert_eq!(calc_level_from_experience(growth_rate, experience), output,
                "growth rate {growth_rate}, {experience} experience");
        }
    }

    /// The hand-written tables agree from level 2, where the cartridge stops wrapping.
    #[test]
    fn every_species_matches_the_hand_written_table() {
        for species in PokemonSpecies::iter() {
            let growth_rate = BaseStats::of(species).growth_rate;
            let group = species.metadata().experience_group;
            for level in 2..=100 {
                assert_eq!(calc_experience(growth_rate, level), group.experience_for_level(level), "{species} at {level}");
            }
        }
    }
}
