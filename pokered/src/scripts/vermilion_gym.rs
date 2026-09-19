//! `VermilionGym_Script`: the double doors the two rubbish-bin switches open, LT.SURGE, the Thunder
//! Badge and TM24.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_2ND_LOCK_OPENED, EVENT_BEAT_LT_SURGE,
    EVENT_BEAT_VERMILION_GYM_TRAINER_0, EVENT_BEAT_VERMILION_GYM_TRAINER_2, EVENT_GOT_TM24};
use poke_core::symbols::pokered_local_labels::{VermilionGymGymGuideText, VermilionGymLTSurgeText};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_VERMILIONGYM_DEFAULT,
    SCRIPT_VERMILIONGYM_LT_SURGE_AFTER_BATTLE, TEXT_VERMILIONGYM_GENTLEMAN, TEXT_VERMILIONGYM_GYM_GUIDE,
    TEXT_VERMILIONGYM_LT_SURGE, TEXT_VERMILIONGYM_LT_SURGE_RECEIVED_TM24,
    TEXT_VERMILIONGYM_LT_SURGE_THUNDER_BADGE_INFO, TEXT_VERMILIONGYM_LT_SURGE_TM24_NO_ROOM,
    TEXT_VERMILIONGYM_SAILOR, TEXT_VERMILIONGYM_SUPER_NERD};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `BIT_THUNDERBADGE`.
const BIT_THUNDERBADGE: u8 = 2;
/// `wGymLeaderNo` for LT.SURGE.
const LT_SURGE: u8 = 3;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// The block behind the bins: the double door, and the floor it becomes.
const DOUBLE_DOOR: u8 = 0x24;
const CLEAR_FLOOR: u8 = 0x05;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wVermilionGymCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wVermilionGymCurScript], a` after the table's routine.
    StoreCurScript,
    /// `VermilionGymLTSurgeReceiveTM24Script`, which the post-battle script and his own text run.
    ReceiveTm24,
    BadgeInfo,
    ReceivedTm24,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` his text does once the TM24 script has returned to it.
    TextDone,
    PreBattle,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(1) {
        rt.load_gym_leader_and_city_name("VERMILION CITY", "LT.SURGE");
    }
    // The second switch marks the map loaded again, which is how the doors open where they stand.
    if rt.check_and_reset_cur_map_loaded(2) {
        set_door_tile(rt);
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().vermilion_gym.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::VermilionGymTrainerHeaders);
    if index == SCRIPT_VERMILIONGYM_LT_SURGE_AFTER_BATTLE {
        return lt_surge_after_battle(rt);
    }
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `VermilionGymSetDoorTile`.
fn set_door_tile(rt: &mut Script) {
    let block = match rt.check_event(EVENT_2ND_LOCK_OPENED) {
        true => {
            rt.play_sound(sounds::SFX_GO_INSIDE);
            CLEAR_FLOOR
        }
        false => DOUBLE_DOOR,
    };
    rt.replace_tile_block(2, 2, block);
}

/// `VermilionGymLTSurgeAfterBattleScript`.
fn lt_surge_after_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm24(rt)
}

/// `VermilionGymLTSurgeReceiveTM24Script`.
fn receive_tm24(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_VERMILIONGYM_LT_SURGE_THUNDER_BADGE_INFO).then(Label::BadgeInfo)
}

/// `VermilionGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().vermilion_gym.cur_script = SCRIPT_VERMILIONGYM_DEFAULT;
    rt.set_cur_map_script(SCRIPT_VERMILIONGYM_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_VERMILIONGYM_LT_SURGE => lt_surge_text(rt),
        TEXT_VERMILIONGYM_GENTLEMAN => rt.talk_to_trainer(sym::VermilionGymTrainerHeader0).ret(),
        TEXT_VERMILIONGYM_SUPER_NERD => rt.talk_to_trainer(sym::VermilionGymTrainerHeader1).ret(),
        TEXT_VERMILIONGYM_SAILOR => rt.talk_to_trainer(sym::VermilionGymTrainerHeader2).ret(),
        TEXT_VERMILIONGYM_GYM_GUIDE => {
            let words = match rt.badges() & (1 << BIT_THUNDERBADGE) != 0 {
                true => VermilionGymGymGuideText::BeatLTSurgeText,
                false => VermilionGymGymGuideText::ChampInMakingText,
            };
            rt.print_text(text_at(words)).ret()
        }
        _ => return None,
    })
}

/// `VermilionGymLTSurgeText`.
fn lt_surge_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_LT_SURGE) {
        return rt.print_text(text_at(VermilionGymLTSurgeText::PreBattleText)).then(Label::PreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM24) {
        return Flow::Call(Label::ReceiveTm24.into(), Label::TextDone.into());
    }
    rt.print_text(text_at(VermilionGymLTSurgeText::PostBattleAdviceText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().vermilion_gym.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ReceiveTm24 => receive_tm24(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::BadgeInfo => {
            rt.set_event(EVENT_BEAT_LT_SURGE);
            match rt.give_item(ItemId::Tm24Thunderbolt, 1) {
                true => rt.display_text_id(TEXT_VERMILIONGYM_LT_SURGE_RECEIVED_TM24).then(Label::ReceivedTm24),
                false => rt.display_text_id(TEXT_VERMILIONGYM_LT_SURGE_TM24_NO_ROOM).then(Label::GymVictory),
            }
        }
        Label::ReceivedTm24 => {
            rt.set_event(EVENT_GOT_TM24);
            resume(rt, Label::GymVictory)
        }
        Label::GymVictory => {
            rt.set_badge(BIT_THUNDERBADGE);
            for event in EVENT_BEAT_VERMILION_GYM_TRAINER_0..=EVENT_BEAT_VERMILION_GYM_TRAINER_2 {
                rt.set_event(event);
            }
            reset_scripts(rt)
        }
        Label::PreBattle => {
            rt.save_end_battle_text(sym::VermilionGymLTSurgeReceivedThunderBadgeText);
            rt.engage_map_trainer(rt.sprite_index(), LT_SURGE);
            rt.maps().vermilion_gym.cur_script = SCRIPT_VERMILIONGYM_LT_SURGE_AFTER_BATTLE;
            rt.set_cur_map_script(SCRIPT_VERMILIONGYM_LT_SURGE_AFTER_BATTLE);
            Flow::Return
        }
    }
}
