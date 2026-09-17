//! `StatModifierUpEffect` and `StatModifierDownEffect`.

use poke_core::battle_data::stat_modifier_ratios;
use poke_core::move_name::PokemonMoveName;
use crate::rng::Rng;
use crate::systems::math::{divide, multiply};
use crate::systems::stats::MAX_STAT_VALUE;
use super::super::accuracy::move_hit_test;
use super::super::modified_stats::{apply_badge_stat_boosts, apply_burn_and_paralysis_penalties};
use super::super::{effect, stat, Battle, Side, Status1, Status2, MAX_STAT_LEVEL};
use super::BattleText;

/// `25 percent + 1`: below it, an enemy's stat-lowering move misses outright.
const ENEMY_STAT_DOWN_MISS: u8 = 64;
/// `33 percent + 1`: below it, a stat-lowering side effect happens.
const STAT_DOWN_SIDE_EFFECT_CHANCE: u8 = 85;

/// The unmodified stat through its modifier's ratio, in the two bytes the routines keep.
fn modified(unmodified: u16, stat_mod: u8) -> u16 {
    let (numerator, denominator) = stat_modifier_ratios()[stat_mod as usize - 1];
    let (quotient, _) = divide(multiply(unmodified as u32, numerator).to_be_bytes(), denominator, 4);
    u16::from_be_bytes([quotient[2], quotient[3]])
}

/// `StatModifierUpEffect`: one stage, or two for the `_UP2` effects without passing +6, and
/// nothing at +6 already or at a stat of exactly 999. The stat is worked out from the unmodified one
/// with no penalties or badge boosts, and then the player's badge boosts are applied to every stat
/// again, and the *other* mon's paralysis and burn penalties again, however many times before.
pub fn stat_modifier_up_effect(battle: &mut Battle, user: Side, badges: u8) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    let move_effect = side.current_move.effect;
    let mut which = move_effect.wrapping_sub(effect::ATTACK_UP1_EFFECT);
    if which >= effect::EVASION_UP1_EFFECT + 3 - effect::ATTACK_UP1_EFFECT {
        which = which.wrapping_sub(effect::ATTACK_UP2_EFFECT - effect::ATTACK_UP1_EFFECT);
    }
    let which = which as usize;
    let mut stage = side.stat_mods[which] + 1;
    if stage > MAX_STAT_LEVEL {
        return vec![BattleText::NothingHappenedText];
    }
    if move_effect >= effect::ATTACK_UP1_EFFECT + 8 {
        stage = (stage + 1).min(MAX_STAT_LEVEL);
    }
    side.stat_mods[which] = stage;
    if which < 4 {
        let index = stat::ATTACK + which;
        if side.mon.stats[index] == MAX_STAT_VALUE {
            side.stat_mods[which] -= 1;
            return vec![BattleText::NothingHappenedText];
        }
        side.mon.stats[index] = modified(side.unmodified_stats[index], stage).min(MAX_STAT_VALUE);
    }
    if side.current_move.animation == PokemonMoveName::Minimize as u8 {
        side.minimized = 1;
    }
    if user == Side::Player {
        apply_badge_stat_boosts(battle, badges);
    }
    apply_burn_and_paralysis_penalties(battle, user.other());
    vec![BattleText::MonsStatsRoseText]
}

/// `StatModifierDownEffect`, on the user's target. The enemy's copy misses a quarter of the time
/// before anything else, a substitute blocks it, and a side effect lands a third of the time
/// without a hit test while the move itself takes one. One stage, or two for the `_DOWN2` effects,
/// not below -6, and nothing for a stat of exactly 1. The stat is worked out afresh, never below 1,
/// then the player's badge boosts are applied again when the enemy used it, and the target's
/// paralysis and burn penalties again. A side effect that fails says nothing.
pub fn stat_modifier_down_effect(battle: &mut Battle, user: Side, badges: u8, rng: &mut impl Rng) -> Vec<BattleText> {
    let move_effect = battle.side(user).current_move.effect;
    let side_effect = move_effect >= effect::ATTACK_DOWN_SIDE_EFFECT;
    let missed = |battle: &Battle| match side_effect || battle.move_didnt_miss {
        true => vec![],
        false => vec![BattleText::ButItFailedText],
    };
    let cant_lower = || if side_effect { vec![] } else { vec![BattleText::NothingHappenedText] };
    let target = user.other();
    if user == Side::Enemy && rng.random() < ENEMY_STAT_DOWN_MISS {
        return missed(battle);
    }
    if battle.side(target).status2.contains(Status2::HAS_SUBSTITUTE_UP) {
        return missed(battle);
    }
    let which = if side_effect {
        if rng.random() >= STAT_DOWN_SIDE_EFFECT_CHANCE {
            return cant_lower();
        }
        move_effect - effect::ATTACK_DOWN_SIDE_EFFECT
    } else {
        move_hit_test(battle, user, rng);
        if battle.move_missed || battle.side(target).status1.contains(Status1::INVULNERABLE) {
            return missed(battle);
        }
        let which = move_effect.wrapping_sub(effect::ATTACK_DOWN1_EFFECT);
        if which >= effect::EVASION_DOWN1_EFFECT + 3 - effect::ATTACK_DOWN1_EFFECT {
            which.wrapping_sub(effect::ATTACK_DOWN2_EFFECT - effect::ATTACK_DOWN1_EFFECT)
        } else {
            which
        }
    } as usize;
    let side = battle.side_mut(target);
    let mut stage = side.stat_mods[which] - 1;
    if stage == 0 {
        return cant_lower();
    }
    // `ATTACK_DOWN2_EFFECT - $16`: every effect from `$24` that is not a side effect lowers twice.
    if move_effect >= effect::ATTACK_DOWN2_EFFECT - 0x16 && !side_effect {
        stage = (stage - 1).max(1);
    }
    side.stat_mods[which] = stage;
    if which < 4 {
        let index = stat::ATTACK + which;
        if side.mon.stats[index] == 1 {
            side.stat_mods[which] += 1;
            return cant_lower();
        }
        side.mon.stats[index] = modified(side.unmodified_stats[index], stage).max(1);
    }
    if user == Side::Enemy {
        apply_badge_stat_boosts(battle, badges);
    }
    apply_burn_and_paralysis_penalties(battle, target);
    vec![BattleText::MonsStatsFellText]
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::super::fixture::{each_case, side};
    use super::*;

    #[test]
    fn every_harvested_case_of_stat_modifier_up_effect() {
        each_case(include_str!("../../../../fixtures/battle/stat_modifier_up_effect.jsonl"), |arena, input, _| {
            json!(stat_modifier_up_effect(&mut arena.battle, side(input), arena.badges))
        });
    }

    #[test]
    fn every_harvested_case_of_stat_modifier_down_effect() {
        each_case(include_str!("../../../../fixtures/battle/stat_modifier_down_effect.jsonl"), |arena, input, rng| {
            json!(stat_modifier_down_effect(&mut arena.battle, side(input), arena.badges, rng))
        });
    }
}
