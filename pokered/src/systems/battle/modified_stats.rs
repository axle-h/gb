//! `CalculateModifiedStat(s)`, `ApplyBurnAndParalysisPenalties` and `ApplyBadgeStatBoosts`: the
//! stats a mon fights with, worked out again from the unmodified ones whenever a modifier moves.

use poke_core::battle_data::stat_modifier_ratios;
use crate::systems::math::{divide, multiply};
use crate::systems::stats::MAX_STAT_VALUE;
use super::{stat, status, Battle, Side};

/// `CalculateModifiedStat` for `wCalculateWhoseStats` and `c`, 0 to 3 for attack to special: the
/// unmodified stat times its modifier's ratio, capped at 999 and never 0.
pub fn calculate_modified_stat(battle: &mut Battle, whose: Side, which: usize) {
    let side = battle.side_mut(whose);
    let (numerator, denominator) = stat_modifier_ratios()[side.stat_mods[which] as usize - 1];
    let product = multiply(side.unmodified_stats[stat::ATTACK + which] as u32, numerator);
    let (quotient, _) = divide(product.to_be_bytes(), denominator, 4);
    let value = u16::from_be_bytes([quotient[2], quotient[3]]).min(MAX_STAT_VALUE);
    side.mon.stats[stat::ATTACK + which] = value.max(1);
}

/// `CalculateModifiedStats`: attack, defense, speed and special.
pub fn calculate_modified_stats(battle: &mut Battle, whose: Side) {
    for which in 0..4 {
        calculate_modified_stat(battle, whose, which);
    }
}

/// `ApplyBurnAndParalysisPenaltiesToPlayer` and `...ToEnemy`: a paralysed mon's speed quartered and
/// a burned one's attack halved, each never below 1. The cartridge sets `hWhoseTurn` to the other
/// side on the way, which a caller that reads it afterwards inherits.
pub fn apply_burn_and_paralysis_penalties(battle: &mut Battle, to: Side) {
    let side = battle.side_mut(to);
    if side.mon.status & status::PAR != 0 {
        side.mon.stats[stat::SPEED] = (side.mon.stats[stat::SPEED] >> 2).max(1);
    }
    if side.mon.status & status::BRN != 0 {
        side.mon.stats[stat::ATTACK] = (side.mon.stats[stat::ATTACK] >> 1).max(1);
    }
}

/// `ApplyBadgeStatBoosts`: an eighth more attack for the Boulder Badge, defense for the Thunder
/// Badge, speed for the Soul Badge and special for the Volcano Badge, capped at 999, to the
/// player's mon. The boosts compound each time it runs.
pub fn apply_badge_stat_boosts(battle: &mut Battle, badges: u8) {
    for which in 0..4 {
        if badges & 1 << (which * 2) != 0 {
            let value = &mut battle.player.mon.stats[stat::ATTACK + which];
            *value = value.wrapping_add(*value >> 3).min(MAX_STAT_VALUE);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use super::super::fixture::{each_case, side};
    use super::*;

    #[test]
    fn every_harvested_case_of_calculate_modified_stat() {
        each_case(include_str!("../../../fixtures/battle/calculate_modified_stat.jsonl"), |arena, input, _| {
            calculate_modified_stat(&mut arena.battle, side(input), input["which"].as_u64().unwrap() as usize);
            Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_apply_burn_and_paralysis_penalties() {
        each_case(include_str!("../../../fixtures/battle/apply_burn_and_paralysis_penalties.jsonl"), |arena, input, _| {
            apply_burn_and_paralysis_penalties(&mut arena.battle, side(input));
            Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_apply_badge_stat_boosts() {
        each_case(include_str!("../../../fixtures/battle/apply_badge_stat_boosts.jsonl"), |arena, _, _| {
            apply_badge_stat_boosts(&mut arena.battle, arena.badges);
            Value::Null
        });
    }
}
