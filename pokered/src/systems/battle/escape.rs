//! `TryRunningFromBattle`.

use serde::{Deserialize, Serialize};
use crate::rng::Rng;
use crate::systems::math::{divide, multiply};
use super::effects::BattleText;
use super::{stat, Battle, BattleKind};

/// `TryRunningFromBattle`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Carry: `wBattleResult` is 2, "got away safely".
    pub escaped: bool,
    /// `wActionResultOrTookBattleTurn`: a failed run in a wild battle costs the turn.
    pub took_turn: bool,
    pub texts: Vec<BattleText>,
}

/// `TryRunningFromBattle`: a ghost battle and the Safari Zone always let the player go, and a
/// trainer never does. In the wild it counts the attempt; a mon at least as fast gets away, and
/// otherwise the low sixteen bits of 32 times its speed over the enemy's quarter speed in a byte,
/// which gets away outright at 0, at 256 or more, or by carrying past 255 with 30 added per earlier
/// attempt, and else on a random byte no higher. `speed` is the battle mon's from the battle menu,
/// and the *first* party mon's from the prompt after a faint.
pub fn try_running_from_battle(battle: &Battle, speed: u16, ghost_or_safari: bool, num_run_attempts: &mut u8,
                               rng: &mut impl Rng) -> Run {
    let got_away = Run { escaped: true, took_turn: false, texts: vec![BattleText::GotAwayText] };
    if ghost_or_safari {
        return got_away;
    }
    if battle.kind != BattleKind::Wild {
        return Run { escaped: false, took_turn: false, texts: vec![BattleText::NoRunningText] };
    }
    *num_run_attempts = num_run_attempts.wrapping_add(1);
    let enemy_speed = battle.enemy.mon.stats[stat::SPEED];
    if speed >= enemy_speed {
        return got_away;
    }
    let product = multiply(speed as u32, 32).to_be_bytes();
    let divisor = (enemy_speed >> 2) as u8;
    if divisor == 0 {
        return got_away;
    }
    let (quotient, _) = divide([product[2], product[3], 0, 0], divisor, 2);
    if quotient[2] != 0 {
        return got_away;
    }
    let mut odds = quotient[3];
    for _ in 1..if *num_run_attempts == 0 { 256 } else { *num_run_attempts as u32 } {
        match odds.checked_add(30) {
            Some(sum) => odds = sum,
            None => return got_away,
        }
    }
    if odds >= rng.random() {
        return got_away;
    }
    Run { escaped: false, took_turn: true, texts: vec![BattleText::CantEscapeText] }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::fixture::each_case;
    use super::*;

    #[test]
    fn every_harvested_case_of_try_running_from_battle() {
        each_case(include_str!("../../../fixtures/battle/try_running_from_battle.jsonl"), |arena, input, rng| {
            let speed = if input["from_menu"].as_bool().unwrap() { arena.battle.player.mon.stats[stat::SPEED] }
                else { arena.party[0].stats[stat::SPEED] };
            let mut attempts = input["attempts"].as_u64().unwrap() as u8;
            let run = try_running_from_battle(&arena.battle, speed, input["safari"].as_bool().unwrap(), &mut attempts, rng);
            json!({"escaped": run.escaped, "took_turn": run.took_turn, "texts": run.texts, "attempts": attempts})
        });
    }
}
