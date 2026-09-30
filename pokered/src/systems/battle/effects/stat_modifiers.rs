//! `StatModifierUpEffect` and `StatModifierDownEffect`.

use poke_core::battle_data::stat_modifier_ratios;
use poke_core::move_name::PokemonMoveName;
use crate::rng::Rng;
use crate::systems::math::{divide, multiply};
use crate::systems::stats::MAX_STAT_VALUE;
use super::super::accuracy::move_hit_test;
use super::super::modified_stats::{apply_badge_stat_boosts, apply_burn_and_paralysis_penalties, apply_penalty_and_badge_boost_to};
use super::super::{effect, stat, Battle, Side, Status1, Status2, MAX_STAT_LEVEL};
use super::BattleText;

/// `25 percent + 1`: below it, the cartridge's enemy stat-lowering move misses outright.
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
/// nothing at +6 already or at a stat of exactly 999. The stat is worked out afresh from the
/// unmodified one, then given back its own penalty and badge boost.
pub fn stat_modifier_up_effect(battle: &mut Battle, user: Side, badges: u8) -> Vec<BattleText> {
    let cartridge_bugs = battle.cartridge_bugs;
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
    let old_stage = side.stat_mods[which];
    side.stat_mods[which] = stage;
    if which < 4 {
        let index = stat::ATTACK + which;
        if side.mon.stats[index] == MAX_STAT_VALUE {
            // The cartridge takes back one stage of a +2.
            side.stat_mods[which] = if cartridge_bugs { stage - 1 } else { old_stage };
            return vec![BattleText::NothingHappenedText];
        }
        side.mon.stats[index] = modified(side.unmodified_stats[index], stage).min(MAX_STAT_VALUE);
    }
    if side.current_move.animation == PokemonMoveName::Minimize as u8 {
        side.minimized = 1;
    }
    reapply_penalties_and_boosts(battle, user, user, which, badges);
    vec![BattleText::MonsStatsRoseText]
}

