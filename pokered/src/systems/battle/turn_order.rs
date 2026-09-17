//! Who moves first: `MainInBattleLoop` from `.noLinkBattle` to `.playerMovesFirst` or
//! `.enemyMovesFirst`.

use poke_core::move_name::PokemonMoveName;
use crate::rng::Rng;
use super::{stat, Battle, Side};

/// `50 percent + 1`.
const HALF: u8 = 128;

/// Quick Attack goes first and Counter last, unless both sides chose the same one; then the
/// faster mon, and a tie is a random byte below 128 for the player.
pub fn first_to_move(battle: &Battle, rng: &mut impl Rng) -> Side {
    let (quick_attack, counter) = (PokemonMoveName::QuickAttack as u8, PokemonMoveName::Counter as u8);
    let (player, enemy) = (battle.player.selected_move, battle.enemy.selected_move);
    if player == quick_attack && enemy != quick_attack {
        return Side::Player;
    }
    if player != quick_attack && enemy == quick_attack {
        return Side::Enemy;
    }
    if player != quick_attack {
        if player == counter && enemy != counter {
            return Side::Enemy;
        }
        if player != counter && enemy == counter {
            return Side::Player;
        }
    }
    let (player_speed, enemy_speed) = (battle.player.mon.stats[stat::SPEED], battle.enemy.mon.stats[stat::SPEED]);
    match player_speed.cmp(&enemy_speed) {
        std::cmp::Ordering::Greater => Side::Player,
        std::cmp::Ordering::Less => Side::Enemy,
        std::cmp::Ordering::Equal if rng.random() < HALF => Side::Player,
        std::cmp::Ordering::Equal => Side::Enemy,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::fixture::each_case;
    use super::*;

    #[test]
    fn every_harvested_case_of_first_to_move() {
        each_case(include_str!("../../../fixtures/battle/first_to_move.jsonl"), |arena, _, rng| {
            json!(first_to_move(&arena.battle, rng))
        });
    }
}
