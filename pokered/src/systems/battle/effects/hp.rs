//! The effects that move HP: draining, recoil, Substitute, the healing moves and Explosion.

use poke_core::move_name::PokemonMoveName;
use crate::party::PartyMon;
use super::super::{effect, Battle, Side, Status2};
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
/// becomes the substitute's HP whether or not the user can pay it. Paying it may leave 0 HP.
pub fn substitute_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    if side.status2.contains(Status2::HAS_SUBSTITUTE_UP) {
        return vec![BattleText::HasSubstituteText];
    }
    let cost = (side.mon.stats[0] >> 2) as u8;
    side.substitute_hp = cost;
    match side.mon.hp.checked_sub(cost as u16) {
        Some(hp) => {
            side.mon.hp = hp;
            side.status2 |= Status2::HAS_SUBSTITUTE_UP;
            vec![BattleText::SubstituteText]
        }
        None => vec![BattleText::TooWeakSubstituteText],
    }
}

/// `HealEffect_`. Full HP is tested by the low byte less the borrow of the high bytes, so a mon
/// 255 or 511 below its max counts as full and fails. Rest puts the user to sleep for 2 turns
/// whatever its status, and heals the whole max HP; Recover and Softboiled heal half, in sixteen
/// bits, up to the max.
pub fn heal_effect(battle: &mut Battle, user: Side) -> Vec<BattleText> {
    let side = battle.side_mut(user);
    let [hp_high, hp_low] = side.mon.hp.to_be_bytes();
    let [max_high, max_low] = side.mon.stats[0].to_be_bytes();
    let borrow = hp_high < max_high;
    if hp_low.wrapping_sub(max_low).wrapping_sub(borrow as u8) == 0 {
        return vec![BattleText::ButItFailedText];
    }
    let mut texts = vec![];
    let rest = side.current_move.animation == PokemonMoveName::Rest as u8;
    if rest {
        texts.push(if side.mon.status == 0 { BattleText::StartedSleepingEffect } else { BattleText::FellAsleepBecameHealthyText });
        side.mon.status = 2;
    }
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
