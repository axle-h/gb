//! The move effects of `engine/battle/effects.asm` and `engine/battle/move_effects/`, as what
//! they change and the texts they print, by the label that prints each. Animations, HUD redraws
//! and the waits between are the battle mode's.

pub mod condition;
pub mod hp;
pub mod stat_modifiers;
pub mod volatile;

use serde::{Deserialize, Serialize};
use crate::party::PartyMon;
use crate::rng::Rng;
use super::damage::one_hit_ko_effect;
use super::{effect, Battle, Side, Status2};

/// A text a battle routine prints, named by its label. Some go on to choose their own ending from
/// the battle, as `MonsStatsRoseText` does between "rose" and "greatly rose".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleText {
    AIBattleWithdrawText,
    AlreadyAsleepText,
    AttackContinuesText,
    BadlyPoisonedText,
    BecameConfusedText,
    BeganToNapText,
    BuildingRageText,
    BurnedText,
    ButItFailedText,
    CantEscapeText,
    CantMoveText,
    ChargeMoveEffectText,
    CoinsScatteredText,
    ConfusedNoMoreText,
    ConvertedTypeText,
    DidntAffectText,
    DisabledNoMoreText,
    DoesntAffectMonText,
    DreamWasEatenText,
    EvadedAttackText,
    FastAsleepText,
    FellAsleepBecameHealthyText,
    FellAsleepText,
    FireDefrostedText,
    FlinchedText,
    FrozenText,
    FullyParalyzedText,
    GettingPumpedText,
    GotAwayText,
    HasSubstituteText,
    HitWithRecoilText,
    HurtByBurnText,
    HurtByLeechSeedText,
    HurtByPoisonText,
    HurtItselfText,
    IgnoredOrdersText,
    IsConfusedText,
    IsFrozenText,
    IsUnaffectedText,
    LightScreenProtectedText,
    LoafingAroundText,
    MimicLearnedMoveText,
    MirrorMoveFailedText,
    MonsStatsFellText,
    MonsStatsRoseText,
    MoveIsDisabledText,
    MoveWasDisabledText,
    MustRechargeText,
    NoEffectText,
    NoRunningText,
    NothingHappenedText,
    ParalyzedMayNotAttackText,
    PoisonedText,
    RanAwayScaredText,
    RanFromBattleText,
    ReflectGainedArmorText,
    RegainedHealthText,
    ShroudedInMistText,
    StartedSleepingEffect,
    StatusChangesEliminatedText,
    SubstituteBrokeText,
    SubstituteText,
    SubstituteTookDamageText,
    SuckedHealthText,
    ThrashingAboutText,
    TooWeakSubstituteText,
    TransformedText,
    TurnedAwayText,
    UnleashedEnergyText,
    WasBlownAwayText,
    WasSeededText,
    WokeUpText,
    WontObeyText,
}

/// What the player's Mimic reads from its move menu: `wCurrentMenuItem` as the fight menu left it,
/// which is the slot the copied move goes into, and the enemy move chosen from `MoveSelectionMenu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MimicMenu {
    pub cursor: u8,
    pub chosen: u8,
}

