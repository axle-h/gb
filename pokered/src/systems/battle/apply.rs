//! `ApplyAttackToEnemyPokemon` and `ApplyAttackToPlayerPokemon`, `AttackSubstitute` and
//! `HandleBuildingRage`: a move's damage landing.

use poke_core::move_name::PokemonMoveName;
use crate::rng::Rng;
use super::effects::stat_modifiers::stat_modifier_up_effect;
use super::effects::BattleText;
use super::{effect, Battle, Side, Status2, MAX_STAT_LEVEL};

const SONICBOOM_DAMAGE: u8 = 20;
const DRAGON_RAGE_DAMAGE: u8 = 40;

/// `ApplyAttackToEnemyPokemon` or `ApplyAttackToPlayerPokemon` for `attacker`'s move: a one-hit KO's
/// 65535, half the target's HP for Super Fang, the user's level for Seismic Toss and Night Shade, 20
/// and 40 for Sonic Boom and Dragon Rage, and for Psywave a random byte below one and a half times
/// the level, in a byte, which the player's copy also draws again at 0. A move with no power
/// deals nothing.
pub fn apply_attack_to_pokemon(battle: &mut Battle, attacker: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let user = battle.side(attacker);
    let (current, level) = (user.current_move, user.mon.level);
    match current.effect {
        effect::OHKO_EFFECT => {}
        effect::SUPER_FANG_EFFECT => {
            battle.damage = (battle.side(attacker.other()).mon.hp >> 1).max(1);
        }
        effect::SPECIAL_DAMAGE_EFFECT => {
            let name = current.animation;
            let damage = if name == PokemonMoveName::SeismicToss as u8 || name == PokemonMoveName::NightShade as u8 {
                level
            } else if name == PokemonMoveName::Sonicboom as u8 {
                SONICBOOM_DAMAGE
            } else if name == PokemonMoveName::DragonRage as u8 {
                DRAGON_RAGE_DAMAGE
            } else {
                let bound = level.wrapping_add(level >> 1);
                loop {
                    let random = rng.random();
                    if (attacker == Side::Enemy || random != 0) && random < bound {
                        break random;
                    }
                }
            };
            battle.damage = damage as u16;
        }
        _ if current.power == 0 => return vec![],
        _ => {}
    }
    apply_damage_to_pokemon(battle, attacker.other(), attacker)
}

/// `ApplyDamageToEnemyPokemon` or `ApplyDamageToPlayerPokemon`: `wDamage` off `target`'s HP, or
/// off a substitute if it has one; overkill leaves 0 HP and `wDamage` what the HP was. `turn` is
/// `hWhoseTurn`, which `AttackSubstitute` reads: self-inflicted damage hits the *other* substitute.
pub fn apply_damage_to_pokemon(battle: &mut Battle, target: Side, turn: Side) -> Vec<BattleText> {
    if battle.damage == 0 {
        return vec![];
    }
    if battle.side(target).status2.contains(Status2::HAS_SUBSTITUTE_UP) {
        return attack_substitute(battle, turn);
    }
    let damage = battle.damage;
    let mon = &mut battle.side_mut(target).mon;
    match mon.hp.checked_sub(damage) {
        Some(hp) => mon.hp = hp,
        None => {
            let hp = mon.hp;
            mon.hp = 0;
            battle.damage = hp;
        }
    }
    vec![]
}

/// `AttackSubstitute` on `turn`'s target. Damage over a byte breaks it outright; otherwise the
/// low byte comes off its HP, and only a borrow breaks it, so a substitute can stand at 0. A break
/// does not touch `wDamage` and zeroes the attacker's move effect.
pub fn attack_substitute(battle: &mut Battle, turn: Side) -> Vec<BattleText> {
    let mut texts = vec![BattleText::SubstituteTookDamageText];
    let damage = battle.damage;
    let victim = battle.side_mut(turn.other());
    if damage >> 8 == 0 {
        let (hp, borrow) = victim.substitute_hp.overflowing_sub(damage as u8);
        victim.substitute_hp = hp;
        if !borrow {
            return texts;
        }
    }
    victim.status2.remove(Status2::HAS_SUBSTITUTE_UP);
    texts.push(BattleText::SubstituteBrokeText);
    battle.side_mut(turn).current_move.effect = 0;
    texts
}

/// `HandleBuildingRage`: a target using Rage, below +6 attack, raises it a stage as though its own
/// move did, and is left with Rage's number and no effect.
pub fn handle_building_rage(battle: &mut Battle, attacker: Side, badges: u8) -> Vec<BattleText> {
    let raging = attacker.other();
    let side = battle.side_mut(raging);
    if !side.status2.contains(Status2::USING_RAGE) || side.stat_mods[0] == MAX_STAT_LEVEL {
        return vec![];
    }
    side.current_move.animation = 0;
    side.current_move.effect = effect::ATTACK_UP1_EFFECT;
    let mut texts = vec![BattleText::BuildingRageText];
    texts.extend(stat_modifier_up_effect(battle, raging, badges));
    let side = battle.side_mut(raging);
    side.current_move.effect = 0;
    side.current_move.animation = PokemonMoveName::Rage as u8;
    texts
}


#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use super::super::fixture::{each_case, side};
    use super::*;

    #[test]
    fn every_harvested_case_of_apply_attack_to_pokemon() {
        each_case(include_str!("../../../fixtures/battle/apply_attack_to_pokemon.jsonl"), |arena, input, rng| {
            json!([Value::Null, apply_attack_to_pokemon(&mut arena.battle, side(input), rng)])
        });
    }

    #[test]
    fn every_harvested_case_of_handle_building_rage() {
        each_case(include_str!("../../../fixtures/battle/handle_building_rage.jsonl"), |arena, input, _| {
            json!([Value::Null, handle_building_rage(&mut arena.battle, side(input), arena.badges)])
        });
    }
}
