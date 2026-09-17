//! What decides whether a mon moves this turn and what the turn costs it: `CheckPlayerStatusConditions`
//! and its enemy twin, `HandleSelfConfusionDamage`, `CheckForDisobedience`,
//! `HandlePoisonBurnLeechSeed`, `CheckNumAttacksLeft`, `DecrementPP`, `IncrementMovePP`,
//! `MirrorMoveCopyMove` and `MetronomePickMove`.

use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use serde::{Deserialize, Serialize};
use crate::party::PartyMon;
use crate::rng::Rng;
use super::apply::apply_damage_to_pokemon;
use super::damage::{calculate_damage, get_damage_vars};
use super::effects::BattleText;
use super::{stat, status, Battle, CriticalHitOrOhko, Side, Status1, Status2, Status3, PP_MASK};

/// Where `CheckPlayerStatusConditions` sends the turn: the `hl` it returns with `z`, or on with `nz`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Continuation {
    /// `nz`: the mon moves as chosen.
    Move,
    /// `ExecutePlayerMoveDone`: no move this turn.
    MoveDone,
    /// `HandleIfPlayerMoveMissed`: Bide unleashes its stored damage.
    HandleIfMoveMissed,
    /// `PlayerCalcMoveDamage`: Thrash goes on, with no PP spent.
    CalcMoveDamage,
    /// `GetPlayerAnimationType`: a trapping move hits again for the damage it did last.
    GetAnimationType,
    /// `PlayerCanExecuteMove`: Rage goes on, with its effect gone.
    CanExecuteMove,
}

/// `50 percent + 1` and `25 percent`.
const HURT_ITSELF_BELOW: u8 = 128;
const FULLY_PARALYZED_BELOW: u8 = 63;

