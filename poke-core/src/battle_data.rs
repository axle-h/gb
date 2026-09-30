use crate::tables::{HIGH_CRITICAL_MOVES, STAT_MODIFIER_RATIOS};

/// `HighCriticalMoves`.
pub fn high_critical_moves() -> Vec<u8> {
    HIGH_CRITICAL_MOVES.to_vec()
}

/// `StatModifierRatios`: `(numerator, denominator)` for stages -6 to +6.
pub fn stat_modifier_ratios() -> [(u8, u8); 13] {
    STAT_MODIFIER_RATIOS
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
