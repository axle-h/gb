//! `FuchsiaGym_Script`: Koga behind his four walls of poison, the Soul Badge and TM06.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_FUCHSIA_GYM_TRAINER_0, EVENT_BEAT_FUCHSIA_GYM_TRAINER_5,
    EVENT_BEAT_KOGA, EVENT_GOT_TM06};
use poke_core::symbols::pokered_local_labels::{FuchsiaGymGymGuideText as guide, FuchsiaGymKogaText as koga};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_FUCHSIAGYM_DEFAULT, SCRIPT_FUCHSIAGYM_KOGA_POST_BATTLE,
    TEXT_FUCHSIAGYM_GYM_GUIDE, TEXT_FUCHSIAGYM_KOGA, TEXT_FUCHSIAGYM_KOGA_RECEIVED_TM06,
    TEXT_FUCHSIAGYM_KOGA_SOUL_BADGE_INFO, TEXT_FUCHSIAGYM_KOGA_TM06_NO_ROOM, TEXT_FUCHSIAGYM_ROCKER1,
    TEXT_FUCHSIAGYM_ROCKER2, TEXT_FUCHSIAGYM_ROCKER3, TEXT_FUCHSIAGYM_ROCKER4, TEXT_FUCHSIAGYM_ROCKER5,
    TEXT_FUCHSIAGYM_ROCKER6};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `BIT_SOULBADGE`.
const BIT_SOULBADGE: u8 = 4;
/// `wGymLeaderNo` for Koga.
const KOGA: u8 = 5;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wFuchsiaGymCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wFuchsiaGymCurScript], a` after the table's routine.
    StoreCurScript,
    /// `FuchsiaGymReceiveTM06`, which both the post-battle script and his own text run.
    ReceiveTm06,
    BadgeInfo,
    ReceivedTm06,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` his text does once the TM06 script has returned to it.
    TextDone,
    PreBattle,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(2) {
        rt.load_gym_leader_and_city_name("FUCHSIA CITY", "KOGA");
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().fuchsia_gym.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::FuchsiaGymTrainerHeaders);
    if index == SCRIPT_FUCHSIAGYM_KOGA_POST_BATTLE {
        return koga_post_battle(rt);
    }
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `FuchsiaGymKogaPostBattleScript`.
fn koga_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm06(rt)
}

/// `FuchsiaGymReceiveTM06`.
fn receive_tm06(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_FUCHSIAGYM_KOGA_SOUL_BADGE_INFO).then(Label::BadgeInfo)
}

/// `FuchsiaGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().fuchsia_gym.cur_script = SCRIPT_FUCHSIAGYM_DEFAULT;
    rt.set_cur_map_script(SCRIPT_FUCHSIAGYM_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_FUCHSIAGYM_KOGA => return Some(koga_text(rt)),
        TEXT_FUCHSIAGYM_GYM_GUIDE => {
            let said = match rt.check_event(EVENT_BEAT_KOGA) {
                true => guide::BeatKogaText,
                false => guide::ChampInMakingText,
            };
            return Some(rt.print_text(text_at(said)).ret());
        }
        TEXT_FUCHSIAGYM_ROCKER1 => sym::FuchsiaGymTrainerHeader0,
        TEXT_FUCHSIAGYM_ROCKER2 => sym::FuchsiaGymTrainerHeader1,
        TEXT_FUCHSIAGYM_ROCKER3 => sym::FuchsiaGymTrainerHeader2,
        TEXT_FUCHSIAGYM_ROCKER4 => sym::FuchsiaGymTrainerHeader3,
        TEXT_FUCHSIAGYM_ROCKER5 => sym::FuchsiaGymTrainerHeader4,
        TEXT_FUCHSIAGYM_ROCKER6 => sym::FuchsiaGymTrainerHeader5,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `FuchsiaGymKogaText`: he hands the badge over himself if a full bag stopped the TM.
fn koga_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_KOGA) {
        return rt.print_text(text_at(koga::BeforeBattleText)).then(Label::PreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM06) {
        return Flow::Call(Label::ReceiveTm06.into(), Label::TextDone.into());
    }
    rt.print_text(text_at(koga::PostBattleAdviceText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().fuchsia_gym.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ReceiveTm06 => receive_tm06(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::BadgeInfo => {
            rt.set_event(EVENT_BEAT_KOGA);
            match rt.give_item(ItemId::Tm06Toxic, 1) {
                true => rt.display_text_id(TEXT_FUCHSIAGYM_KOGA_RECEIVED_TM06).then(Label::ReceivedTm06),
                false => rt.display_text_id(TEXT_FUCHSIAGYM_KOGA_TM06_NO_ROOM).then(Label::GymVictory),
            }
        }
        Label::ReceivedTm06 => {
            rt.set_event(EVENT_GOT_TM06);
            resume(rt, Label::GymVictory)
        }
        // `.gymVictory`: his six trainers are all marked beaten, so nobody stops the way out.
        Label::GymVictory => {
            rt.set_badge(BIT_SOULBADGE);
            for event in EVENT_BEAT_FUCHSIA_GYM_TRAINER_0..=EVENT_BEAT_FUCHSIA_GYM_TRAINER_5 {
                rt.set_event(event);
            }
            reset_scripts(rt)
        }
        Label::PreBattle => {
            rt.save_end_battle_text(koga::ReceivedSoulBadgeText);
            rt.engage_map_trainer(rt.sprite_index(), KOGA);
            rt.clear_joy_held();
            rt.maps().fuchsia_gym.cur_script = SCRIPT_FUCHSIAGYM_KOGA_POST_BATTLE;
            Flow::Return
        }
    }
}