/// `CheckPlayerStatusConditions` and `CheckEnemyStatusConditions`, in order: sleep counts down and
/// wakes without moving; frozen; held by the other's trapping move; flinched; recharging. Then a
/// disabled move counts down; confusion counts down, and otherwise half of random bytes hurt the
/// mon, clearing every other flag in its first status byte; a disabled move chosen before it was
/// disabled; a quarter of random bytes fully paralyse. Hurt or paralysed, Bide, Thrash, charging
/// and trapping end. Then Bide stores the damage just taken and unleashes it doubled after its
/// turns; Thrash counts down into 2 to 5 turns of confusion; a trapping move counts down; Rage
/// goes on without its effect.
pub fn check_status_conditions(battle: &mut Battle, party: &[PartyMon], side: Side, rng: &mut impl Rng)
                               -> (Continuation, Vec<BattleText>) {
    let mut texts = vec![];
    let other_traps = battle.side(side.other()).status1.contains(Status1::USING_TRAPPING_MOVE);
    let me = battle.side_mut(side);
    if me.mon.status & status::SLP_MASK != 0 {
        // The whole status byte is written back, so a sleeping mon's other bits go.
        me.mon.status = (me.mon.status & status::SLP_MASK) - 1;
        texts.push(if me.mon.status != 0 { BattleText::FastAsleepText } else { BattleText::WokeUpText });
        me.used_move = 0;
        return (Continuation::MoveDone, texts);
    }
    if me.mon.status & status::FRZ != 0 {
        me.used_move = 0;
        return (Continuation::MoveDone, vec![BattleText::IsFrozenText]);
    }
    if other_traps {
        return (Continuation::MoveDone, vec![BattleText::CantMoveText]);
    }
    if me.status1.contains(Status1::FLINCHED) {
        me.status1.remove(Status1::FLINCHED);
        return (Continuation::MoveDone, vec![BattleText::FlinchedText]);
    }
    if me.status2.contains(Status2::NEEDS_TO_RECHARGE) {
        me.status2.remove(Status2::NEEDS_TO_RECHARGE);
        return (Continuation::MoveDone, vec![BattleText::MustRechargeText]);
    }
    if me.disabled_move != 0 {
        me.disabled_move -= 1;
        if me.disabled_move & 0xF == 0 {
            me.disabled_move = 0;
            me.disabled_move_number = 0;
            texts.push(BattleText::DisabledNoMoreText);
        }
    }
    let mut stopped = false;
    if me.status1.contains(Status1::CONFUSED) {
        me.confused_counter = me.confused_counter.wrapping_sub(1);
        if me.confused_counter == 0 {
            me.status1.remove(Status1::CONFUSED);
            texts.push(BattleText::ConfusedNoMoreText);
        } else {
            texts.push(BattleText::IsConfusedText);
            if rng.random() >= HURT_ITSELF_BELOW {
                me.status1 &= Status1::CONFUSED;
                texts.extend(handle_self_confusion_damage(battle, party, side));
                stopped = true;
            }
        }
    }
    if !stopped {
        let me = battle.side_mut(side);
        if me.disabled_move_number != 0 && me.disabled_move_number == me.selected_move {
            me.status1.remove(Status1::CHARGING_UP);
            texts.push(BattleText::MoveIsDisabledText);
            return (Continuation::MoveDone, texts);
        }
        if me.mon.status & status::PAR != 0 && rng.random() < FULLY_PARALYZED_BELOW {
            texts.push(BattleText::FullyParalyzedText);
            stopped = true;
        }
    }
    let damage = battle.damage;
    let me = battle.side_mut(side);
    if stopped {
        me.status1.remove(Status1::STORING_ENERGY | Status1::THRASHING_ABOUT | Status1::CHARGING_UP | Status1::USING_TRAPPING_MOVE);
        return (Continuation::MoveDone, texts);
    }
    if me.status1.contains(Status1::STORING_ENERGY) {
        me.current_move.animation = 0;
        me.bide_accumulated_damage = me.bide_accumulated_damage.wrapping_add(damage);
        me.num_attacks_left = me.num_attacks_left.wrapping_sub(1);
        if me.num_attacks_left != 0 {
            return (Continuation::MoveDone, texts);
        }
        me.status1.remove(Status1::STORING_ENERGY);
        texts.push(BattleText::UnleashedEnergyText);
        me.current_move.power = 1;
        let doubled = me.bide_accumulated_damage.wrapping_shl(1);
        me.bide_accumulated_damage = 0;
        me.current_move.animation = PokemonMoveName::Bide as u8;
        battle.damage = doubled;
        if doubled == 0 {
            battle.move_missed = true;
        }
        return (Continuation::HandleIfMoveMissed, texts);
    }
    if me.status1.contains(Status1::THRASHING_ABOUT) {
        me.current_move.animation = PokemonMoveName::Thrash as u8;
        texts.push(BattleText::ThrashingAboutText);
        me.num_attacks_left = me.num_attacks_left.wrapping_sub(1);
        if me.num_attacks_left == 0 {
            me.status1.remove(Status1::THRASHING_ABOUT);
            me.status1 |= Status1::CONFUSED;
            me.confused_counter = (rng.random() & 3) + 2;
        }
        return (Continuation::CalcMoveDamage, texts);
    }
    if me.status1.contains(Status1::USING_TRAPPING_MOVE) {
        texts.push(BattleText::AttackContinuesText);
        me.num_attacks_left = me.num_attacks_left.wrapping_sub(1);
        return (Continuation::GetAnimationType, texts);
    }
    if me.status2.contains(Status2::USING_RAGE) {
        me.current_move.effect = 0;
        return (Continuation::CanExecuteMove, texts);
    }
    (Continuation::Move, texts)
}

/// `HandleSelfConfusionDamage` and the enemy's copy inside its status check: a typeless 40-power
/// hit on its own defense, which stands in for the target's for the calculation and so is doubled
/// by the *target's* Reflect. It never crits, is not randomised, and leaves the move's power at 40
/// and its type Normal. It lands on the mon itself, or on the other mon's substitute if the mon has
/// one of its own.
pub fn handle_self_confusion_damage(battle: &mut Battle, party: &[PartyMon], side: Side) -> Vec<BattleText> {
    let mut texts = vec![BattleText::HurtItselfText];
    let target_defense = battle.side(side.other()).mon.stats[stat::DEFENSE];
    battle.side_mut(side.other()).mon.stats[stat::DEFENSE] = battle.side(side).mon.stats[stat::DEFENSE];
    let me = battle.side_mut(side);
    let saved_effect = me.current_move.effect;
    me.current_move.effect = 0;
    me.current_move.power = 40;
    me.current_move.move_type = 0;
    battle.critical_hit_or_ohko = CriticalHitOrOhko::Normal;
    let vars = get_damage_vars(battle, party, side).expect("40 power");
    calculate_damage(battle, side, vars);
    battle.side_mut(side).current_move.effect = saved_effect;
    battle.side_mut(side.other()).mon.stats[stat::DEFENSE] = target_defense;
    texts.extend(apply_damage_to_pokemon(battle, side, side));
    texts
}

