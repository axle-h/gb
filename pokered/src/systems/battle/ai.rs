//! `engine/battle/trainer_ai.asm` and `SelectEnemyMove`: the enemy's choice of move, and a trainer's
//! choice to use an item or switch instead.

use poke_core::item::ItemId;
use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::trainers::{ai_pointer, move_choices};
use serde::{Deserialize, Serialize};
use crate::party::NUM_MOVES;
use crate::rng::Rng;
use crate::systems::math::divide;
use super::damage::{ai_get_type_effectiveness, AI_NEUTRAL};
use super::effects::stat_modifiers::stat_modifier_up_effect;
use super::effects::BattleText;
use super::{effect, status, Battle, BattleKind, Status1, Status2, Status3};

/// `AIEnemyTrainerChooseMoves`'s starting score: the lowest score wins.
const NEUTRAL_SCORE: u8 = 10;
/// What a disabled move is scored.
const DISABLED_SCORE: u8 = 0x50;

/// `ReadMove`: the enemy's current move loaded from `Moves`, which every layer does in passing.
fn read_move(battle: &mut Battle, id: u8) -> MoveData {
    battle.enemy.current_move = MoveData::of(id);
    battle.enemy.current_move
}

/// `AIEnemyTrainerChooseMoves`: the moves the enemy may pick from, as bytes, 0 where a slot is
/// ruled out. A class with no layers picks from its whole moveset. Every move starts on 10 and a
/// disabled one on `$50`; each of the class's layers adds to or takes from them, and the lowest
/// scores survive. Scoring leaves the last move a layer read as the enemy's current move.
pub fn ai_enemy_trainer_choose_moves(battle: &mut Battle) -> [u8; NUM_MOVES] {
    let moves = battle.enemy.mon.moves.map(|name| name.map_or(0, |name| name as u8));
    let mut scores = [NEUTRAL_SCORE; NUM_MOVES];
    let disabled = battle.enemy.disabled_move >> 4;
    if let Some(score) = scores.get_mut((disabled as usize).wrapping_sub(1)) {
        *score = DISABLED_SCORE;
    }
    let layers = move_choices(battle.trainer_class);
    if layers.is_empty() {
        return moves;
    }
    for layer in layers {
        match layer {
            1 => ai_move_choice_modification_1(battle, &moves, &mut scores),
            2 => ai_move_choice_modification_2(battle, &moves, &mut scores),
            3 => ai_move_choice_modification_3(battle, &moves, &mut scores),
            _ => {}
        }
    }
    let chosen = find_minimum_entries(&moves, &mut scores);
    std::array::from_fn(|slot| if moves[slot] != 0 && chosen[slot] == 1 { moves[slot] } else { 0 })
}

/// The moves up to the first empty slot, as the layers walk them.
fn known(moves: &[u8; NUM_MOVES]) -> impl Iterator<Item = (usize, u8)> + '_ {
    moves.iter().copied().enumerate().take_while(|&(_, id)| id != 0)
}

/// `EFFECT_01`, `SLEEP_EFFECT`, `POISON_EFFECT` and `PARALYZE_EFFECT`: `StatusAilmentMoveEffects`.
const STATUS_AILMENT_MOVE_EFFECTS: [u8; 4] = [effect::EFFECT_01, effect::SLEEP_EFFECT, effect::POISON_EFFECT,
    effect::PARALYZE_EFFECT];

/// `AIMoveChoiceModification1`: against a mon with a status already, a move with no power that only
/// inflicts one scores 5 worse.
fn ai_move_choice_modification_1(battle: &mut Battle, moves: &[u8; NUM_MOVES], scores: &mut [u8; NUM_MOVES]) {
    if battle.player.mon.status == 0 {
        return;
    }
    for (slot, id) in known(moves) {
        let data = read_move(battle, id);
        if data.power == 0 && STATUS_AILMENT_MOVE_EFFECTS.contains(&data.effect) {
            scores[slot] = scores[slot].wrapping_add(5);
        }
    }
}

