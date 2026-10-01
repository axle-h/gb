use crate::battle::{BattleAction, BattleState};
use crate::move_name::{PokemonMoveEffect, PokemonMoveName};
use crate::pokemon::{PokemonSummary, PokemonType, PokemonTypeCategory};
use crate::ruleset::Ruleset;
use crate::types;

fn expected_psywave_damage(level: u8) -> u16 {
    // Uniform in [1, floor(1.5 × level)].
    let n = (level as f64 * 1.5).floor() as u16;
    ((n + 1) as f64 / 2.0).round() as u16
}

/// A move that deals direct damage: the fixed-damage cases `expected_damage` opens with, or power.
pub fn is_damaging_move(name: PokemonMoveName) -> bool {
    matches!(
        name,
        PokemonMoveName::SeismicToss
            | PokemonMoveName::NightShade
            | PokemonMoveName::Sonicboom
            | PokemonMoveName::DragonRage
            | PokemonMoveName::Psywave
    ) || name.metadata().power.is_some()
}

/// The multipliers, in tenths and in the cartridge's order, that a move of `move_type` takes
/// against `defender`: one per chart row naming either of its types (`AdjustDamageForMoveType`).
fn multipliers(move_type: PokemonType, defender: &PokemonSummary, ruleset: Ruleset) -> impl Iterator<Item = u32> + '_ {
    types::chart(ruleset)
        .filter(move |&(attacking, defending, _)| {
            attacking == move_type as u8 && defender.types.iter().any(|&t| t as u8 == defending)
        })
        .map(|(_, _, tenths)| tenths as u32)
}

/// Approximate damage, with no crit and no stat stages, or `None` when the move cannot deal any.
pub fn expected_damage(attacker: &PokemonSummary, move_name: PokemonMoveName, defender: &PokemonSummary, ruleset: Ruleset) -> Option<u16> {

    match move_name {
        PokemonMoveName::SeismicToss | PokemonMoveName::NightShade => return Some(attacker.level as u16),
        PokemonMoveName::Sonicboom => return Some(20),
        PokemonMoveName::DragonRage => return Some(40),
        PokemonMoveName::Psywave => return Some(expected_psywave_damage(attacker.level)),
        _ => {}
    }

    let metadata = move_name.metadata();

    let power = metadata.power? as u32;

    let (a, d) = match metadata.move_type.category() {
        PokemonTypeCategory::Physical => (attacker.stats.attack, defender.stats.defense),
        PokemonTypeCategory::Special  => (attacker.stats.special, defender.stats.special),
    };

    // Stats unmodified, as there is no stage information, and no Reflect or Light Screen.
    let (mut a, mut d) = (a as u32, d as u32);

    if a >= 256 || d >= 256 {
        a = (a / 4) % 256;
        d = (d / 4) % 256;
        if a == 0 { a = 1; }
    }

    // The cartridge divides by zero here; Modern floors the defense at 1.
    if d == 0 {
        match ruleset {
            Ruleset::Gen1 => return None,
            Ruleset::Modern => d = 1,
        }
    }

    let l = attacker.level as u32;

    let base = ((l * 2 / 5 + 2) * power * a / d) / 50;
    let base = base.min(997) + 2;

    let mut damage = base;

    // Same-type attack bonus.
    if attacker.types.contains(&metadata.move_type) {
        damage += base / 2;
    }

    for tenths in multipliers(metadata.move_type, defender, ruleset) {
        damage = damage * tenths / 10;
    }

    // Damage that rounds down to 0 misses.
    if damage == 0 { return None; }
    Some(damage as u16)
}

/// A charge move is worth half its damage per turn.
fn damage_per_turn(name: PokemonMoveName, damage: u16) -> u16 {
    match name.metadata().effect {
        PokemonMoveEffect::Charge => damage / 2,
        _ => damage,
    }
}

pub fn pick_best_move(battle_state: &BattleState, actions: &[BattleAction], catching_pokemon: bool) -> Option<BattleAction> {
    actions.iter()
        .filter_map(|a| match a {
            BattleAction::Fight { battle_move, .. } => {
                let dmg = expected_damage(&battle_state.player, battle_move.name, &battle_state.enemy, battle_state.ruleset)?;
                if dmg > 0 && (!catching_pokemon || dmg < battle_state.enemy.current_hp) {
                    // The catching guard stays on raw damage: a charge move lands it all when it goes off.
                    Some((damage_per_turn(battle_move.name, dmg), *a))
                } else {
                    None
                }
            }
            _ => None,
        })
        .max_by_key(|(dmg, _)| *dmg)
        .map(|(_, a)| a)
}

#[cfg(test)]
mod test {
    use crate::pokemon::Pokemon;
    use crate::species::PokemonSpecies;
    use super::*;

    fn alakazam() -> PokemonSummary {
        Pokemon::maxed(
            PokemonSpecies::Alakazam,
            "ALAKAZAM",
            [
                PokemonMoveName::Psychic,
                PokemonMoveName::SeismicToss,
                PokemonMoveName::Recover,
                PokemonMoveName::ThunderWave,
            ],
            "TEST",
            1,
        ).summary()
    }

