//! `PewterGym_Script`: Brock, the Boulder Badge and TM34, and the guide who offers his advice free.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_1ST_ROUTE22_RIVAL_BATTLE, EVENT_BEAT_BROCK,
    EVENT_BEAT_PEWTER_GYM_TRAINER_0, EVENT_GOT_TM34, EVENT_ROUTE22_RIVAL_WANTS_BATTLE};
use poke_core::symbols::pokered_local_labels::PewterGymBrockText;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_PEWTERGYM_BROCK_POST_BATTLE, SCRIPT_PEWTERGYM_DEFAULT,
    TEXT_PEWTERGYM_BROCK, TEXT_PEWTERGYM_BROCK_WAIT_TAKE_THIS, TEXT_PEWTERGYM_COOLTRAINER_M,
    TEXT_PEWTERGYM_GYM_GUIDE, TEXT_PEWTERGYM_RECEIVED_TM34, TEXT_PEWTERGYM_TM34_NO_ROOM};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::{TOGGLE_GYM_GUY, TOGGLE_ROUTE_22_RIVAL_1};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `BIT_BOULDERBADGE`.
const BIT_BOULDERBADGE: u8 = 0;
/// `wGymLeaderNo` for Brock.
const BROCK: u8 = 1;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPewterGymCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPewterGymCurScript], a` after the table's routine.
    StoreCurScript,
    /// `PewterGymScriptReceiveTM34`, which both the post-battle script and Brock's own text run.
    ReceiveTm34,
    WaitTakeThis,
    ReceivedTm34,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` Brock's text does once the TM34 script has returned to it.
    TextDone,
    PreBattle,
    GuideAsk,
    GuideAnswer,
    GuideAdvice,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(2) {
        rt.load_gym_leader_and_city_name("PEWTER CITY", "BROCK");
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().pewter_gym.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::PewterGymTrainerHeaders);
    if index == SCRIPT_PEWTERGYM_BROCK_POST_BATTLE {
        return brock_post_battle(rt);
    }
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `PewterGymBrockPostBattle`.
fn brock_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm34(rt)
}

/// `PewterGymScriptReceiveTM34`.
fn receive_tm34(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_PEWTERGYM_BROCK_WAIT_TAKE_THIS).then(Label::WaitTakeThis)
}

/// `PewterGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().pewter_gym.cur_script = SCRIPT_PEWTERGYM_DEFAULT;
    rt.set_cur_map_script(SCRIPT_PEWTERGYM_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_PEWTERGYM_BROCK => brock_text(rt),
        TEXT_PEWTERGYM_COOLTRAINER_M => rt.talk_to_trainer(sym::PewterGymTrainerHeader0).ret(),
        TEXT_PEWTERGYM_GYM_GUIDE => guide_text(rt),
        _ => return None,
    })
}

/// `PewterGymBrockText`.
fn brock_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_BROCK) {
        return rt.print_text(text_at(PewterGymBrockText::PreBattleText)).then(Label::PreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM34) {
        return Flow::Call(Label::ReceiveTm34.into(), Label::TextDone.into());
    }
    rt.print_text(text_at(PewterGymBrockText::PostBattleAdviceText)).ret()
}

/// `PewterGymGuideText`: the badge is what decides which of his two speeches he gives, and a no to
/// his offer only gets him to say the advice is free.
fn guide_text(rt: &mut Script) -> Flow {
    if rt.badges() & (1 << BIT_BOULDERBADGE) != 0 {
        return rt.print_text(text_at(sym::PewterGymGuidePostBattleText)).ret();
    }
    rt.print_text(text_at(sym::PewterGymGuidePreAdviceText)).then(Label::GuideAsk)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().pewter_gym.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ReceiveTm34 => receive_tm34(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::WaitTakeThis => {
            rt.set_event(EVENT_BEAT_BROCK);
            let text_id = match rt.give_item(ItemId::Tm34Bide, 1) {
                true => TEXT_PEWTERGYM_RECEIVED_TM34,
                false => TEXT_PEWTERGYM_TM34_NO_ROOM,
            };
            rt.display_text_id(text_id).then(Label::ReceivedTm34)
        }
        Label::ReceivedTm34 => {
            if rt.is_item_in_bag(ItemId::Tm34Bide) {
                rt.set_event(EVENT_GOT_TM34);
            }
            resume(rt, Label::GymVictory)
        }
        // `.gymVictory`: the guide goes, and the rival is taken off Route 22 until Giovanni puts him
        // back, since the first battle there is only offered on the way to Pewter.
        Label::GymVictory => {
            rt.set_badge(BIT_BOULDERBADGE);
            rt.hide_object(TOGGLE_GYM_GUY);
            rt.hide_object(TOGGLE_ROUTE_22_RIVAL_1);
            rt.reset_event(EVENT_1ST_ROUTE22_RIVAL_BATTLE);
            rt.reset_event(EVENT_ROUTE22_RIVAL_WANTS_BATTLE);
            rt.set_event(EVENT_BEAT_PEWTER_GYM_TRAINER_0);
            reset_scripts(rt)
        }
        Label::PreBattle => {
            rt.save_end_battle_text(sym::PewterGymBrockReceivedBoulderBadgeText);
            rt.engage_map_trainer(rt.sprite_index(), BROCK);
            rt.maps().pewter_gym.cur_script = SCRIPT_PEWTERGYM_BROCK_POST_BATTLE;
            rt.set_cur_map_script(SCRIPT_PEWTERGYM_BROCK_POST_BATTLE);
            Flow::Return
        }
        Label::GuideAsk => rt.yes_no_choice().then(Label::GuideAnswer),
        Label::GuideAnswer => {
            let words = match rt.chose_yes() {
                true => sym::PewterGymGuideBeginAdviceText,
                false => sym::PewterGymGuideFreeServiceText,
            };
            rt.print_text(text_at(words)).then(Label::GuideAdvice)
        }
        Label::GuideAdvice => rt.print_text(text_at(sym::PewterGymGuideAdviceText)).ret(),
    }
}
