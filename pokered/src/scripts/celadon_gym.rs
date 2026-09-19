//! `CeladonGym_Script`: Erika, the Rainbow Badge and TM21.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_CELADON_GYM_TRAINER_0, EVENT_BEAT_CELADON_GYM_TRAINER_6,
    EVENT_BEAT_ERIKA, EVENT_GOT_TM21};
use poke_core::symbols::pokered_local_labels::CeladonGymErikaText;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_CELADONGYM_DEFAULT, SCRIPT_CELADONGYM_ERIKA_POST_BATTLE,
    TEXT_CELADONGYM_BEAUTY1, TEXT_CELADONGYM_BEAUTY2, TEXT_CELADONGYM_BEAUTY3, TEXT_CELADONGYM_COOLTRAINER_F1,
    TEXT_CELADONGYM_COOLTRAINER_F2, TEXT_CELADONGYM_COOLTRAINER_F3, TEXT_CELADONGYM_COOLTRAINER_F4,
    TEXT_CELADONGYM_ERIKA, TEXT_CELADONGYM_RAINBOWBADGE_INFO, TEXT_CELADONGYM_RECEIVED_TM21,
    TEXT_CELADONGYM_TM21_NO_ROOM};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `BIT_RAINBOWBADGE`.
const BIT_RAINBOWBADGE: u8 = 3;
/// `wGymLeaderNo` for Erika.
const ERIKA: u8 = 4;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wCeladonGymCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wCeladonGymCurScript], a` after the table's routine.
    StoreCurScript,
    /// `CeladonGymReceiveTM21`, which both the post-battle script and Erika's own text run.
    ReceiveTm21,
    BadgeInfo,
    ReceivedTm21,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` her text does once the TM21 script has returned to it.
    TextDone,
    PreBattle,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(2) {
        rt.load_gym_leader_and_city_name("CELADON CITY", "ERIKA");
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().celadon_gym.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::CeladonGymTrainerHeaders);
    if index == SCRIPT_CELADONGYM_ERIKA_POST_BATTLE {
        return erika_post_battle(rt);
    }
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `CeladonGymErikaPostBattleScript`.
fn erika_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm21(rt)
}

/// `CeladonGymReceiveTM21`.
fn receive_tm21(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_CELADONGYM_RAINBOWBADGE_INFO).then(Label::BadgeInfo)
}

/// `CeladonGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().celadon_gym.cur_script = SCRIPT_CELADONGYM_DEFAULT;
    rt.set_cur_map_script(SCRIPT_CELADONGYM_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id == TEXT_CELADONGYM_ERIKA {
        return Some(erika_text(rt));
    }
    let header = match text_id {
        TEXT_CELADONGYM_COOLTRAINER_F1 => sym::CeladonGymTrainerHeader0,
        TEXT_CELADONGYM_BEAUTY1 => sym::CeladonGymTrainerHeader1,
        TEXT_CELADONGYM_COOLTRAINER_F2 => sym::CeladonGymTrainerHeader2,
        TEXT_CELADONGYM_BEAUTY2 => sym::CeladonGymTrainerHeader3,
        TEXT_CELADONGYM_COOLTRAINER_F3 => sym::CeladonGymTrainerHeader4,
        TEXT_CELADONGYM_BEAUTY3 => sym::CeladonGymTrainerHeader5,
        TEXT_CELADONGYM_COOLTRAINER_F4 => sym::CeladonGymTrainerHeader6,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `CeladonGymErikaText`.
fn erika_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_ERIKA) {
        return rt.print_text(text_at(CeladonGymErikaText::PreBattleText)).then(Label::PreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM21) {
        return Flow::Call(Label::ReceiveTm21.into(), Label::TextDone.into());
    }
    rt.print_text(text_at(CeladonGymErikaText::PostBattleAdviceText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().celadon_gym.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ReceiveTm21 => receive_tm21(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::BadgeInfo => {
            rt.set_event(EVENT_BEAT_ERIKA);
            match rt.give_item(ItemId::Tm21MegaDrain, 1) {
                true => rt.display_text_id(TEXT_CELADONGYM_RECEIVED_TM21).then(Label::ReceivedTm21),
                false => rt.display_text_id(TEXT_CELADONGYM_TM21_NO_ROOM).then(Label::GymVictory),
            }
        }
        Label::ReceivedTm21 => {
            rt.set_event(EVENT_GOT_TM21);
            resume(rt, Label::GymVictory)
        }
        // `.gymVictory`: her seven trainers are all marked beaten, so nobody stops the way out.
        Label::GymVictory => {
            rt.set_badge(BIT_RAINBOWBADGE);
            for event in EVENT_BEAT_CELADON_GYM_TRAINER_0..=EVENT_BEAT_CELADON_GYM_TRAINER_6 {
                rt.set_event(event);
            }
            reset_scripts(rt)
        }
        Label::PreBattle => {
            rt.save_end_battle_text(CeladonGymErikaText::ReceivedRainbowBadgeText);
            rt.engage_map_trainer(rt.sprite_index(), ERIKA);
            rt.maps().celadon_gym.cur_script = SCRIPT_CELADONGYM_ERIKA_POST_BATTLE;
            rt.set_cur_map_script(SCRIPT_CELADONGYM_ERIKA_POST_BATTLE);
            Flow::Return
        }
    }
}