    fn arcanine() -> PokemonSummary {
        Pokemon::maxed(
            PokemonSpecies::Arcanine,
            "ARCANINE",
            [
                PokemonMoveName::FireBlast,
                PokemonMoveName::BodySlam,
                PokemonMoveName::HyperBeam,
                PokemonMoveName::Agility,
            ],
            "TEST",
            1,
        ).summary()
    }

    #[test]
    fn test_alakazam() {
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::Psychic, &arcanine(), Ruleset::Gen1), Some(165)); // psychic
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::SeismicToss, &arcanine(), Ruleset::Gen1), Some(100)); // seismic toss
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::Recover, &arcanine(), Ruleset::Gen1), None); // recover
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::ThunderWave, &arcanine(), Ruleset::Gen1), None); // thunder wave
    }

    fn summary(species: PokemonSpecies, name: &str) -> PokemonSummary {
        Pokemon::maxed(species, name, [PokemonMoveName::Tackle; 4], "TEST", 1).summary()
    }

    /// A single type is stored in both slots and the cartridge counts it once.
    #[test]
    fn water_on_charmander_is_double_not_quadruple() {
        let blastoise = summary(PokemonSpecies::Blastoise, "BLASTOISE");
        let charmander = summary(PokemonSpecies::Charmander, "CHARMANDER");
        let normal = PokemonSummary { types: [PokemonType::Normal; 2], ..charmander };
        for ruleset in [Ruleset::Gen1, Ruleset::Modern] {
            assert_eq!(type_multiplier(PokemonMoveName::Surf, &charmander, ruleset), 2.0);
            assert_eq!(effectiveness_phrase(type_multiplier(PokemonMoveName::Surf, &charmander, ruleset)), Some("super effective"));
            let surf = |defender| expected_damage(&blastoise, PokemonMoveName::Surf, defender, ruleset).unwrap();
            assert_eq!(surf(&charmander), surf(&normal) * 2);
        }
    }

    #[test]
    fn ice_on_dragon_flying_is_quadruple() {
        let dragonite = summary(PokemonSpecies::Dragonite, "DRAGONITE");
        for ruleset in [Ruleset::Gen1, Ruleset::Modern] {
            assert_eq!(type_multiplier(PokemonMoveName::IceBeam, &dragonite, ruleset), 4.0);
            assert_eq!(effectiveness_phrase(type_multiplier(PokemonMoveName::IceBeam, &dragonite, ruleset)), Some("doubly super effective"));
        }
    }

    #[test]
    fn a_single_type_immunity_is_no_damage() {
        let gengar = summary(PokemonSpecies::Gengar, "GENGAR");
        let diglett = summary(PokemonSpecies::Diglett, "DIGLETT");
        let pikachu = summary(PokemonSpecies::Pikachu, "PIKACHU");
        for ruleset in [Ruleset::Gen1, Ruleset::Modern] {
            assert_eq!(type_multiplier(PokemonMoveName::Thunderbolt, &diglett, ruleset), 0.0);
            assert_eq!(expected_damage(&pikachu, PokemonMoveName::Thunderbolt, &diglett, ruleset), None);
            assert_eq!(expected_damage(&pikachu, PokemonMoveName::Tackle, &gengar, ruleset), None);
        }
    }

    /// The recreation's Modern ruleset doubles Ghost on Psychic where the cartridge's chart says no
    /// effect, and the estimate follows whichever is being played.
    #[test]
    fn lick_on_a_psychic_type_follows_the_ruleset() {
        let gengar = Pokemon::maxed(PokemonSpecies::Gengar, "GENGAR", [PokemonMoveName::Lick; 4], "TEST", 1).summary();
        let slowbro = Pokemon::maxed(PokemonSpecies::Slowbro, "SLOWBRO", [PokemonMoveName::Surf; 4], "TEST", 1).summary();
        let lick = |ruleset| (expected_damage(&gengar, PokemonMoveName::Lick, &slowbro, ruleset),
                              type_multiplier(PokemonMoveName::Lick, &slowbro, ruleset));
        assert_eq!(lick(Ruleset::Gen1), (None, 0.0));
        let (damage, multiplier) = lick(Ruleset::Modern);
        assert!(damage.is_some_and(|damage| damage > 0), "{damage:?}");
        assert_eq!(multiplier, 2.0);
    }
}

/// The multiplier `move_name` gets against both of `defender`'s types together.
pub fn type_multiplier(move_name: PokemonMoveName, defender: &PokemonSummary, ruleset: Ruleset) -> f64 {
    multipliers(move_name.metadata().move_type, defender, ruleset).fold(1.0, |total, tenths| total * tenths as f64 / 10.0)
}

/// [`type_multiplier`] in the cartridge's own words, or `None` at 1.0 where the game says nothing.
pub fn effectiveness_phrase(multiplier: f64) -> Option<&'static str> {
    match multiplier {
        m if m == 0.0 => Some("no effect"),
        m if m >= 4.0 => Some("doubly super effective"),
        m if m >= 2.0 => Some("super effective"),
        m if m <= 0.25 => Some("doubly resisted"),
        m if m < 1.0 => Some("not very effective"),
        _ => None,
    }
}
