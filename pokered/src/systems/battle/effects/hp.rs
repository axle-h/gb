//! The effects that move HP: draining, recoil, Substitute, the healing moves and Explosion.

use poke_core::move_name::PokemonMoveName;
use crate::party::PartyMon;
use super::super::modified_stats::{apply_penalty_and_badge_boost_to, calculate_modified_stat};
use super::super::{effect, stat, status, Battle, Side, Status2, Status3};
use super::{read_player_mon_cur_hp_and_status, BattleText};

/// `DrainHPEffect_`: `wDamage` halved in place, never below 1, onto the user's HP up to its max,
/// and the player's HP and status copied back to its party slot either way.
pub fn drain_hp_effect(battle: &mut Battle, party: &mut [PartyMon], user: Side) -> Vec<BattleText> {
    battle.damage = (battle.damage >> 1).max(1);
    let damage = battle.damage;
    let mon = &mut battle.side_mut(user).mon;
    mon.hp = match mon.hp.checked_add(damage) {
        Some(hp) if hp <= mon.stats[0] => hp,
        _ => mon.stats[0],
    };
    read_player_mon_cur_hp_and_status(battle, party);
    if battle.side(user).current_move.effect == effect::DREAM_EATER_EFFECT {
        vec![BattleText::DreamWasEatenText]
    } else {
        vec![BattleText::SuckedHealthText]
    }
}

/// `RecoilEffect_`: a quarter of `wDamage`, a half for Struggle by move number, at least 1, off the
/// user's HP and never below 0.
pub fn recoil_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let damage = battle.damage;
    let side = battle.side_mut(user);
    let recoil = if side.current_move.animation == PokemonMoveName::Struggle as u8 { damage >> 1 } else { damage >> 2 };
    side.mon.hp = side.mon.hp.saturating_sub(recoil.max(1));
    vec![BattleText::HitWithRecoilText]
}

/// `SubstituteEffect_`: a quarter of the max HP in one byte, which assumes a max HP below 1024,
/// becomes the substitute's HP whether or not the user can pay it, and a user with no more HP than
/// that is too weak.
pub fn substitute_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let cartridge_bugs = battle.cartridge_bugs;
    let side = battle.side_mut(user);
    if side.status2.contains(Status2::HAS_SUBSTITUTE_UP) {
        return vec![BattleText::HasSubstituteText];
    }
    let cost = (side.mon.stats[0] >> 2) as u8;
    side.substitute_hp = cost;
    match side.mon.hp.checked_sub(cost as u16) {
        // The cartridge only fails on a borrow, so paying exactly the HP left leaves 0.
        Some(hp) if hp != 0 || cartridge_bugs => {
            side.mon.hp = hp;
            side.status2 |= Status2::HAS_SUBSTITUTE_UP;
            vec![BattleText::SubstituteText]
        }
        _ => vec![BattleText::TooWeakSubstituteText],
    }
}

/// `HealEffect_`. A mon at full HP fails. Rest puts the user to sleep for 2 turns whatever its
/// status, and heals the whole max HP; Recover and Softboiled heal half, in sixteen bits, up to the
/// max.
pub fn heal_effect(battle: &mut Battle, user: Side, badges: u8) -> Vec<BattleText> {
    let cartridge_bugs = battle.cartridge_bugs;
    let side = battle.side_mut(user);
    // The cartridge tests the low byte less the high bytes' borrow, so 255 or 511 below max is full.
    let full = if cartridge_bugs {
        let [hp_high, hp_low] = side.mon.hp.to_be_bytes();
        let [max_high, max_low] = side.mon.stats[0].to_be_bytes();
        hp_low.wrapping_sub(max_low).wrapping_sub((hp_high < max_high) as u8) == 0
    } else {
        side.mon.hp == side.mon.stats[0]
    };
    if full {
        return vec![BattleText::ButItFailedText];
    }
    let mut texts = vec![];
    let rest = side.current_move.animation == PokemonMoveName::Rest as u8;
    if rest {
        texts.push(if side.mon.status == 0 { BattleText::StartedSleepingEffect } else { BattleText::FellAsleepBecameHealthyText });
        let replaced = side.mon.status;
        side.mon.status = 2;
        // The cartridge keeps the replaced status's stat penalty and Toxic's flag.
        if !cartridge_bugs {
            side.status3.remove(Status3::BADLY_POISONED);
            for (penalty, index) in [(status::PAR, stat::SPEED), (status::BRN, stat::ATTACK)] {
                if replaced & penalty != 0 {
                    calculate_modified_stat(battle, user, index - stat::ATTACK);
                    apply_penalty_and_badge_boost_to(battle, user, index - stat::ATTACK, badges);
                }
            }
        }
    }
    let side = battle.side_mut(user);
    let amount = if rest { side.mon.stats[0] } else { side.mon.stats[0] >> 1 };
    side.mon.hp = side.mon.hp.wrapping_add(amount);
    if side.mon.hp >= side.mon.stats[0] {
        side.mon.hp = side.mon.stats[0];
    }
    texts.push(BattleText::RegainedHealthText);
    texts
}

