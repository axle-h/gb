use crate::pokemon::PokemonType;
use crate::ruleset::Ruleset;
use crate::tables::TYPE_EFFECTS;

pub const SUPER_EFFECTIVE: u8 = 20;
pub const NOT_VERY_EFFECTIVE: u8 = 5;
pub const NO_EFFECT: u8 = 0;

/// `TypeEffects` in the cartridge's order, which is the order the damage routine applies them:
/// `(attacker, defender, multiplier × 10)`.
pub fn matchups() -> Vec<(u8, u8, u8)> {
    TYPE_EFFECTS.to_vec()
}

const GHOST: u8 = PokemonType::Ghost as u8;
const PSYCHIC: u8 = PokemonType::Psychic as u8;

/// [`matchups`] as `ruleset` plays them. A move takes each row for its type once when either of
/// the defender's types is the row's, so a single type stored in both slots counts once.
pub fn chart(ruleset: Ruleset) -> impl Iterator<Item = (u8, u8, u8)> {
    TYPE_EFFECTS.iter().map(move |&row| match row {
        // The cartridge's chart gives Ghost no effect on Psychic.
        (GHOST, PSYCHIC, NO_EFFECT) if !ruleset.is_gen1() => (GHOST, PSYCHIC, SUPER_EFFECTIVE),
        row => row,
    })
}

#[cfg(test)]
mod tests {
    use crate::pokemon::MoveEffectiveness;
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