/// `StatModifierDownEffect`, on the user's target. A substitute blocks it, and a side effect lands
/// a third of the time without a hit test while the move itself takes one. One stage, or two for
/// the `_DOWN2` effects, not below -6, and nothing for a stat of exactly 1. The stat is worked out
/// afresh, never below 1, then given back its own penalty and badge boost. A side effect that fails
/// says nothing.
pub fn stat_modifier_down_effect(battle: &mut Battle, user: Side, badges: u8, rng: &mut impl Rng) -> Vec<BattleText> {
    let cartridge_bugs = battle.cartridge_bugs;
    let move_effect = battle.side(user).current_move.effect;
    let side_effect = move_effect >= effect::ATTACK_DOWN_SIDE_EFFECT;
    let missed = |battle: &Battle| match side_effect || battle.move_didnt_miss {
        true => vec![],
        false => vec![BattleText::ButItFailedText],
    };
    let cant_lower = || if side_effect { vec![] } else { vec![BattleText::NothingHappenedText] };
    let target = user.other();
    // The cartridge makes the enemy's copy miss a quarter of the time before any other check.
    if cartridge_bugs && user == Side::Enemy && rng.random() < ENEMY_STAT_DOWN_MISS {
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
    let old_stage = side.stat_mods[which];
    side.stat_mods[which] = stage;
    if which < 4 {
        let index = stat::ATTACK + which;
        if side.mon.stats[index] == 1 {
            // The cartridge gives back one stage of a -2.
            side.stat_mods[which] = if cartridge_bugs { stage + 1 } else { old_stage };
            return cant_lower();
        }
        side.mon.stats[index] = modified(side.unmodified_stats[index], stage).max(1);
    }
    reapply_penalties_and_boosts(battle, user, target, which, badges);
    vec![BattleText::MonsStatsFellText]
}

/// After `user`'s move changed `whose` stat `which`. The cartridge applies the player's badge
/// boosts to every stat again, compounding them, whenever the player's stat moved, and the
/// paralysis and burn penalties again to the mon not moving, while the stat worked out afresh keeps
/// neither unless one of those lands on it.
fn reapply_penalties_and_boosts(battle: &mut Battle, user: Side, whose: Side, which: usize, badges: u8) {
    if battle.cartridge_bugs {
        if whose == Side::Player {
            apply_badge_stat_boosts(battle, badges);
        }
        apply_burn_and_paralysis_penalties(battle, user.other());
    } else if which < 4 {
        apply_penalty_and_badge_boost_to(battle, whose, which, badges);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use crate::rng::GameRng;
    use super::super::super::fixture::{each_case, side};
    use super::super::super::{status, Arena, BASE_STAT_LEVEL};
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

    /// The baseline battle playing the fixes, `user` about to use a move with `move_effect`.
    fn playing(arena: &mut Arena, user: Side, move_effect: u8) -> &mut Battle {
        arena.battle.cartridge_bugs = false;
        let current_move = &mut arena.battle.side_mut(user).current_move;
        current_move.effect = move_effect;
        current_move.accuracy = 255;
        &mut arena.battle
    }

    /// An enemy stat-down move that does not miss its hit test.
    fn enemy_hits() -> GameRng {
        GameRng::tape(vec![0])
    }

    #[test]
    fn an_enemy_stat_down_move_misses_no_more_often_than_the_players() {
        let lowered = |cartridge_bugs: bool| {
            let mut arena = Arena::baseline();
            let battle = playing(&mut arena, Side::Enemy, effect::ATTACK_DOWN1_EFFECT);
            battle.cartridge_bugs = cartridge_bugs;
            let texts = stat_modifier_down_effect(battle, Side::Enemy, 0, &mut GameRng::tape(vec![0, 0]));
            (texts, arena.battle.player.stat_mods[0])
        };
        assert_eq!(lowered(false), (vec![BattleText::MonsStatsFellText], BASE_STAT_LEVEL - 1));
        assert_eq!(lowered(true), (vec![BattleText::ButItFailedText], BASE_STAT_LEVEL));
    }

    #[test]
    fn a_badge_boost_is_applied_once_however_many_stat_moves_follow() {
        let mut arena = Arena::baseline();
        let badges = 0xFF;
        apply_badge_stat_boosts(&mut arena.battle, badges);
        let sent_out = arena.battle.player.mon.stats;
        let unmodified = arena.battle.player.unmodified_stats;
        for _ in 0..2 {
            stat_modifier_up_effect(playing(&mut arena, Side::Player, effect::DEFENSE_UP1_EFFECT), Side::Player, badges);
        }
        stat_modifier_down_effect(playing(&mut arena, Side::Enemy, effect::ATTACK_DOWN1_EFFECT), Side::Enemy, badges, &mut enemy_hits());
        let stats = arena.battle.player.mon.stats;
        let boosted = |value: u16| value + (value >> 3);
        assert_eq!(stats[stat::DEFENSE], boosted(modified(unmodified[stat::DEFENSE], BASE_STAT_LEVEL + 2)));
        assert_eq!(stats[stat::ATTACK], boosted(modified(unmodified[stat::ATTACK], BASE_STAT_LEVEL - 1)));
        assert_eq!((stats[stat::SPEED], stats[stat::SPECIAL]), (sent_out[stat::SPEED], sent_out[stat::SPECIAL]));
    }

    #[test]
    fn a_stat_move_leaves_the_other_mons_penalties_alone_and_keeps_its_own() {
        let mut arena = Arena::baseline();
        arena.battle.enemy.mon.status = status::PAR | status::BRN;
        apply_burn_and_paralysis_penalties(&mut arena.battle, Side::Enemy);
        let inflicted = arena.battle.enemy.mon.stats;
        stat_modifier_up_effect(playing(&mut arena, Side::Player, effect::SPEED_UP1_EFFECT), Side::Player, 0);
        assert_eq!(arena.battle.enemy.mon.stats, inflicted);

        stat_modifier_up_effect(playing(&mut arena, Side::Enemy, effect::SPEED_UP2_EFFECT), Side::Enemy, 0);
        let unmodified = arena.battle.enemy.unmodified_stats;
        assert_eq!(arena.battle.enemy.mon.stats[stat::SPEED], modified(unmodified[stat::SPEED], BASE_STAT_LEVEL + 2) >> 2);
        assert_eq!(arena.battle.enemy.mon.stats[stat::ATTACK], inflicted[stat::ATTACK]);
    }

    #[test]
    fn a_two_stage_move_that_does_nothing_leaves_the_stage_as_it_was() {
        let mut arena = Arena::baseline();
        arena.battle.player.mon.stats[stat::ATTACK] = MAX_STAT_VALUE;
        let texts = stat_modifier_up_effect(playing(&mut arena, Side::Player, effect::ATTACK_UP2_EFFECT), Side::Player, 0);
        assert_eq!((texts, arena.battle.player.stat_mods[0]), (vec![BattleText::NothingHappenedText], BASE_STAT_LEVEL));

        arena.battle.player.mon.stats[stat::DEFENSE] = 1;
        let battle = playing(&mut arena, Side::Enemy, effect::DEFENSE_DOWN2_EFFECT);
        let texts = stat_modifier_down_effect(battle, Side::Enemy, 0, &mut enemy_hits());
        assert_eq!((texts, arena.battle.player.stat_mods[1]), (vec![BattleText::NothingHappenedText], BASE_STAT_LEVEL));
    }
}
