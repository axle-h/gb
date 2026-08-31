use crate::pokemon::battle::{BattleAction, BattleState};
use crate::pokemon::move_name::{PokemonMoveEffect, PokemonMoveName};
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

/// What a move is worth **per turn**, which is what choosing between two of them is actually about.
///
/// ⚠️ **A charge move is half the damage and a free hit for the opponent, and ranking on raw power
/// hides both.** `PokemonMoveEffect::Charge` — Skull Bash, Razor Wind, Solarbeam, Sky Attack and Dig
/// — spends the first turn winding up, so its 100 power is 50 a turn, and the enemy attacks into it.
/// Measured in Koga's gym: a Blastoise holding Surf *and* Dig picked **Skull Bash** (equal raw power,
/// Normal against a Poison type), was badly poisoned mid-charge, and fainted without landing it — and
/// the Oddish behind it went down with the gym.
///
/// Halving is enough to get this right without a special case: Dig into Poison is 2× before the
/// halving and still wins, which is what the route relies on for Koga, Surge and Giovanni.
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
                    // ⚠️ Ranked per turn, but the *catching* guard above stays on the raw number:
                    // what must not happen there is a knockout, and a charge move lands its full
                    // damage when it finally goes off.
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

/// The combined type multiplier `move_name` gets against `defender`, both of the defender's types
/// folded together: `0.0`, `0.25`, `0.5`, `1.0`, `2.0` or `4.0`.
///
/// ⚠️ **One function, three callers, on purpose.** The battle script reads it as `mv.effectiveness`,
/// `tools::battle_menu` prints it on the `fight:` row, and both have to agree with each other and
/// with `expected_damage`, which folds the same multiplier in internally. A model that installs a
/// script off one set of numbers and then answers a fallback turn off another has two type charts to
/// reconcile, which is the thing this exists to stop it needing at all.
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

/// [`type_multiplier`] as the cartridge's own words for it, or `None` at 1.0 — where the game says
/// nothing and so does this.
///
/// ⚠️ **A multiplier of 1.0 prints nothing rather than "normally effective".** The same argument as
/// `prompt::ailment` and the `34 → 34` rule in `battle_report`: a phrase on every row for the
/// commonest case is noise that buries the two rows where the number is the whole decision.
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