/// `AIMoveChoiceModification2`: with `wAILayer2Encouragement` at 1, the effects from `ATTACK_UP1`
/// to Haze and from `ATTACK_UP2` to Reflect score one better.
fn ai_move_choice_modification_2(battle: &mut Battle, moves: &[u8; NUM_MOVES], scores: &mut [u8; NUM_MOVES]) {
    if battle.ai_layer2_encouragement != 1 {
        return;
    }
    for (slot, id) in known(moves) {
        let move_effect = read_move(battle, id).effect;
        if (effect::ATTACK_UP1_EFFECT..effect::BIDE_EFFECT).contains(&move_effect)
            || (effect::ATTACK_UP2_EFFECT..effect::POISON_EFFECT).contains(&move_effect) {
            scores[slot] = scores[slot].wrapping_sub(1);
        }
    }
}

/// `AIMoveChoiceModification3`: a super-effective move scores one better, and a not very or not
/// at all effective one one worse if the enemy has anything better: Super Fang, a fixed-damage
/// move, Fly, or a move of another type with power. Effectiveness is `AIGetTypeEffectiveness`'s,
/// so a move that is neither counts as neither, however the two types combine.
fn ai_move_choice_modification_3(battle: &mut Battle, moves: &[u8; NUM_MOVES], scores: &mut [u8; NUM_MOVES]) {
    for (slot, id) in known(moves) {
        let move_type = read_move(battle, id).move_type;
        let effectiveness = ai_get_type_effectiveness(battle);
        if effectiveness == AI_NEUTRAL {
            continue;
        }
        if effectiveness > AI_NEUTRAL {
            scores[slot] = scores[slot].wrapping_sub(1);
            continue;
        }
        let better = known(moves).any(|(_, other)| {
            let data = read_move(battle, other);
            matches!(data.effect, effect::SUPER_FANG_EFFECT | effect::SPECIAL_DAMAGE_EFFECT | effect::FLY_EFFECT)
                || data.move_type != move_type && data.power != 0
        });
        if better {
            scores[slot] = scores[slot].wrapping_add(1);
        }
    }
}

/// `.loopFindMinimumEntries`: every score counted down in turn, the pass starting over at the first
/// empty slot or past the last, until one reaches 0; the pass that did so is then undone up to it,
/// which leaves 1 in exactly the lowest.
fn find_minimum_entries(moves: &[u8; NUM_MOVES], scores: &mut [u8; NUM_MOVES]) -> [u8; NUM_MOVES] {
    'pass: loop {
        for slot in 0..NUM_MOVES {
            if moves[slot] == 0 {
                continue 'pass;
            }
            scores[slot] = scores[slot].wrapping_sub(1);
            if scores[slot] == 0 {
                for undone in 0..=slot {
                    scores[undone] = scores[undone].wrapping_add(1);
                }
                return *scores;
            }
        }
    }
}

/// `25 percent`, `50 percent` and `75 percent - 1`: the random bytes that pick moves 1 to 3.
const MOVE_THRESHOLDS: [u8; 3] = [63, 127, 190];
/// `CANNOT_MOVE`.
pub const CANNOT_MOVE: u8 = 0xFF;