/// `JumpMoveEffect`: the user's current move's effect. The effects with no routine (Mirror Move,
/// Swift, Super Fang, the fixed-damage moves, Jump Kick, Metronome) jump to 0 on the cartridge and
/// are never dispatched; here they do nothing.
pub fn jump_move_effect(battle: &mut Battle, party: &mut [PartyMon], user: Side, badges: u8, mimic: MimicMenu,
                        rng: &mut impl Rng) -> Vec<BattleText> {
    use effect::*;
    use condition::*;
    use hp::*;
    use stat_modifiers::*;
    use volatile::*;
    match battle.side(user).current_move.effect {
        EFFECT_01 | SLEEP_EFFECT => sleep_effect(battle, user, rng),
        POISON_SIDE_EFFECT1 | POISON_SIDE_EFFECT2 | POISON_EFFECT => poison_effect(battle, user, rng),
        DRAIN_HP_EFFECT | DREAM_EATER_EFFECT => drain_hp_effect(battle, party, user),
        BURN_SIDE_EFFECT1 | FREEZE_SIDE_EFFECT1 | PARALYZE_SIDE_EFFECT1 | BURN_SIDE_EFFECT2 | FREEZE_SIDE_EFFECT2
        | PARALYZE_SIDE_EFFECT2 => freeze_burn_paralyze_effect(battle, party, user, rng),
        EXPLODE_EFFECT => explode_effect(battle, user),
        ATTACK_UP1_EFFECT..=EVASION_UP1_EFFECT | ATTACK_UP2_EFFECT..=EVASION_UP2_EFFECT =>
            stat_modifier_up_effect(battle, user, badges),
        ATTACK_DOWN1_EFFECT..=EVASION_DOWN1_EFFECT | ATTACK_DOWN2_EFFECT..=EVASION_DOWN2_EFFECT
        | ATTACK_DOWN_SIDE_EFFECT..=0x4B => stat_modifier_down_effect(battle, user, badges, rng),
        PAY_DAY_EFFECT => pay_day_effect(battle, user),
        CONVERSION_EFFECT => conversion_effect(battle, user),
        HAZE_EFFECT => haze_effect(battle, user),
        BIDE_EFFECT => bide_effect(battle, user, rng),
        THRASH_PETAL_DANCE_EFFECT => thrash_petal_dance_effect(battle, user, rng),
        SWITCH_AND_TELEPORT_EFFECT => switch_and_teleport_effect(battle, party, user, rng),
        TWO_TO_FIVE_ATTACKS_EFFECT | EFFECT_1E | ATTACK_TWICE_EFFECT | TWINEEDLE_EFFECT =>
            two_to_five_attacks_effect(battle, user, rng),
        FLINCH_SIDE_EFFECT1 | FLINCH_SIDE_EFFECT2 => flinch_side_effect(battle, user, rng),
        OHKO_EFFECT => {
            one_hit_ko_effect(battle, user);
            vec![]
        }
        CHARGE_EFFECT | FLY_EFFECT => charge_effect(battle, user),
        TRAPPING_EFFECT => trapping_effect(battle, user, rng),
        MIST_EFFECT => mist_effect(battle, user),
        FOCUS_ENERGY_EFFECT => focus_energy_effect(battle, user),
        RECOIL_EFFECT => recoil_effect(battle, user),
        CONFUSION_EFFECT => confusion_effect(battle, user, rng),
        CONFUSION_SIDE_EFFECT => confusion_side_effect(battle, user, rng),
        HEAL_EFFECT => heal_effect(battle, user),
        TRANSFORM_EFFECT => transform_effect(battle, user),
        LIGHT_SCREEN_EFFECT | REFLECT_EFFECT => reflect_light_screen_effect(battle, user),
        PARALYZE_EFFECT => paralyze_effect(battle, user, rng),
        SUBSTITUTE_EFFECT => substitute_effect(battle, user),
        HYPER_BEAM_EFFECT => {
            battle.side_mut(user).status2 |= Status2::NEEDS_TO_RECHARGE;
            vec![]
        }
        RAGE_EFFECT => {
            battle.side_mut(user).status2 |= Status2::USING_RAGE;
            vec![]
        }
        MIMIC_EFFECT => mimic_effect(battle, user, mimic, rng),
        LEECH_SEED_EFFECT => leech_seed_effect(battle, user, rng),
        SPLASH_EFFECT => vec![BattleText::NoEffectText],
        DISABLE_EFFECT => disable_effect(battle, user, rng),
        _ => vec![],
    }
}

/// `ClearHyperBeam`: the *target* no longer needs to recharge.
fn clear_hyper_beam(battle: &mut Battle, user: Side) {
    battle.side_mut(user.other()).status2.remove(Status2::NEEDS_TO_RECHARGE);
}