/// `HandlePoisonBurnLeechSeed` at the end of `side`'s turn: an eighth of an eighth of the max HP in
/// sixteen bits that assume a max below 1024, at least 1 in the low byte, off the mon for poison or a
/// burn and again for Leech Seed, which gives the other mon what the drain would have been even when
/// less was left. Badly poisoned, each drain counts the toxic counter up and multiplies by it,
/// Leech Seed's included. Whether the mon fainted.
pub fn handle_poison_burn_leech_seed(battle: &mut Battle, side: Side) -> (bool, Vec<BattleText>) {
    let mut texts = vec![];
    let me = battle.side(side);
    if me.mon.status & (status::BRN | status::PSN) != 0 {
        texts.push(if me.mon.status & status::BRN != 0 { BattleText::HurtByBurnText } else { BattleText::HurtByPoisonText });
        decrease_own_hp(battle, side);
    }
    if battle.side(side).status2.contains(Status2::SEEDED) {
        let drained = decrease_own_hp(battle, side);
        let other = &mut battle.side_mut(side.other()).mon;
        other.hp = other.hp.wrapping_add(drained);
        if other.hp >= other.stats[0] {
            other.hp = other.stats[0];
        }
        texts.push(BattleText::HurtByLeechSeedText);
    }
    (battle.side(side).mon.hp == 0, texts)
}

/// `HandlePoisonBurnLeechSeed_DecreaseOwnHP`: the damage worked out, whatever of it was left to take.
pub fn decrease_own_hp(battle: &mut Battle, side: Side) -> u16 {
    let me = battle.side_mut(side);
    let quarter = me.mon.stats[0] >> 2;
    let low = ((quarter & 0xFF) as u8 >> 2).max(1);
    let mut damage = (quarter >> 8) << 8 | low as u16;
    if me.status3.contains(Status3::BADLY_POISONED) {
        me.toxic_counter = me.toxic_counter.wrapping_add(1);
        let ticks = if me.toxic_counter == 0 { 256 } else { me.toxic_counter as u16 };
        damage = damage.wrapping_mul(ticks);
    }
    me.mon.hp = me.mon.hp.saturating_sub(damage);
    damage
}

/// `CheckNumAttacksLeft`: a trapping move with no turns left ends, for either side.
pub fn check_num_attacks_left(battle: &mut Battle) {
    for side in [&mut battle.player, &mut battle.enemy] {
        if side.num_attacks_left == 0 {
            side.status1.remove(Status1::USING_TRAPPING_MOVE);
        }
    }
}

/// `DecrementPP`, the player's only: nothing for Struggle, while Bide, Thrash, a multi-hit move or
/// Rage goes on; the battle mon's PP byte, and its party slot's unless transformed.
pub fn decrement_pp(battle: &mut Battle, party: &mut [PartyMon]) {
    let player = &mut battle.player;
    if player.selected_move == PokemonMoveName::Struggle as u8
        || player.status1.intersects(Status1::STORING_ENERGY | Status1::THRASHING_ABOUT | Status1::ATTACKING_MULTIPLE_TIMES)
        || player.status2.contains(Status2::USING_RAGE) {
        return;
    }
    let slot = player.move_list_index as usize;
    player.mon.pp[slot] = player.mon.pp[slot].wrapping_sub(1);
    if !player.status3.contains(Status3::TRANSFORMED) {
        let pp = &mut party[battle.player_mon_number as usize].mon.pp[slot];
        *pp = pp.wrapping_sub(1);
    }
}

/// `IncrementMovePP`: a PP back to the move in use, in battle and in the party it came from, so a
/// move that calls another spends one.
fn increment_move_pp(battle: &mut Battle, party: &mut [PartyMon], side: Side) {
    let me = battle.side_mut(side);
    let slot = me.move_list_index as usize;
    me.mon.pp[slot] = me.mon.pp[slot].wrapping_add(1);
    let owner = match side {
        Side::Player => party.get_mut(battle.player_mon_number as usize),
        Side::Enemy => battle.enemy_party.get_mut(battle.enemy.mon.party_pos as usize),
    };
    if let Some(mon) = owner {
        mon.mon.pp[slot] = mon.mon.pp[slot].wrapping_add(1);
    }
}