/// `SelectEnemyMove`: nothing while the enemy recharges, rages, charges, thrashes, sleeps, is frozen,
/// wraps or bides; `CANNOT_MOVE` while wrapped; Struggle for a mon whose only move is disabled.
/// Otherwise a slot at random from the moveset, or in a trainer battle from what the trainer AI
/// allows, drawn again for a disabled slot or an empty one.
pub fn select_enemy_move(battle: &mut Battle, rng: &mut impl Rng) {
    let enemy = &battle.enemy;
    if enemy.status2.intersects(Status2::NEEDS_TO_RECHARGE | Status2::USING_RAGE)
        || enemy.status1.intersects(Status1::CHARGING_UP | Status1::THRASHING_ABOUT)
        || enemy.mon.status & (status::FRZ | status::SLP_MASK) != 0
        || enemy.status1.intersects(Status1::USING_TRAPPING_MOVE | Status1::STORING_ENERGY) {
        return;
    }
    if battle.player.status1.contains(Status1::USING_TRAPPING_MOVE) {
        battle.enemy.selected_move = CANNOT_MOVE;
        return;
    }
    if enemy.mon.moves[1].is_none() && enemy.disabled_move != 0 {
        battle.enemy.selected_move = PokemonMoveName::Struggle as u8;
        return;
    }
    let choices = if battle.kind == BattleKind::Wild {
        battle.enemy.mon.moves.map(|name| name.map_or(0, |name| name as u8))
    } else {
        ai_enemy_trainer_choose_moves(battle)
    };
    loop {
        let random = rng.random();
        let slot = MOVE_THRESHOLDS.iter().position(|&threshold| random < threshold).unwrap_or(3);
        battle.enemy.move_list_index = slot as u8;
        if battle.enemy.disabled_move >> 4 == slot as u8 + 1 || choices[slot] == 0 {
            continue;
        }
        battle.enemy.selected_move = choices[slot];
        return;
    }
}

/// A trainer's move instead of an attack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiAction {
    /// `wAIItem`, used on the enemy's mon.
    UseItem(ItemId),
    /// `SwitchEnemyMon`: the mon going out is `EnemySendOut`'s to choose.
    Switch,
}

/// A class's `TrainerAIPointers` routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiRoutine {
    Generic, Juggler, Blackbelt, Giovanni, CooltrainerM, CooltrainerF, Brock, Misty, LtSurge, Erika, Koga,
    Blaine, Sabrina, Rival2, Rival3, Lorelei, Bruno, Agatha, Lance,
}

impl AiRoutine {
    fn of(address: u16) -> AiRoutine {
        [
            (sym::GenericAI, AiRoutine::Generic), (sym::JugglerAI, AiRoutine::Juggler),
            (sym::BlackbeltAI, AiRoutine::Blackbelt), (sym::GiovanniAI, AiRoutine::Giovanni),
            (sym::CooltrainerMAI, AiRoutine::CooltrainerM), (sym::CooltrainerFAI, AiRoutine::CooltrainerF),
            (sym::BrockAI, AiRoutine::Brock), (sym::MistyAI, AiRoutine::Misty), (sym::LtSurgeAI, AiRoutine::LtSurge),
            (sym::ErikaAI, AiRoutine::Erika), (sym::KogaAI, AiRoutine::Koga), (sym::BlaineAI, AiRoutine::Blaine),
            (sym::SabrinaAI, AiRoutine::Sabrina), (sym::Rival2AI, AiRoutine::Rival2), (sym::Rival3AI, AiRoutine::Rival3),
            (sym::LoreleiAI, AiRoutine::Lorelei), (sym::BrunoAI, AiRoutine::Bruno), (sym::AgathaAI, AiRoutine::Agatha),
            (sym::LanceAI, AiRoutine::Lance),
        ].into_iter().find(|(label, _)| label.address == address).expect("a trainer AI routine").1
    }
}

/// `percent` thresholds the routines compare the random byte against.
const PERCENT_8: u8 = 20;
const PERCENT_13_LESS_1: u8 = 32;
const PERCENT_25_PLUS_1: u8 = 64;
const PERCENT_50_PLUS_1: u8 = 128;