/// `ExplodeEffect`: the user faints, its status and Leech Seed gone with it.
pub fn explode_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    side.mon.hp = 0;
    side.mon.status = 0;
    side.status2.remove(Status2::SEEDED);
    vec![]
}

#[cfg(test)]
mod tests {
    use crate::systems::battle::stat_mod;
    use super::super::tests::using;
    use super::*;

    #[test]
    fn a_substitute_that_would_leave_0_hp_fails() {
        let mut arena = using(Side::Player, PokemonMoveName::Substitute);
        let cost = arena.battle.player.mon.stats[stat::MAX_HP] >> 2;
        arena.battle.player.mon.hp = cost;
        assert_eq!(substitute_effect(&mut arena.battle, Side::Player), vec![BattleText::TooWeakSubstituteText]);
        assert_eq!(arena.battle.player.mon.hp, cost);
        assert!(!arena.battle.player.status2.contains(Status2::HAS_SUBSTITUTE_UP));
    }

    #[test]
    fn recover_heals_255_or_511_below_max_and_fails_only_at_max() {
        for below in [255, 511, 0] {
            let mut arena = using(Side::Player, PokemonMoveName::Recover);
            let mon = &mut arena.battle.player.mon;
            mon.stats[stat::MAX_HP] = 600;
            mon.hp = 600 - below;
            let texts = heal_effect(&mut arena.battle, Side::Player, 0);
            let expected = if below == 0 { BattleText::ButItFailedText } else { BattleText::RegainedHealthText };
            assert_eq!(texts, vec![expected], "{below} below max");
            assert_eq!(arena.battle.player.mon.hp, 600 - below.saturating_sub(300), "{below} below max");
        }
    }

    #[test]
    fn rest_lifts_the_replaced_status_penalty_and_toxic() {
        let after_rest = |cartridge_bugs, user: Side, status_byte| {
            let mut arena = using(user, PokemonMoveName::Rest);
            arena.battle.cartridge_bugs = cartridge_bugs;
            let side = arena.battle.side_mut(user);
            side.stat_mods[stat_mod::SPEED] = 9;
            let unmodified = side.unmodified_stats;
            side.mon.stats[stat::SPEED] = unmodified[stat::SPEED] * 2 / 4;
            side.mon.stats[stat::ATTACK] = unmodified[stat::ATTACK] / 2;
            side.mon.status = status_byte;
            side.status3 |= Status3::BADLY_POISONED;
            side.mon.hp = 1;
            // The Boulder and Soul badges.
            heal_effect(&mut arena.battle, user, 1 | 1 << 4);
            let side = arena.battle.side(user);
            assert_eq!(side.mon.status, 2);
            (unmodified, side.mon.stats, side.status3.contains(Status3::BADLY_POISONED))
        };
        let (unmodified, stats, toxic) = after_rest(false, Side::Player, status::PAR);
        let doubled = unmodified[stat::SPEED] * 2;
        assert_eq!(stats[stat::SPEED], doubled + doubled / 8, "+2 and the Soul Badge's eighth, unquartered");
        assert!(!toxic);
        let (unmodified, stats, _) = after_rest(false, Side::Player, status::BRN);
        assert_eq!(stats[stat::ATTACK], unmodified[stat::ATTACK] + unmodified[stat::ATTACK] / 8);
        let (unmodified, stats, toxic) = after_rest(false, Side::Enemy, status::PAR);
        assert_eq!(stats[stat::SPEED], unmodified[stat::SPEED] * 2, "no badge boost for the enemy");
        assert!(!toxic);
        let (unmodified, stats, toxic) = after_rest(true, Side::Player, status::PAR);
        assert_eq!(stats[stat::SPEED], unmodified[stat::SPEED] * 2 / 4, "the cartridge keeps the penalty");
        assert!(toxic, "and Toxic's flag");
    }
}
