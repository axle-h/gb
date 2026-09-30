use crate::tables::TYPE_EFFECTS;

pub const SUPER_EFFECTIVE: u8 = 20;
pub const NOT_VERY_EFFECTIVE: u8 = 5;
pub const NO_EFFECT: u8 = 0;

/// `TypeEffects` in the cartridge's order, which is the order the damage routine applies them:
/// `(attacker, defender, multiplier × 10)`.
pub fn matchups() -> Vec<(u8, u8, u8)> {
    TYPE_EFFECTS.to_vec()
}

#[cfg(test)]
mod tests {
    use crate::pokemon::{MoveEffectiveness, PokemonType};
    use super::*;

    #[test]
    fn every_matchup_matches_the_hand_written_chart() {
        let table = matchups();
        assert_eq!(table.len(), 82);
        for (attacker, defender, multiplier) in table {
            let (a, d) = (PokemonType::from_repr(attacker).unwrap(), PokemonType::from_repr(defender).unwrap());
            let expected = match multiplier {
                SUPER_EFFECTIVE => MoveEffectiveness::Double,
                NOT_VERY_EFFECTIVE => MoveEffectiveness::Half,
                _ => MoveEffectiveness::None,
            };
            assert_eq!(a.attack_effectiveness(d), expected, "{a} on {d}");
        }
    }
}