/// `ReloadMoveData`: the move the side now uses, and its PP back.
fn reload_move_data(battle: &mut Battle, party: &mut [PartyMon], side: Side, id: u8) {
    battle.side_mut(side).current_move = MoveData::of(id);
    increment_move_pp(battle, party, side);
}

/// `MirrorMoveCopyMove`: the move the other mon last used, which fails if there is none or it was
/// Mirror Move. Whether it was copied.
pub fn mirror_move_copy_move(battle: &mut Battle, party: &mut [PartyMon], side: Side) -> (bool, Vec<BattleText>) {
    let last = battle.side(side.other()).used_move;
    battle.side_mut(side).selected_move = last;
    if last == 0 || last == PokemonMoveName::MirrorMove as u8 {
        return (false, vec![BattleText::MirrorMoveFailedText]);
    }
    reload_move_data(battle, party, side, last);
    (true, vec![])
}

/// `MetronomePickMove`: a random byte from 1 below Struggle that is not Metronome.
pub fn metronome_pick_move(battle: &mut Battle, party: &mut [PartyMon], side: Side, rng: &mut impl Rng) {
    let picked = loop {
        let id = rng.random();
        if id != 0 && id < PokemonMoveName::Struggle as u8 && id != PokemonMoveName::Metronome as u8 {
            break id;
        }
    };
    battle.side_mut(side).selected_move = picked;
    reload_move_data(battle, party, side, picked);
}

/// `wCurrentMenuItem` and `wMaxMenuItem` as the move menu leaves them: the slot chosen, and one past
/// the number of moves, counted from the box border.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveMenu {
    pub current: u8,
    pub max: u8,
}

/// What `CheckForDisobedience` decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Obedience {
    pub obeys: bool,
    pub texts: Vec<BattleText>,
}

const BIT_CASCADEBADGE: u8 = 1;
const BIT_RAINBOWBADGE: u8 = 3;
const BIT_MARSHBADGE: u8 = 5;
const BIT_EARTHBADGE: u8 = 7;

/// `CheckForDisobedience`: only a traded mon, only over the level its badges allow. A random
/// byte, nybbles swapped and drawn again at or over level plus limit in a byte, below the limit
/// obeys; another, not swapped, below the limit uses another move at random; otherwise a third
/// byte, swapped, below the excess naps for 1 to 7 turns, at least twice the excess does nothing,
/// and between them the mon hurts itself. Another move needs a second move known, no disabled
/// move, not Struggle, and PP left besides the chosen move's, and is a random slot below
/// `wMaxMenuItem` other than the chosen one with a PP byte, which the menu is moved onto.
pub fn check_for_disobedience(battle: &mut Battle, party: &[PartyMon], player_id: u16, badges: u8,
                             menu: &mut MoveMenu, rng: &mut impl Rng) -> Obedience {
    battle.mon_is_disobedient = false;
    let obeys = Obedience { obeys: true, texts: vec![] };
    if party[battle.player_mon_number as usize].mon.ot_id == player_id {
        return obeys;
    }
    let limit: u8 = if badges & 1 << BIT_EARTHBADGE != 0 { 101 }
        else if badges & 1 << BIT_MARSHBADGE != 0 { 70 }
        else if badges & 1 << BIT_RAINBOWBADGE != 0 { 50 }
        else if badges & 1 << BIT_CASCADEBADGE != 0 { 30 }
        else { 10 };
    let level = battle.player.mon.level;
    if limit >= level {
        return obeys;
    }
    let bound = limit.checked_add(level).unwrap_or(0xFF);
    let below_bound = |rng: &mut dyn FnMut() -> u8| loop {
        let random = rng();
        if random < bound {
            break random;
        }
    };
    if below_bound(&mut || rng.random().rotate_left(4)) < limit {
        return obeys;
    }
    if below_bound(&mut || rng.random()) < limit {
        let player = &battle.player;
        let pp_total = player.mon.pp.iter().fold(0u8, |total, &pp| total.wrapping_add(pp & PP_MASK));
        let chosen_pp = player.mon.pp[menu.current as usize] & PP_MASK;
        if player.mon.moves[1].is_none() || player.disabled_move_number != 0
            || player.selected_move == PokemonMoveName::Struggle as u8 || pp_total == chosen_pp {
            return Obedience { obeys: false, texts: vec![does_nothing(rng.random())] };
        }
        battle.mon_is_disobedient = true;
        let slot = loop {
            let slot = rng.random() & 3;
            if slot < menu.max && slot != menu.current && battle.player.mon.pp[slot as usize] != 0 {
                break slot;
            }
        };
        menu.current = slot;
        let id = battle.player.mon.moves[slot as usize].map_or(0, |name| name as u8);
        battle.player.selected_move = id;
        battle.player.current_move = MoveData::of(id);
        return obeys;
    }
    let excess = level - limit;
    let texts = match rng.random().rotate_left(4).checked_sub(excess) {
        None => {
            battle.player.mon.status = loop {
                let turns = rng.random().wrapping_shl(1).rotate_left(4) & status::SLP_MASK;
                if turns != 0 {
                    break turns;
                }
            };
            vec![BattleText::BeganToNapText]
        }
        Some(over) if over >= excess => vec![does_nothing(rng.random())],
        Some(_) => {
            let mut texts = vec![BattleText::WontObeyText];
            texts.extend(handle_self_confusion_damage(battle, party, Side::Player));
            texts
        }
    };
    Obedience { obeys: false, texts }
}

