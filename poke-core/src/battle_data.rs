use crate::rom_gfx::rom_slice;
use crate::symbols::pokered_symbols;

/// `HighCriticalMoves`.
pub fn high_critical_moves() -> Vec<u8> {
    rom_slice(pokered_symbols::HighCriticalMoves).iter().copied().take_while(|&m| m != 0xFF).collect()
}

/// `StatModifierRatios`: `(numerator, denominator)` for stages -6 to +6.
pub fn stat_modifier_ratios() -> [(u8, u8); 13] {
    let bytes = rom_slice(pokered_symbols::StatModifierRatios);
    std::array::from_fn(|stage| (bytes[stage * 2], bytes[stage * 2 + 1]))
}

#[cfg(test)]
mod tests {
    use crate::move_name::PokemonMoveName as M;
    use super::*;

    #[test]
    fn four_moves_crit_often_and_stage_zero_is_one() {
        assert_eq!(high_critical_moves(), [M::KarateChop as u8, M::RazorLeaf as u8, M::Crabhammer as u8, M::Slash as u8]);
        let ratios = stat_modifier_ratios();
        assert_eq!((ratios[0], ratios[6], ratios[12]), ((25, 100), (1, 1), (4, 1)));
    }
}
