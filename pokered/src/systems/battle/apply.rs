//! `ApplyAttackToEnemyPokemon` and `ApplyAttackToPlayerPokemon`, `AttackSubstitute` and
//! `HandleBuildingRage`: a move's damage landing.

use poke_core::move_name::PokemonMoveName;
use crate::rng::Rng;
use super::effects::stat_modifiers::stat_modifier_up_effect;
use super::damage::{FIGHTING, NORMAL};
use super::effects::BattleText;
use super::{effect, Battle, Side, Status2, MAX_STAT_LEVEL};

const SONICBOOM_DAMAGE: u8 = 20;
const DRAGON_RAGE_DAMAGE: u8 = 40;

/// `ApplyAttackToEnemyPokemon` or `ApplyAttackToPlayerPokemon` for `attacker`'s move: a one-hit KO's
/// 65535, half the target's HP for Super Fang, the user's level for Seismic Toss and Night Shade, 20
/// and 40 for Sonic Boom and Dragon Rage, and for Psywave a random byte from 1 to below one and a
/// half times the level, in a byte, and never below 2. A move with no power deals nothing.
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
                let mut bound = level.wrapping_add(level >> 1);
                // The cartridge lets the enemy roll 0, and the player's roll never ends at level 1.
                let zero_allowed = battle.cartridge_bugs && attacker == Side::Enemy;
                if !battle.cartridge_bugs {
                    bound = bound.max(2);
                }
                loop {
                    let random = rng.random();
                    if (zero_allowed || random != 0) && random < bound {
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
/// `hWhoseTurn`: damage a mon does itself ignores substitutes. What `turn`'s move did to the other
/// mon is kept for Counter.
pub fn apply_damage_to_pokemon(battle: &mut Battle, target: Side, turn: Side) -> Vec<BattleText> {
    if battle.damage == 0 {
        return vec![];
    }
    let texts = damage_hp_or_substitute(battle, target, turn);
    let current = battle.side(turn).current_move;
    if !battle.cartridge_bugs && target != turn && current.power != 0
        && matches!(current.move_type, NORMAL | FIGHTING) && current.animation != PokemonMoveName::Counter as u8 {
        battle.side_mut(turn).counter_damage = battle.damage;
    }
    texts
}

fn damage_hp_or_substitute(battle: &mut Battle, target: Side, turn: Side) -> Vec<BattleText> {
    // The cartridge swaps `hWhoseTurn` for self-inflicted damage, so it hits the *other* substitute.
    let self_inflicted = target == turn && !battle.cartridge_bugs;
    if battle.side(target).status2.contains(Status2::HAS_SUBSTITUTE_UP) && !self_inflicted {
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
/// leaves `wDamage` what the substitute had and stops the move's effect on the target, not the
/// effects on its user.
pub fn attack_substitute(battle: &mut Battle, turn: Side) -> Vec<BattleText> {
    let mut texts = vec![BattleText::SubstituteTookDamageText];
    let damage = battle.damage;
    let cartridge_bugs = battle.cartridge_bugs;
    let victim = battle.side_mut(turn.other());
    let substitute_hp = victim.substitute_hp;
    if damage >> 8 == 0 {
        let (hp, borrow) = victim.substitute_hp.overflowing_sub(damage as u8);
        victim.substitute_hp = hp;
        if !borrow {
            return texts;
        }
    }
    victim.status2.remove(Status2::HAS_SUBSTITUTE_UP);
    texts.push(BattleText::SubstituteBrokeText);
    let attacker = battle.side_mut(turn);
    // The cartridge zeroes the whole effect, so the user's own recoil, drain, recharge and
    // self-KO are lost too, and leaves `wDamage` the full hit.
    if cartridge_bugs || !ATTACKERS_OWN_EFFECTS.contains(&attacker.current_move.effect) {
        attacker.current_move.effect = 0;
    }
    if !cartridge_bugs {
        battle.damage = substitute_hp as u16;
    }
    texts
}

/// Effects that act on the move's user alone, which a substitute breaking does not stop.
const ATTACKERS_OWN_EFFECTS: [u8; 7] = [effect::RECOIL_EFFECT, effect::DRAIN_HP_EFFECT, effect::DREAM_EATER_EFFECT,
    effect::HYPER_BEAM_EFFECT, effect::EXPLODE_EFFECT, effect::PAY_DAY_EFFECT, effect::RAGE_EFFECT];

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
    use poke_core::moves::MoveData;
    use serde_json::{json, Value};
    use crate::rng::GameRng;
    use super::super::fixture::{each_case, side};
    use super::super::turn::handle_self_confusion_damage;
    use super::super::Arena;
    use super::*;

    #[test]
    fn every_harvested_case_of_apply_attack_to_pokemon() {
        each_case(include_str!("../../../fixtures/battle/apply_attack_to_pokemon.jsonl"), |arena, input, rng| {
            json!([Value::Null, apply_attack_to_pokemon(&mut arena.battle, side(input), rng)])
        });
    }

    fn psywave(attacker: Side) -> u16 {
        let mut arena = Arena::baseline();
        arena.battle.cartridge_bugs = false;
        let user = arena.battle.side_mut(attacker);
        user.current_move = MoveData::of_move(PokemonMoveName::Psywave);
        user.mon.level = 1;
        apply_attack_to_pokemon(&mut arena.battle, attacker, &mut GameRng::tape(vec![0, 1]));
        arena.battle.damage
    }

    #[test]
    fn psywave_at_level_1_does_1_for_either_side() {
        assert_eq!(psywave(Side::Player), 1);
        assert_eq!(psywave(Side::Enemy), 1);
    }

    #[test]
    fn damage_a_mon_does_itself_ignores_substitutes() {
        let mut arena = Arena::baseline();
        arena.battle.cartridge_bugs = false;
        for side in [Side::Player, Side::Enemy] {
            let combatant = arena.battle.side_mut(side);
            combatant.status2 |= Status2::HAS_SUBSTITUTE_UP;
            combatant.substitute_hp = 50;
        }
        let party = arena.party.clone();
        handle_self_confusion_damage(&mut arena.battle, &party, Side::Player);
        let (player, enemy) = (&arena.battle.player, &arena.battle.enemy);
        assert!(player.mon.hp < player.mon.stats[0]);
        assert_eq!((player.substitute_hp, enemy.substitute_hp), (50, 50));
    }

    /// The player's `name` doing 100 damage to an enemy substitute of 10 HP.
    fn breaking_a_substitute(name: PokemonMoveName, cartridge_bugs: bool) -> Battle {
        let mut arena = Arena::baseline();
        arena.battle.cartridge_bugs = cartridge_bugs;
        arena.battle.player.current_move = MoveData::of_move(name);
        arena.battle.enemy.status2 |= Status2::HAS_SUBSTITUTE_UP;
        arena.battle.enemy.substitute_hp = 10;
        arena.battle.damage = 100;
        apply_damage_to_pokemon(&mut arena.battle, Side::Enemy, Side::Player);
        arena.battle
    }

    #[test]
    fn breaking_a_substitute_keeps_the_users_own_effect_and_stops_the_targets() {
        use PokemonMoveName::*;
        let effect_after = |name, cartridge_bugs| breaking_a_substitute(name, cartridge_bugs).player.current_move.effect;
        for name in [DoubleEdge, Explosion, HyperBeam] {
            assert_eq!(effect_after(name, false), MoveData::of_move(name).effect);
            assert_eq!(effect_after(name, true), 0);
        }
        assert_eq!(effect_after(Flamethrower, false), 0);
        assert_eq!(breaking_a_substitute(DoubleEdge, false).damage, 10);
        assert_eq!(breaking_a_substitute(DoubleEdge, true).damage, 100);
    }

    #[test]
    fn every_harvested_case_of_handle_building_rage() {
        each_case(include_str!("../../../fixtures/battle/handle_building_rage.jsonl"), |arena, input, _| {
            json!([Value::Null, handle_building_rage(&mut arena.battle, side(input), arena.badges)])
        });
    }
}