/// `.monDoesNothing`: which of four texts, by a random byte's low two bits.
fn does_nothing(random: u8) -> BattleText {
    match random & 3 {
        0 => BattleText::LoafingAroundText,
        1 => BattleText::WontObeyText,
        2 => BattleText::TurnedAwayText,
        _ => BattleText::IgnoredOrdersText,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use super::super::fixture::{each_case, side};
    use super::*;

    #[test]
    fn every_harvested_case_of_check_status_conditions() {
        each_case(include_str!("../../../fixtures/battle/check_status_conditions.jsonl"), |arena, input, rng| {
            json!(check_status_conditions(&mut arena.battle, &arena.party, side(input), rng))
        });
    }

    #[test]
    fn every_harvested_case_of_handle_poison_burn_leech_seed() {
        each_case(include_str!("../../../fixtures/battle/handle_poison_burn_leech_seed.jsonl"), |arena, input, _| {
            json!(handle_poison_burn_leech_seed(&mut arena.battle, side(input)))
        });
    }

    #[test]
    fn every_harvested_case_of_mirror_move_copy_move() {
        each_case(include_str!("../../../fixtures/battle/mirror_move_copy_move.jsonl"), |arena, input, _| {
            json!(mirror_move_copy_move(&mut arena.battle, &mut arena.party, side(input)))
        });
    }

    #[test]
    fn every_harvested_case_of_metronome_pick_move() {
        each_case(include_str!("../../../fixtures/battle/metronome_pick_move.jsonl"), |arena, input, rng| {
            metronome_pick_move(&mut arena.battle, &mut arena.party, side(input), rng);
            json!([Value::Null, []])
        });
    }

    #[test]
    fn every_harvested_case_of_decrement_pp() {
        each_case(include_str!("../../../fixtures/battle/decrement_pp.jsonl"), |arena, _, _| {
            decrement_pp(&mut arena.battle, &mut arena.party);
            json!([Value::Null, []])
        });
    }

    #[test]
    fn every_harvested_case_of_check_num_attacks_left() {
        each_case(include_str!("../../../fixtures/battle/check_num_attacks_left.jsonl"), |arena, _, _| {
            check_num_attacks_left(&mut arena.battle);
            json!([Value::Null, []])
        });
    }

    #[test]
    fn every_harvested_case_of_handle_self_confusion_damage() {
        each_case(include_str!("../../../fixtures/battle/handle_self_confusion_damage.jsonl"), |arena, _, _| {
            json!([Value::Null, handle_self_confusion_damage(&mut arena.battle, &arena.party, Side::Player)])
        });
    }

    #[test]
    fn every_harvested_case_of_check_for_disobedience() {
        each_case(include_str!("../../../fixtures/battle/check_for_disobedience.jsonl"), |arena, input, rng| {
            let mut menu: MoveMenu = serde_json::from_value(input["menu"].clone()).unwrap();
            let obedience = check_for_disobedience(&mut arena.battle, &arena.party, arena.player_id, arena.badges, &mut menu, rng);
            json!([{"obeys": obedience.obeys, "menu": menu}, obedience.texts])
        });
    }
}
