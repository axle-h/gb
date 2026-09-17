//! `engine/battle/experience.asm`, and the Exp. All half of `FaintEnemyPokemon`: what a fainted
//! enemy gives each party mon, and the levels it brings.

use poke_core::base_stats::BaseStats;
use serde::{Deserialize, Serialize};
use crate::party::PartyMon;
use crate::systems::experience::{calc_experience, calc_level_from_experience};
use crate::systems::math::{divide, multiply};
use crate::systems::stats::calc_stats;
use super::modified_stats::{apply_badge_stat_boosts, apply_burn_and_paralysis_penalties, calculate_modified_stats};
use super::{Battle, BattleKind, Side, Status3};

pub const MAX_LEVEL: u8 = 100;
const EXPERIENCE_MASK: u32 = 0xFF_FFFF;

/// What `GainExperience` prints, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpEvent {
    /// `GainedText`: `wExpAmountGained`, boosted where the mon was traded.
    Gained { slot: u8, amount: u16, boosted: bool },
    /// `GrewLevelText`, after which the stats box is shown and `LearnMoveFromLevelUp` asks about
    /// the one move at exactly `level`.
    GrewLevel { slot: u8, level: u8 },
}

/// `GainExperience`. Every mon with HP and its bit in `wPartyGainExpFlags` gets the enemy's base
/// stats as stat experience, saturating, and `base exp * level / 7`, half again for a traded mon
/// and half again in a trainer battle, capped at level 100's experience. A level gained works out
/// the stats with stat experience and adds what the max HP gained to the HP; for the mon in
/// battle the battle mon is refreshed and its modifiers, penalties and badge boosts reapplied, so
/// the boosts compound. The enemy's worth is first divided by the number of mons with their bit set,
/// in place, which is what a second call for Exp. All then divides again.
pub fn gain_experience(battle: &mut Battle, party: &mut [PartyMon], player_id: u16, badges: u8) -> Vec<ExpEvent> {
    divide_exp_data_by_num_mons_gaining_exp(battle);
    let mut events = vec![];
    for slot in 0..party.len() {
        if party[slot].mon.hp == 0 || battle.gain_exp_flags & 1 << slot == 0 {
            continue;
        }
        let mon = &mut party[slot];
        for (stat_exp, &base) in mon.mon.stat_exp.iter_mut().zip(&battle.enemy_exp.base_stats) {
            *stat_exp = stat_exp.saturating_add(base as u16);
        }
        let product = multiply(battle.enemy_exp.base_exp as u32, battle.enemy.mon.level);
        let (quotient, _) = divide(product.to_be_bytes(), 7, 4);
        let mut gained = u16::from_be_bytes([quotient[2], quotient[3]]);
        let boosted = mon.mon.ot_id != player_id;
        if boosted {
            gained = boost_exp(gained);
        }
        if battle.kind != BattleKind::Wild {
            gained = boost_exp(gained);
        }
        mon.mon.exp = (mon.mon.exp + gained as u32) & EXPERIENCE_MASK;
        let growth_rate = BaseStats::of(mon.mon.species).growth_rate;
        mon.mon.exp = mon.mon.exp.min(calc_experience(growth_rate, MAX_LEVEL));
        events.push(ExpEvent::Gained { slot: slot as u8, amount: gained, boosted });

        let level = calc_level_from_experience(growth_rate, mon.mon.exp);
        if level == mon.level {
            continue;
        }
        mon.level = level;
        let old_max_hp = mon.stats[0];
        mon.stats = calc_stats(BaseStats::of(mon.mon.species).stats, mon.mon.dvs, Some(mon.mon.stat_exp), level);
        mon.mon.hp = mon.mon.hp.wrapping_add(mon.stats[0].wrapping_sub(old_max_hp));
        if slot == battle.player_mon_number as usize {
            let player = &mut battle.player;
            player.mon.hp = mon.mon.hp;
            player.mon.level = level;
            player.mon.stats = mon.stats;
            if !player.status3.contains(Status3::TRANSFORMED) {
                player.unmodified_level = level;
                player.unmodified_stats = mon.stats;
            }
            calculate_modified_stats(battle, Side::Player);
            apply_burn_and_paralysis_penalties(battle, Side::Player);
            apply_badge_stat_boosts(battle, badges);
        }
        events.push(ExpEvent::GrewLevel { slot: slot as u8, level });
        battle.can_evolve_flags |= 1 << slot;
    }
    battle.gain_exp_flags = 1 << battle.player_mon_number;
    battle.fought_current_enemy_flags = 1 << battle.player_mon_number;
    events
}

/// `BoostExp`: half again, in the sixteen bits `wExpAmountGained` keeps.
fn boost_exp(exp: u16) -> u16 {
    exp.wrapping_add(exp >> 1)
}

/// `DivideExpDataByNumMonsGainingExp`: every flag counts, all eight bits of it.
fn divide_exp_data_by_num_mons_gaining_exp(battle: &mut Battle) {
    let count = battle.gain_exp_flags.count_ones() as u8;
    if count < 2 {
        return;
    }
    let exp = &mut battle.enemy_exp;
    for value in exp.base_stats.iter_mut().chain([&mut exp.catch_rate, &mut exp.base_exp]) {
        *value = divide([0, *value, 0, 0], count, 2).0[3];
    }
}

/// `FaintEnemyPokemon` from `.playermonnotfaint`: nothing if the whole party has fainted; then the
/// mons that fought share the enemy's worth, halved first if the bag holds an Exp. All, and with
/// one every party mon then shares what the first division left.
pub fn faint_enemy_pokemon_experience(battle: &mut Battle, party: &mut [PartyMon], player_id: u16, badges: u8,
                                      has_exp_all: bool) -> Vec<ExpEvent> {
    if party.iter().all(|mon| mon.mon.hp == 0) {
        return vec![];
    }
    if has_exp_all {
        let exp = &mut battle.enemy_exp;
        for value in exp.base_stats.iter_mut().chain([&mut exp.catch_rate, &mut exp.base_exp]) {
            *value >>= 1;
        }
    }
    let mut events = gain_experience(battle, party, player_id, badges);
    if has_exp_all {
        battle.gain_exp_flags = ((1u16 << party.len()) - 1) as u8;
        events.extend(gain_experience(battle, party, player_id, badges));
    }
    events
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::fixture::each_case;
    use super::*;

    #[test]
    fn every_harvested_case_of_gain_experience() {
        each_case(include_str!("../../../fixtures/battle/gain_experience.jsonl"), |arena, _, _| {
            json!(gain_experience(&mut arena.battle, &mut arena.party, arena.player_id, arena.badges))
        });
    }

    #[test]
    fn every_harvested_case_of_faint_enemy_pokemon_experience() {
        each_case(include_str!("../../../fixtures/battle/faint_enemy_pokemon_experience.jsonl"), |arena, input, _| {
            let has_exp_all = input["has_exp_all"].as_bool().unwrap();
            json!(faint_enemy_pokemon_experience(&mut arena.battle, &mut arena.party, arena.player_id, arena.badges, has_exp_all))
        });
    }
}
