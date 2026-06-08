use crate::pokemon::battle::{BattleAction, BattleState};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::pokemon::{MoveEffectiveness, Pokemon, PokemonSummary, PokemonTypeCategory};

fn expected_psywave_damage(level: u8) -> u16 {
    // Psywave deals uniform random damage in [1, floor(1.5 × level)].
    // Expected value of a uniform distribution over {1..n} is (n+1)/2.
    let n = (level as f64 * 1.5).floor() as u16;
    ((n + 1) as f64 / 2.0).round() as u16
}

/// Returns `true` if `name` is a move that deals direct damage.
/// Mirrors the special cases at the top of `expected_damage` in `damage.rs` as well as
/// ordinary moves that have a positive base power.
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

/// Returns approximate damage dealt by `move_name` used by `attacker` against `defender`,
/// or `None` if the move cannot deal damage (zero power, or the defender is immune).
///
/// Assumes no critical hit, no Reflect/Light Screen, and all stat stages at zero.
pub fn expected_damage(attacker: &PokemonSummary, move_name: PokemonMoveName, defender: &PokemonSummary) -> Option<u16> {

    match move_name {
        // https://bulbapedia.bulbagarden.net/wiki/Seismic_Toss_(move)#Generation_I
        // https://bulbapedia.bulbagarden.net/wiki/Night_Shade_(move)#Generation_I
        PokemonMoveName::SeismicToss | PokemonMoveName::NightShade => return Some(attacker.level as u16),
        // https://bulbapedia.bulbagarden.net/wiki/Sonic_Boom_(move)
        PokemonMoveName::Sonicboom => return Some(20),
        // https://bulbapedia.bulbagarden.net/wiki/Dragon_Rage_(move)
        PokemonMoveName::DragonRage => return Some(40),
        // https://bulbapedia.bulbagarden.net/wiki/Psywave_(move)
        PokemonMoveName::Psywave => return Some(expected_psywave_damage(attacker.level)),
        _ => {}
    }

    let metadata = move_name.metadata();

    // Status moves have no power and deal no damage.
    let power = metadata.power? as u32;

    // Calculating A and D:
    // For special moves, substitute Attack and Defense for Special.
    let (a, d) = match metadata.move_type.category() {
        PokemonTypeCategory::Physical => (attacker.stats.attack, defender.stats.defense),
        PokemonTypeCategory::Special  => (attacker.stats.special, defender.stats.special),
    };

    // No critical hit, so use the modified stats (here treated as unmodified since we
    // have no in-battle stage information), with no Reflect/Light Screen doubling of D.
    let (mut a, mut d) = (a as u32, d as u32);

    // If A or D >= 256, divide both by 4 and reduce modulo 256. If A becomes 0, make it 1.
    if a >= 256 || d >= 256 {
        a = (a / 4) % 256;
        d = (d / 4) % 256;
        if a == 0 { a = 1; }
    }

    // D == 0 causes division by zero in the original game; treat as no damage.
    if d == 0 { return None; }

    // L = attacker's level (no critical hit, so no doubling).
    let l = attacker.level as u32;

    // Base damage:
    //   1. Take L.
    //   2. Multiply by 2.
    //   3. Divide by 5, rounding down.
    //   4. Add 2.
    //   5. Multiply by move's power.
    //   6. Multiply by A.
    //   7. Divide by D, rounding down.
    //   8. Divide by 50, rounding down.
    //   9. Decrease to 997 if higher.
    //  10. Add 2.
    let base = ((l * 2 / 5 + 2) * power * a / d) / 50;
    let base = base.min(997) + 2;

    // Modified damage:
    //   1. Take Base damage.
    let mut damage = base;

    //   2. STAB: if the move shares its type with one of the attacker's, add Base damage / 2.
    if attacker.types.contains(&metadata.move_type) {
        damage += base / 2;
    }

    //   3-5. Apply type effectiveness against each of the defender's types.
    //        ×20/10 = super effective, ×10/10 = neutral, ×5/10 = not very effective, None = no effect.
    for &def_type in &defender.types {
        damage = match metadata.move_type.attack_effectiveness(def_type) {
            MoveEffectiveness::Double => damage * 20 / 10,
            MoveEffectiveness::Base   => damage,
            MoveEffectiveness::Half   => damage * 5 / 10,
            MoveEffectiveness::None   => return None,
        };
    }

    // If damage rounded down to 0, the move misses.
    if damage == 0 { return None; }
    Some(damage as u16)
}

pub fn pick_best_move(battle_state: &BattleState, actions: &[BattleAction], catching_pokemon: bool) -> Option<BattleAction> {
    actions.iter()
        .filter_map(|a| match a {
            BattleAction::Fight { battle_move, .. } => {
                let dmg = expected_damage(&battle_state.player, battle_move.name, &battle_state.enemy)?;
                if dmg > 0 && (!catching_pokemon || dmg < battle_state.enemy.current_hp) {
                    Some((dmg, *a))
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
    use crate::pokemon::pokemon::{Pokemon, PokemonType};
    use crate::pokemon::species::PokemonSpecies;
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
