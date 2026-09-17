//! The effects that give the target a condition: sleep, poison, burn, freeze, paralysis,
//! confusion, flinching and Leech Seed.

use poke_core::move_name::PokemonMoveName;
use crate::party::PartyMon;
use crate::rng::Rng;
use super::super::accuracy::move_hit_test;
use super::super::modified_stats::apply_burn_and_paralysis_penalties;
use super::super::{effect, status, Battle, Side, Status1, Status2, Status3};
use super::{clear_hyper_beam, conditional_but_it_failed, target_has_substitute, BattleText};

const POISON: u8 = 3;
const GROUND: u8 = 4;
const FIRE: u8 = 20;
const GRASS: u8 = 22;
const ELECTRIC: u8 = 23;

/// `SleepEffect`. A target that must recharge wakes from that into sleep with no checks at all,
/// losing any status it had; otherwise one already asleep or with any status is unaffected, and a
/// miss is "didn't affect". Sleep lasts 1 to 7 turns, a random byte's low three bits drawn again
/// until they are not 0.
pub fn sleep_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let target = battle.side_mut(user.other());
    let recharging = target.status2.contains(Status2::NEEDS_TO_RECHARGE);
    target.status2.remove(Status2::NEEDS_TO_RECHARGE);
    if !recharging {
        if target.mon.status & status::SLP_MASK != 0 {
            return vec![BattleText::AlreadyAsleepText];
        }
        if target.mon.status != 0 {
            return vec![BattleText::DidntAffectText];
        }
        move_hit_test(battle, user, rng);
        if battle.move_missed {
            return vec![BattleText::DidntAffectText];
        }
    }
    let turns = loop {
        let turns = rng.random() & status::SLP_MASK;
        if turns != 0 {
            break turns;
        }
    };
    battle.side_mut(user.other()).mon.status = turns;
    vec![BattleText::FellAsleepText]
}

/// `20 percent + 1` and `40 percent + 1`.
const POISON_SIDE_EFFECT1_CHANCE: u8 = 52;
const POISON_SIDE_EFFECT2_CHANCE: u8 = 103;

/// `PoisonEffect`. A substitute, any status or a Poison type on either side of the target stops it,
/// silently for a side effect. A side effect then lands on a random byte below its chance, and a
/// poisoning move takes a hit test instead. Toxic, by move number, badly poisons and restarts the
/// counter.
pub fn poison_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let move_effect = battle.side(user).current_move.effect;
    let target = battle.side(user.other());
    if target_has_substitute(battle, user) || target.mon.status != 0 || target.mon.types.contains(&POISON) {
        return if move_effect == effect::POISON_EFFECT { vec![BattleText::DidntAffectText] } else { vec![] };
    }
    let chance = match move_effect {
        effect::POISON_SIDE_EFFECT1 => Some(POISON_SIDE_EFFECT1_CHANCE),
        effect::POISON_SIDE_EFFECT2 => Some(POISON_SIDE_EFFECT2_CHANCE),
        _ => None,
    };
    match chance {
        Some(chance) => if rng.random() >= chance {
            return vec![];
        },
        None => {
            move_hit_test(battle, user, rng);
            if battle.move_missed {
                return vec![BattleText::DidntAffectText];
            }
        }
    }
    let toxic = battle.side(user).current_move.animation == PokemonMoveName::Toxic as u8;
    let target = battle.side_mut(user.other());
    target.mon.status |= status::PSN;
    if toxic {
        target.status3 |= Status3::BADLY_POISONED;
        target.toxic_counter = 0;
        vec![BattleText::BadlyPoisonedText]
    } else {
        vec![BattleText::PoisonedText]
    }
}

/// `10 percent + 1` and `30 percent + 1`.
const SIDE_EFFECT1_CHANCE: u8 = 26;
const SIDE_EFFECT2_CHANCE: u8 = 77;

/// `FreezeBurnParalyzeEffect`: nothing through a substitute; a target with a status already can
/// only be thawed, by `CheckDefrost`; nothing if the move shares a type with the target. Otherwise
/// a tenth, or for the `_2` effects nearly a third, of random bytes inflict it: paralysis quarters
/// the speed, a burn halves the attack, and freezing ends the target's recharge, but only when the
/// player froze the enemy.
pub fn freeze_burn_paralyze_effect(battle: &mut Battle, party: &mut [PartyMon], user: Side, rng: &mut impl Rng)
                                   -> Vec<BattleText> {
    if target_has_substitute(battle, user) {
        return vec![];
    }
    let current = battle.side(user).current_move;
    if battle.side(user.other()).mon.status != 0 {
        return check_defrost(battle, party, user);
    }
    if battle.side(user.other()).mon.types.contains(&current.move_type) {
        return vec![];
    }
    let (chance, kind) = if current.effect <= effect::PARALYZE_SIDE_EFFECT1 {
        (SIDE_EFFECT1_CHANCE, current.effect)
    } else {
        (SIDE_EFFECT2_CHANCE, current.effect - (effect::PARALYZE_SIDE_EFFECT2 - effect::PARALYZE_SIDE_EFFECT1))
    };
    if rng.random() >= chance {
        return vec![];
    }
    match kind {
        effect::BURN_SIDE_EFFECT1 => {
            battle.side_mut(user.other()).mon.status = status::BRN;
            apply_burn_and_paralysis_penalties(battle, user.other());
            vec![BattleText::BurnedText]
        }
        effect::FREEZE_SIDE_EFFECT1 => {
            if user == Side::Player {
                clear_hyper_beam(battle, user);
            }
            battle.side_mut(user.other()).mon.status = status::FRZ;
            vec![BattleText::FrozenText]
        }
        _ => {
            battle.side_mut(user.other()).mon.status = status::PAR;
            apply_burn_and_paralysis_penalties(battle, user.other());
            vec![BattleText::ParalyzedMayNotAttackText]
        }
    }
}