/// `TrainerAI`, before the enemy's move: nothing in a wild battle or once the class's uses for
/// this mon are spent, `$FF` in `wAICount` loading them. Otherwise one `Random` byte decides by
/// class. An item heals or cures at once and costs a use; switching costs none. What the routines
/// print is returned beside the action.
pub fn trainer_ai(battle: &mut Battle, badges: u8, rng: &mut impl Rng) -> (Option<AiAction>, Vec<BattleText>) {
    if battle.kind == BattleKind::Wild {
        return (None, vec![]);
    }
    let (count, address) = ai_pointer(battle.trainer_class);
    if battle.ai_count == 0 {
        return (None, vec![]);
    }
    if battle.ai_count == 0xFF {
        battle.ai_count = count;
    }
    let random = rng.random();
    let below = |fraction: u8, battle: &Battle| ai_check_if_hp_below_fraction(battle, fraction);
    use AiRoutine::*;
    let item = |item| Some(Use::Item(item));
    let choice = match AiRoutine::of(address) {
        Generic => None,
        Juggler => (random < PERCENT_25_PLUS_1).then_some(Use::Switch),
        Blackbelt => (random < PERCENT_13_LESS_1).then_some(Use::Item(ItemId::XAttack)),
        Giovanni => (random < PERCENT_25_PLUS_1).then_some(Use::Item(ItemId::GuardSpec)),
        CooltrainerM | Koga => (random < PERCENT_25_PLUS_1).then_some(Use::Item(ItemId::XAttack)),
        // The 25% gate is compared and never tested.
        CooltrainerF => if below(10, battle) { item(ItemId::HyperPotion) }
            else if below(5, battle) { Some(Use::Switch) } else { None },
        Brock => (battle.enemy.mon.status != 0).then_some(Use::Item(ItemId::FullHeal)),
        Misty | Bruno => (random < PERCENT_25_PLUS_1).then_some(Use::Item(ItemId::XDefend)),
        LtSurge => (random < PERCENT_25_PLUS_1).then_some(Use::Item(ItemId::XSpeed)),
        Erika => (random < PERCENT_50_PLUS_1 && below(10, battle)).then_some(Use::Item(ItemId::SuperPotion)),
        Blaine => (random < PERCENT_25_PLUS_1).then_some(Use::Item(ItemId::SuperPotion)),
        Sabrina => (random < PERCENT_25_PLUS_1 && below(10, battle)).then_some(Use::Item(ItemId::HyperPotion)),
        Rival2 => (random < PERCENT_13_LESS_1 && below(5, battle)).then_some(Use::Item(ItemId::Potion)),
        Rival3 => (random < PERCENT_13_LESS_1 && below(5, battle)).then_some(Use::Item(ItemId::FullRestore)),
        Lorelei => (random < PERCENT_50_PLUS_1 && below(5, battle)).then_some(Use::Item(ItemId::SuperPotion)),
        Agatha => if random < PERCENT_8 { Some(Use::Switch) }
            else { (random < PERCENT_50_PLUS_1 && below(4, battle)).then_some(Use::Item(ItemId::SuperPotion)) },
        Lance => (random < PERCENT_50_PLUS_1 && below(5, battle)).then_some(Use::Item(ItemId::HyperPotion)),
    };
    match choice {
        None => (None, vec![]),
        Some(Use::Switch) => ai_switch_if_enough_mons(battle),
        Some(Use::Item(item)) => {
            let texts = ai_use_item(battle, item, badges);
            battle.ai_count = battle.ai_count.wrapping_sub(1);
            (Some(AiAction::UseItem(item)), texts)
        }
    }
}

enum Use {
    Item(ItemId),
    Switch,
}

/// `AICheckIfHPBelowFraction`: the enemy's HP below its max HP over `fraction`, in whole points.
fn ai_check_if_hp_below_fraction(battle: &Battle, fraction: u8) -> bool {
    let max = battle.enemy.mon.stats[0].to_be_bytes();
    let (quotient, _) = divide([max[0], max[1], 0, 0], fraction, 2);
    battle.enemy.mon.hp < u16::from_be_bytes([quotient[2], quotient[3]])
}

