//! `CeruleanGym_Script`: Misty, the Cascade Badge and TM11.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_CERULEAN_GYM_TRAINER_0, EVENT_BEAT_CERULEAN_GYM_TRAINER_1,
    EVENT_BEAT_MISTY, EVENT_GOT_TM11};
use poke_core::symbols::pokered_local_labels::{CeruleanGymGymGuideText, CeruleanGymMistyText};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_CERULEANGYM_DEFAULT, SCRIPT_CERULEANGYM_MISTY_POST_BATTLE,
    TEXT_CERULEANGYM_COOLTRAINER_F, TEXT_CERULEANGYM_GYM_GUIDE, TEXT_CERULEANGYM_MISTY,
    TEXT_CERULEANGYM_MISTY_CASCADE_BADGE_INFO, TEXT_CERULEANGYM_MISTY_RECEIVED_TM11,
    TEXT_CERULEANGYM_MISTY_TM11_NO_ROOM, TEXT_CERULEANGYM_SWIMMER};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `BIT_CASCADEBADGE`.
const BIT_CASCADEBADGE: u8 = 1;
/// `wGymLeaderNo` for Misty.
const MISTY: u8 = 2;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wCeruleanGymCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wCeruleanGymCurScript], a` after the table's routine.
    StoreCurScript,
    /// `CeruleanGymReceiveTM11`, which both the post-battle script and Misty's own text run.
    ReceiveTm11,
    BadgeInfo,
    ReceivedTm11,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` Misty's text does once the TM11 script has returned to it.
    TextDone,
    PreBattle,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(2) {
        rt.load_gym_leader_and_city_name("CERULEAN CITY", "MISTY");
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().cerulean_gym.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::CeruleanGymTrainerHeaders);
    if index == SCRIPT_CERULEANGYM_MISTY_POST_BATTLE {
        return misty_post_battle(rt);
    }
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `CeruleanGymMistyPostBattleScript`.
fn misty_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm11(rt)
}

/// `CeruleanGymReceiveTM11`.
fn receive_tm11(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_CERULEANGYM_MISTY_CASCADE_BADGE_INFO).then(Label::BadgeInfo)
}

/// `CeruleanGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().cerulean_gym.cur_script = SCRIPT_CERULEANGYM_DEFAULT;
    rt.set_cur_map_script(SCRIPT_CERULEANGYM_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_CERULEANGYM_MISTY => misty_text(rt),
        TEXT_CERULEANGYM_COOLTRAINER_F => rt.talk_to_trainer(sym::CeruleanGymTrainerHeader0).ret(),
        TEXT_CERULEANGYM_SWIMMER => rt.talk_to_trainer(sym::CeruleanGymTrainerHeader1).ret(),
        TEXT_CERULEANGYM_GYM_GUIDE => {
            let words = match rt.check_event(EVENT_BEAT_MISTY) {
                true => CeruleanGymGymGuideText::BeatMistyText,
                false => CeruleanGymGymGuideText::ChampInMakingText,
            };
            rt.print_text(text_at(words)).ret()
        }
        _ => return None,
    })
}

/// `CeruleanGymMistyText`.
fn misty_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_MISTY) {
        return rt.print_text(text_at(CeruleanGymMistyText::PreBattleText)).then(Label::PreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM11) {
        return Flow::Call(Label::ReceiveTm11.into(), Label::TextDone.into());
    }
    rt.print_text(text_at(CeruleanGymMistyText::TM11ExplanationText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().cerulean_gym.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ReceiveTm11 => receive_tm11(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::BadgeInfo => {
            rt.set_event(EVENT_BEAT_MISTY);
            match rt.give_item(ItemId::Tm11Bubblebeam, 1) {
                true => rt.display_text_id(TEXT_CERULEANGYM_MISTY_RECEIVED_TM11).then(Label::ReceivedTm11),
                false => rt.display_text_id(TEXT_CERULEANGYM_MISTY_TM11_NO_ROOM).then(Label::GymVictory),
            }
        }
        Label::ReceivedTm11 => {
            rt.set_event(EVENT_GOT_TM11);
            resume(rt, Label::GymVictory)
        }
        Label::GymVictory => {
            rt.set_badge(BIT_CASCADEBADGE);
            rt.set_event(EVENT_BEAT_CERULEAN_GYM_TRAINER_0);
            rt.set_event(EVENT_BEAT_CERULEAN_GYM_TRAINER_1);
            reset_scripts(rt)
        }
        // Misty's text leaves `wCurMapScript` alone, so the pass she is talked to in finishes on the
        // table entry it started on and the post-battle script runs on the next one.
        Label::PreBattle => {
            rt.save_end_battle_text(sym::CeruleanGymMistyReceivedCascadeBadgeText);
            rt.engage_map_trainer(rt.sprite_index(), MISTY);
            rt.maps().cerulean_gym.cur_script = SCRIPT_CERULEANGYM_MISTY_POST_BATTLE;
            Flow::Return
        }
    }
}
