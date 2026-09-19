//! `SaffronGym_Script`: Sabrina at the end of the teleport pads, the Marsh Badge and TM46.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_SABRINA, EVENT_BEAT_SAFFRON_GYM_TRAINER_0,
    EVENT_BEAT_SAFFRON_GYM_TRAINER_6, EVENT_GOT_TM46};
use poke_core::symbols::pokered_local_labels::{SaffronGymGymGuideText as guide, SaffronGymSabrinaText as sabrina};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SAFFRONGYM_DEFAULT, SCRIPT_SAFFRONGYM_SABRINA_POST_BATTLE,
    TEXT_SAFFRONGYM_CHANNELER1, TEXT_SAFFRONGYM_CHANNELER2, TEXT_SAFFRONGYM_CHANNELER3, TEXT_SAFFRONGYM_GYM_GUIDE,
    TEXT_SAFFRONGYM_SABRINA, TEXT_SAFFRONGYM_SABRINA_MARSH_BADGE_INFO, TEXT_SAFFRONGYM_SABRINA_RECEIVED_TM46,
    TEXT_SAFFRONGYM_SABRINA_TM46_NO_ROOM, TEXT_SAFFRONGYM_YOUNGSTER1, TEXT_SAFFRONGYM_YOUNGSTER2,
    TEXT_SAFFRONGYM_YOUNGSTER3, TEXT_SAFFRONGYM_YOUNGSTER4};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `BIT_MARSHBADGE`.
const BIT_MARSHBADGE: u8 = 5;
/// `wGymLeaderNo` for Sabrina.
const SABRINA: u8 = 6;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSaffronGymCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSaffronGymCurScript], a` after the table's routine.
    StoreCurScript,
    /// `SaffronGymSabrinaReceiveTM46Script`, which both the post-battle script and her own text run.
    ReceiveTm46,
    BadgeInfo,
    ReceivedTm46,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` her text does once the TM46 script has returned to it.
    TextDone,
    PreBattle,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(2) {
        rt.load_gym_leader_and_city_name("SAFFRON CITY", "SABRINA");
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().saffron_gym.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SaffronGymTrainerHeaders);
    if index == SCRIPT_SAFFRONGYM_SABRINA_POST_BATTLE {
        return sabrina_post_battle(rt);
    }
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `SaffronGymSabrinaPostBattle`.
fn sabrina_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm46(rt)
}

/// `SaffronGymSabrinaReceiveTM46Script`.
fn receive_tm46(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_SAFFRONGYM_SABRINA_MARSH_BADGE_INFO).then(Label::BadgeInfo)
}

/// `SaffronGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().saffron_gym.cur_script = SCRIPT_SAFFRONGYM_DEFAULT;
    rt.set_cur_map_script(SCRIPT_SAFFRONGYM_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SAFFRONGYM_SABRINA => return Some(sabrina_text(rt)),
        TEXT_SAFFRONGYM_GYM_GUIDE => {
            let said = match rt.check_event(EVENT_BEAT_SABRINA) {
                true => guide::BeatSabrinaText,
                false => guide::ChampInMakingText,
            };
            return Some(rt.print_text(text_at(said)).ret());
        }
        TEXT_SAFFRONGYM_CHANNELER1 => sym::SaffronGymTrainerHeader0,
        TEXT_SAFFRONGYM_YOUNGSTER1 => sym::SaffronGymTrainerHeader1,
        TEXT_SAFFRONGYM_CHANNELER2 => sym::SaffronGymTrainerHeader2,
        TEXT_SAFFRONGYM_YOUNGSTER2 => sym::SaffronGymTrainerHeader3,
        TEXT_SAFFRONGYM_CHANNELER3 => sym::SaffronGymTrainerHeader4,
        TEXT_SAFFRONGYM_YOUNGSTER3 => sym::SaffronGymTrainerHeader5,
        TEXT_SAFFRONGYM_YOUNGSTER4 => sym::SaffronGymTrainerHeader6,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `SaffronGymSabrinaText`: she hands the badge over herself if a full bag stopped the TM.
fn sabrina_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_SABRINA) {
        return rt.print_text(text_at(sabrina::Text)).then(Label::PreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM46) {
        return Flow::Call(Label::ReceiveTm46.into(), Label::TextDone.into());
    }
    rt.print_text(text_at(sabrina::PostBattleAdviceText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().saffron_gym.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ReceiveTm46 => receive_tm46(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::BadgeInfo => {
            rt.set_event(EVENT_BEAT_SABRINA);
            match rt.give_item(ItemId::Tm46Psywave, 1) {
                true => rt.display_text_id(TEXT_SAFFRONGYM_SABRINA_RECEIVED_TM46).then(Label::ReceivedTm46),
                false => rt.display_text_id(TEXT_SAFFRONGYM_SABRINA_TM46_NO_ROOM).then(Label::GymVictory),
            }
        }
        Label::ReceivedTm46 => {
            rt.set_event(EVENT_GOT_TM46);
            resume(rt, Label::GymVictory)
        }
        // `.gymVictory`: her seven trainers are all marked beaten, so nobody stops the way out.
        Label::GymVictory => {
            rt.set_badge(BIT_MARSHBADGE);
            for event in EVENT_BEAT_SAFFRON_GYM_TRAINER_0..=EVENT_BEAT_SAFFRON_GYM_TRAINER_6 {
                rt.set_event(event);
            }
            reset_scripts(rt)
        }
        Label::PreBattle => {
            rt.save_end_battle_text(sabrina::ReceivedMarshBadgeText);
            rt.engage_map_trainer(rt.sprite_index(), SABRINA);
            rt.maps().saffron_gym.cur_script = SCRIPT_SAFFRONGYM_SABRINA_POST_BATTLE;
            Flow::Return
        }
    }
}
