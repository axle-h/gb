//! The Safari Zone's arithmetic: `ItemUseBait`, `ItemUseRock`, `PrintSafariZoneBattleText`, and
//! whether the mon runs after the player's turn in `StartBattle`.

use serde::{Deserialize, Serialize};
use crate::rng::Rng;
use super::Battle;

/// What `PrintSafariZoneBattleText` says after a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafariText {
    /// `SafariZoneEatingText`.
    Eating,
    /// `SafariZoneAngryText`.
    Angry,
}

/// `BaitRockCommon`'s 1 to 5, redrawn until the low three bits are under 5, added up to `$FF`.
fn raise(factor: &mut u8, rng: &mut impl Rng) {
    let step = loop {
        let drawn = rng.random() & 7;
        if drawn < 5 {
            break drawn + 1;
        }
    };
    *factor = factor.saturating_add(step);
}

/// `ItemUseBait`: the catch rate halved, the escape factor cleared, the bait factor raised.
pub fn throw_bait(battle: &mut Battle, rng: &mut impl Rng) {
    battle.enemy_exp.catch_rate >>= 1;
    battle.safari_escape_factor = 0;
    raise(&mut battle.safari_bait_factor, rng);
}

/// `ItemUseRock`: the catch rate doubled up to `$FF`, the bait factor cleared, the escape factor
/// raised.
pub fn throw_rock(battle: &mut Battle, rng: &mut impl Rng) {
    battle.enemy_exp.catch_rate = battle.enemy_exp.catch_rate.saturating_mul(2);
    battle.safari_bait_factor = 0;
    raise(&mut battle.safari_escape_factor, rng);
}

/// `PrintSafariZoneBattleText`: a turn of bait eaten, else a turn of anger spent, and the base catch
/// rate back once the anger is gone. Nothing is said with neither.
pub fn safari_zone_battle_text(battle: &mut Battle) -> Option<SafariText> {
    if battle.safari_bait_factor != 0 {
        battle.safari_bait_factor -= 1;
        return Some(SafariText::Eating);
    }
    if battle.safari_escape_factor == 0 {
        return None;
    }
    battle.safari_escape_factor -= 1;
    if battle.safari_escape_factor == 0 {
        battle.enemy_exp.catch_rate = poke_core::base_stats::BaseStats::of(battle.enemy.mon.species).catch_rate;
    }
    Some(SafariText::Angry)
}

/// `StartBattle`'s safari turn: twice the low byte of the enemy's speed, a quarter of it while it is
/// eating, twice again while it is angry; the mon runs when that overflows or beats a random byte.
pub fn safari_mon_runs(battle: &Battle, rng: &mut impl Rng) -> bool {
    let low = battle.enemy.mon.stats[3] as u8;
    let (mut b, carry) = low.overflowing_add(low);
    if carry {
        return true;
    }
    if battle.safari_bait_factor != 0 {
        b >>= 2;
    }
    if battle.safari_escape_factor != 0 {
        b = b.checked_mul(2).unwrap_or(0xFF);
    }
    rng.random() < b
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::fixture::each_case;
    use super::*;

    #[test]
    fn every_harvested_case_of_throw_bait() {
        each_case(include_str!("../../../fixtures/battle/throw_bait.jsonl"), |arena, _, rng| {
            throw_bait(&mut arena.battle, rng);
            serde_json::Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_throw_rock() {
        each_case(include_str!("../../../fixtures/battle/throw_rock.jsonl"), |arena, _, rng| {
            throw_rock(&mut arena.battle, rng);
            serde_json::Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_safari_zone_battle_text() {
        each_case(include_str!("../../../fixtures/battle/safari_zone_battle_text.jsonl"), |arena, _, _| {
            json!(safari_zone_battle_text(&mut arena.battle).map(|text| match text {
                SafariText::Eating => "SafariZoneEatingText",
                SafariText::Angry => "SafariZoneAngryText",
            }))
        });
    }

    #[test]
    fn every_harvested_case_of_safari_mon_runs() {
        each_case(include_str!("../../../fixtures/battle/safari_mon_runs.jsonl"), |arena, _, rng| {
            json!(safari_mon_runs(&arena.battle, rng))
        });
    }
}