/// `CheckDefrost`: a frozen target hit by a Fire move thaws, in battle and in its party slot.
fn check_defrost(battle: &mut Battle, party: &mut [PartyMon], user: Side) -> Vec<BattleText> {
    if battle.side(user.other()).mon.status & status::FRZ == 0 || battle.side(user).current_move.move_type != FIRE {
        return vec![];
    }
    battle.side_mut(user.other()).mon.status = 0;
    match user {
        Side::Player => {
            let pos = battle.enemy.mon.party_pos as usize;
            if let Some(mon) = battle.enemy_party.get_mut(pos) {
                mon.mon.status = 0;
            }
        }
        Side::Enemy => {
            if let Some(mon) = party.get_mut(battle.player_mon_number as usize) {
                mon.mon.status = 0;
            }
        }
    }
    vec![BattleText::FireDefrostedText]
}

/// `ParalyzeEffect_`: a target with any status is unaffected, an Electric move does not affect a
/// Ground type, and then a hit test.
pub fn paralyze_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let target = battle.side(user.other());
    if target.mon.status != 0 {
        return vec![BattleText::DidntAffectText];
    }
    if battle.side(user).current_move.move_type == ELECTRIC && target.mon.types.contains(&GROUND) {
        return vec![BattleText::DoesntAffectMonText];
    }
    move_hit_test(battle, user, rng);
    if battle.move_missed {
        return vec![BattleText::DidntAffectText];
    }
    battle.side_mut(user.other()).mon.status |= status::PAR;
    apply_burn_and_paralysis_penalties(battle, user.other());
    vec![BattleText::ParalyzedMayNotAttackText]
}

/// `10 percent`: below it, a confusion side effect lands.
const CONFUSION_SIDE_EFFECT_CHANCE: u8 = 25;

/// `ConfusionSideEffect`.
pub fn confusion_side_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    if rng.random() >= CONFUSION_SIDE_EFFECT_CHANCE {
        return vec![];
    }
    confuse_target(battle, user, rng)
}

/// `ConfusionEffect`: a substitute or a miss fails it.
pub fn confusion_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    if target_has_substitute(battle, user) {
        return conditional_but_it_failed(battle);
    }
    move_hit_test(battle, user, rng);
    if battle.move_missed {
        return conditional_but_it_failed(battle);
    }
    confuse_target(battle, user, rng)
}

/// `ConfusionSideEffectSuccess`: 2 to 5 turns, for a target not confused already.
fn confuse_target(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    let side_effect = battle.side(user).current_move.effect == effect::CONFUSION_SIDE_EFFECT;
    if battle.side(user.other()).status1.contains(Status1::CONFUSED) {
        return if side_effect { vec![] } else { conditional_but_it_failed(battle) };
    }
    let target = battle.side_mut(user.other());
    target.status1 |= Status1::CONFUSED;
    target.confused_counter = (rng.random() & 3) + 2;
    vec![BattleText::BecameConfusedText]
}

/// `FlinchSideEffect`: through no substitute, a tenth or nearly a third of random bytes make the
/// target flinch, which also ends its recharge.
pub fn flinch_side_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    if target_has_substitute(battle, user) {
        return vec![];
    }
    let chance = if battle.side(user).current_move.effect == effect::FLINCH_SIDE_EFFECT1 {
        SIDE_EFFECT1_CHANCE
    } else {
        SIDE_EFFECT2_CHANCE
    };
    if rng.random() >= chance {
        return vec![];
    }
    battle.side_mut(user.other()).status1 |= Status1::FLINCHED;
    clear_hyper_beam(battle, user);
    vec![]
}

/// `LeechSeedEffect_`: a hit test, then a Grass type or a target seeded already evades it.
pub fn leech_seed_effect(battle: &mut Battle, user: Side, rng: &mut impl Rng) -> Vec<BattleText> {
    move_hit_test(battle, user, rng);
    let missed = battle.move_missed;
    let target = battle.side_mut(user.other());
    if missed || target.mon.types.contains(&GRASS) || target.status2.contains(Status2::SEEDED) {
        return vec![BattleText::EvadedAttackText];
    }
    target.status2 |= Status2::SEEDED;
    vec![BattleText::WasSeededText]
}
