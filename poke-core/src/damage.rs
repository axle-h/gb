use crate::battle::{BattleAction, BattleState};
use crate::move_name::{PokemonMoveEffect, PokemonMoveName};
use crate::pokemon::{MoveEffectiveness, PokemonSummary, PokemonTypeCategory};

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

/// Approximate damage, with no crit and no stat stages, or `None` when the move cannot deal any.
pub fn expected_damage(attacker: &PokemonSummary, move_name: PokemonMoveName, defender: &PokemonSummary) -> Option<u16> {

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

    // The cartridge divides by zero here.
    if d == 0 { return None; }

    let l = attacker.level as u32;

    let base = ((l * 2 / 5 + 2) * power * a / d) / 50;
    let base = base.min(997) + 2;

    let mut damage = base;

    // Same-type attack bonus.
    if attacker.types.contains(&metadata.move_type) {
        damage += base / 2;
    }

    for &def_type in &defender.types {
        damage = match metadata.move_type.attack_effectiveness(def_type) {
            MoveEffectiveness::Double => damage * 20 / 10,
            MoveEffectiveness::Base   => damage,
            MoveEffectiveness::Half   => damage * 5 / 10,
            MoveEffectiveness::None   => return None,
        };
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
                let dmg = expected_damage(&battle_state.player, battle_move.name, &battle_state.enemy)?;
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
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::Psychic, &arcanine()), Some(165)); // psychic
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::SeismicToss, &arcanine()), Some(100)); // seismic toss
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::Recover, &arcanine()), None); // recover
        assert_eq!(expected_damage(&alakazam(), PokemonMoveName::ThunderWave, &arcanine()), None); // thunder wave
    }
}

/// The multiplier `move_name` gets against both of `defender`'s types together.
pub fn type_multiplier(move_name: PokemonMoveName, defender: &PokemonSummary) -> f64 {
    let move_type = move_name.metadata().move_type;
    defender.types.iter().fold(1.0, |total, &against| {
        total
            * match move_type.attack_effectiveness(against) {
                MoveEffectiveness::Double => 2.0,
                MoveEffectiveness::Base => 1.0,
                MoveEffectiveness::Half => 0.5,
                MoveEffectiveness::None => 0.0,
            }
    })
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