/// What each item does to the enemy's mon; the text naming it is the battle mode's.
fn ai_use_item(battle: &mut Battle, item: ItemId, badges: u8) -> Vec<BattleText> {
    match item {
        ItemId::FullRestore => {
            ai_cure_status(battle);
            battle.enemy.mon.hp = battle.enemy.mon.stats[0];
        }
        ItemId::Potion | ItemId::SuperPotion | ItemId::HyperPotion => {
            let amount = match item { ItemId::Potion => 20, ItemId::SuperPotion => 50, _ => 200 };
            let mon = &mut battle.enemy.mon;
            mon.hp = mon.hp.wrapping_add(amount);
            if mon.hp > mon.stats[0] {
                mon.hp = mon.stats[0];
            }
        }
        ItemId::FullHeal => ai_cure_status(battle),
        ItemId::GuardSpec => battle.enemy.status2 |= Status2::PROTECTED_BY_MIST,
        _ => {
            // `AIIncreaseStat`: the stat-up effect run on the enemy's turn as though a move had it,
            // with its move number and effect put back afterwards.
            let saved = battle.enemy.current_move;
            battle.enemy.current_move.animation = XSTATITEM_DUPLICATE_ANIM;
            battle.enemy.current_move.effect = match item {
                ItemId::XAttack => effect::ATTACK_UP1_EFFECT,
                ItemId::XDefend => effect::DEFENSE_UP1_EFFECT,
                ItemId::XSpeed => effect::SPEED_UP1_EFFECT,
                _ => effect::SPECIAL_UP1_EFFECT,
            };
            let texts = stat_modifier_up_effect(battle, super::Side::Enemy, badges);
            battle.enemy.current_move.animation = saved.animation;
            battle.enemy.current_move.effect = saved.effect;
            return texts;
        }
    }
    vec![]
}

const XSTATITEM_DUPLICATE_ANIM: u8 = 0xAF;

/// `AICureStatus`: the status byte of the mon out and of its party copy, and Toxic's flag.
fn ai_cure_status(battle: &mut Battle) {
    let pos = battle.enemy.mon.party_pos as usize;
    if let Some(mon) = battle.enemy_party.get_mut(pos) {
        mon.mon.status = 0;
    }
    battle.enemy.mon.status = 0;
    battle.enemy.status3.remove(Status3::BADLY_POISONED);
}

/// `AISwitchIfEnoughMons`: a switch if two or more of the party have HP. `SwitchEnemyMon` copies the
/// mon out's HP and status back to its party slot, and its party position over the slot's box level.
fn ai_switch_if_enough_mons(battle: &mut Battle) -> (Option<AiAction>, Vec<BattleText>) {
    if battle.enemy_party.iter().filter(|mon| mon.mon.hp != 0).count() < 2 {
        return (None, vec![]);
    }
    let out = &battle.enemy.mon;
    let (hp, pos, status_byte) = (out.hp, out.party_pos, out.status);
    if let Some(slot) = battle.enemy_party.get_mut(pos as usize) {
        slot.mon.hp = hp;
        slot.mon.box_level = pos;
        slot.mon.status = status_byte;
    }
    (Some(AiAction::Switch), vec![BattleText::AIBattleWithdrawText])
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use super::super::fixture::each_case;
    use super::*;

    #[test]
    fn every_harvested_case_of_ai_enemy_trainer_choose_moves() {
        each_case(include_str!("../../../fixtures/battle/ai_enemy_trainer_choose_moves.jsonl"), |arena, _, _| {
            json!(ai_enemy_trainer_choose_moves(&mut arena.battle))
        });
    }

    #[test]
    fn every_harvested_case_of_select_enemy_move() {
        each_case(include_str!("../../../fixtures/battle/select_enemy_move.jsonl"), |arena, _, rng| {
            select_enemy_move(&mut arena.battle, rng);
            Value::Null
        });
    }

    #[test]
    fn every_harvested_case_of_trainer_ai() {
        each_case(include_str!("../../../fixtures/battle/trainer_ai.jsonl"), |arena, _, rng| {
            json!(trainer_ai(&mut arena.battle, arena.badges, rng))
        });
    }
}
