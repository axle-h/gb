//! `MoveHitTest` and `CalcHitChance`: whether a move lands.

use poke_core::battle_data::stat_modifier_ratios;
use crate::rng::Rng;
use crate::systems::math::{divide, multiply};
use super::{effect, stat_mod, status, Battle, Side, Status1, Status2};

/// `MoveHitTest`. Dream Eater misses a target that is awake, Swift never misses, draining a
/// substitute misses, a target in the air or underground always evades, Mist turns away the
/// stat-lowering effects, and X Accuracy skips the roll. Otherwise a random byte below the scaled
/// accuracy hits, and 255 always does.
pub fn move_hit_test(battle: &mut Battle, attacker: Side, rng: &mut impl Rng) {
    let bugs = battle.cartridge_bugs;
    let move_effect = battle.side(attacker).current_move.effect;
    let target = battle.side(attacker.other());
    let missed = if move_effect == effect::DREAM_EATER_EFFECT && target.mon.status & status::SLP_MASK == 0 {
        true
    } else if move_effect == effect::SWIFT_EFFECT {
        return;
    } else if !bugs && target.status2.contains(Status2::HAS_SUBSTITUTE_UP)
        && matches!(move_effect, effect::DRAIN_HP_EFFECT | effect::DREAM_EATER_EFFECT) {
        // The cartridge compares the substitute check's answer instead of the effect, so never.
        true
    } else if target.status1.contains(Status1::INVULNERABLE) {
        true
    } else if is_blocked_by_mist(move_effect) && target.status2.contains(Status2::PROTECTED_BY_MIST) {
        true
    } else if battle.side(attacker).status2.contains(Status2::USING_X_ACCURACY) {
        return;
    } else {
        calc_hit_chance(battle, attacker);
        let accuracy = battle.side(attacker).current_move.accuracy;
        // The cartridge rolls against 255 too, so a sure hit misses one time in 256.
        if !bugs && accuracy == 0xFF {
            return;
        }
        rng.random() >= accuracy
    };
    if missed {
        battle.damage = 0;
        battle.move_missed = true;
        battle.side_mut(attacker).status1.remove(Status1::USING_TRAPPING_MOVE);
    }
}

/// `ATTACK_DOWN1_EFFECT` to `HAZE_EFFECT` and `ATTACK_DOWN2_EFFECT` to `REFLECT_EFFECT`, which
/// sweeps in Conversion, Haze, Light Screen and Reflect, though those never come here.
fn is_blocked_by_mist(move_effect: u8) -> bool {
    (effect::ATTACK_DOWN1_EFFECT..=effect::HAZE_EFFECT).contains(&move_effect)
        || (effect::ATTACK_DOWN2_EFFECT..=effect::REFLECT_EFFECT).contains(&move_effect)
}

/// `CalcHitChance`: the move's accuracy through the attacker's accuracy ratio and then the target's
/// evasion reflected about 7, each step never below 1, and capped at 255 into the move's accuracy.
pub fn calc_hit_chance(battle: &mut Battle, attacker: Side) {
    let ratios = stat_modifier_ratios();
    let accuracy_mod = battle.side(attacker).stat_mods[stat_mod::ACCURACY];
    let evasion_mod = 14u8.wrapping_sub(battle.side(attacker.other()).stat_mods[stat_mod::EVASION]);
    let mut chance = battle.side(attacker).current_move.accuracy as u32;
    let mut quotient = [0; 4];
    for stat_mod in [accuracy_mod, evasion_mod] {
        let (numerator, denominator) = ratios[stat_mod as usize - 1];
        (quotient, _) = divide(multiply(chance, numerator).to_be_bytes(), denominator, 4);
        if quotient[2] | quotient[3] == 0 {
            quotient[3] = 1;
        }
        chance = u32::from_be_bytes(quotient);
    }
    battle.side_mut(attacker).current_move.accuracy = if quotient[2] != 0 { 0xFF } else { quotient[3] };
}

#[cfg(test)]
mod tests {
    use poke_core::move_name::PokemonMoveName;
    use poke_core::moves::MoveData;
    use serde_json::Value;
    use crate::rng::GameRng;
    use super::super::fixture::{each_case, side};
    use super::super::Arena;
    use super::*;

    #[test]
    fn every_harvested_case_of_move_hit_test() {
        each_case(include_str!("../../../fixtures/battle/move_hit_test.jsonl"), |arena, input, rng| {
            move_hit_test(&mut arena.battle, side(input), rng);
            Value::Null
        });
    }

    fn using(name: PokemonMoveName) -> Arena {
        let mut arena = Arena::baseline();
        arena.battle.cartridge_bugs = false;
        arena.battle.player.current_move = MoveData::of_move(name);
        arena
    }

    #[test]
    fn a_sure_hit_never_misses() {
        let mut arena = using(PokemonMoveName::BodySlam);
        assert_eq!(arena.battle.player.current_move.accuracy, 0xFF);
        move_hit_test(&mut arena.battle, Side::Player, &mut GameRng::tape(vec![0xFF]));
        assert!(!arena.battle.move_missed);
    }

    #[test]
    fn draining_a_substitute_misses() {
        for name in [PokemonMoveName::Absorb, PokemonMoveName::DreamEater] {
            let mut arena = using(name);
            arena.battle.enemy.mon.status = 1;
            arena.battle.enemy.status2 |= Status2::HAS_SUBSTITUTE_UP;
            move_hit_test(&mut arena.battle, Side::Player, &mut GameRng::tape(vec![0]));
            assert!(arena.battle.move_missed, "{name:?}");
        }
    }

    #[test]
    fn every_harvested_case_of_calc_hit_chance() {
        each_case(include_str!("../../../fixtures/battle/calc_hit_chance.jsonl"), |arena, input, _| {
            calc_hit_chance(&mut arena.battle, side(input));
            Value::Null
        });
    }
}