/// `ConditionalPrintButItFailed`: nothing if the move itself landed this turn.
fn conditional_but_it_failed(battle: &Battle) -> Vec<BattleText> {
    if battle.move_didnt_miss { vec![] } else { vec![BattleText::ButItFailedText] }
}

/// `CheckTargetSubstitute`.
fn target_has_substitute(battle: &Battle, user: Side) -> bool {
    battle.side(user.other()).status2.contains(Status2::HAS_SUBSTITUTE_UP)
}

/// `ReadPlayerMonCurHPAndStatus`: the player's HP, party position and status back to its party
/// slot, the party position landing on the slot's box level.
pub fn read_player_mon_cur_hp_and_status(battle: &Battle, party: &mut [PartyMon]) {
    let mon = &battle.player.mon;
    if let Some(slot) = party.get_mut(battle.player_mon_number as usize) {
        slot.mon.hp = mon.hp;
        slot.mon.box_level = mon.party_pos;
        slot.mon.status = mon.status;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::super::fixture::{each_case, side};
    use super::*;

    macro_rules! effect_fixtures {
        ($($name:ident: $file:literal;)+) => {$(
            #[test]
            fn $name() {
                each_case(include_str!(concat!("../../../../fixtures/battle/", $file, ".jsonl")), |arena, input, rng| {
                    let menu = input.get("menu").map_or(MimicMenu { cursor: 0, chosen: 0 },
                        |menu| serde_json::from_value(menu.clone()).unwrap());
                    json!(jump_move_effect(&mut arena.battle, &mut arena.party, side(input), arena.badges, menu, rng))
                });
            }
        )+};
    }

    effect_fixtures! {
        every_harvested_case_of_sleep_effect: "sleep_effect";
        every_harvested_case_of_poison_effect: "poison_effect";
        every_harvested_case_of_drain_hp_effect: "drain_hp_effect";
        every_harvested_case_of_freeze_burn_paralyze_effect: "freeze_burn_paralyze_effect";
        every_harvested_case_of_explode_effect: "explode_effect";
        every_harvested_case_of_bide_effect: "bide_effect";
        every_harvested_case_of_thrash_petal_dance_effect: "thrash_petal_dance_effect";
        every_harvested_case_of_switch_and_teleport_effect: "switch_and_teleport_effect";
        every_harvested_case_of_two_to_five_attacks_effect: "two_to_five_attacks_effect";
        every_harvested_case_of_flinch_side_effect: "flinch_side_effect";
        every_harvested_case_of_charge_effect: "charge_effect";
        every_harvested_case_of_trapping_effect: "trapping_effect";
        every_harvested_case_of_mist_effect: "mist_effect";
        every_harvested_case_of_focus_energy_effect: "focus_energy_effect";
        every_harvested_case_of_recoil_effect: "recoil_effect";
        every_harvested_case_of_confusion_effect: "confusion_effect";
        every_harvested_case_of_heal_effect: "heal_effect";
        every_harvested_case_of_transform_effect: "transform_effect";
        every_harvested_case_of_reflect_light_screen_effect: "reflect_light_screen_effect";
        every_harvested_case_of_paralyze_effect: "paralyze_effect";
        every_harvested_case_of_substitute_effect: "substitute_effect";
        every_harvested_case_of_hyper_beam_effect: "hyper_beam_effect";
        every_harvested_case_of_rage_effect: "rage_effect";
        every_harvested_case_of_mimic_effect: "mimic_effect";
        every_harvested_case_of_leech_seed_effect: "leech_seed_effect";
        every_harvested_case_of_splash_effect: "splash_effect";
        every_harvested_case_of_disable_effect: "disable_effect";
        every_harvested_case_of_pay_day_effect: "pay_day_effect";
        every_harvested_case_of_conversion_effect: "conversion_effect";
        every_harvested_case_of_haze_effect: "haze_effect";
    }
}
