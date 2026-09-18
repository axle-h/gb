//! `Route12_Script`: seven fishermen and trainers along the water, and the Snorlax asleep across the
//! road south of Lavender.

use poke_core::symbols::pokered_events::{EVENT_BEAT_ROUTE12_SNORLAX, EVENT_FIGHT_ROUTE12_SNORLAX};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE12_DEFAULT, SCRIPT_ROUTE12_SNORLAX_POST_BATTLE,
    TEXT_ROUTE12_COOLTRAINER_M, TEXT_ROUTE12_FISHER1, TEXT_ROUTE12_FISHER2, TEXT_ROUTE12_FISHER3,
    TEXT_ROUTE12_FISHER4, TEXT_ROUTE12_FISHER5, TEXT_ROUTE12_SNORLAX_CALMED_DOWN, TEXT_ROUTE12_SNORLAX_WOKE_UP,
    TEXT_ROUTE12_SUPER_NERD};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::TOGGLE_ROUTE_12_SNORLAX;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::snorlax::{PostBattle, Snorlax};
use super::{Flow, Script};

const SNORLAX: Snorlax = Snorlax {
    beat: EVENT_BEAT_ROUTE12_SNORLAX,
    fight: EVENT_FIGHT_ROUTE12_SNORLAX,
    woke_up_text: TEXT_ROUTE12_SNORLAX_WOKE_UP,
    calmed_down_text: TEXT_ROUTE12_SNORLAX_CALMED_DOWN,
    toggle: TOGGLE_ROUTE_12_SNORLAX,
    post_battle_script: SCRIPT_ROUTE12_SNORLAX_POST_BATTLE,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute12CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute12CurScript], a` after the table's routine.
    StoreCurScript,
    WokeUp,
    CalmedDown,
    Beaten,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route12.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route12TrainerHeaders);
    match index {
        SCRIPT_ROUTE12_DEFAULT => default_script(rt),
        SCRIPT_ROUTE12_SNORLAX_POST_BATTLE => post_battle(rt),
        _ => rt.trainer_script(index).then(Label::StoreCurScript),
    }
}

/// `Route12DefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    match SNORLAX.woken(rt) {
        Some(then) => then.then(Label::WokeUp),
        None => rt.trainer_script(0).then(Label::StoreCurScript),
    }
}

/// `Route12SnorlaxPostBattleScript`.
fn post_battle(rt: &mut Script) -> Flow {
    match SNORLAX.post_battle(rt) {
        PostBattle::Lost => reset_scripts(rt),
        PostBattle::CalmedDown(then) => then.then(Label::CalmedDown),
        PostBattle::Caught => SNORLAX.beaten(rt).then(Label::Beaten),
    }
}

/// `Route12ResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_ROUTE12_DEFAULT);
    Flow::Return
}

fn set_script(rt: &mut Script, index: u8) {
    rt.maps().route12.cur_script = index;
    rt.set_cur_map_script(index);
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE12_FISHER1 => sym::Route12TrainerHeader0,
        TEXT_ROUTE12_FISHER2 => sym::Route12TrainerHeader1,
        TEXT_ROUTE12_COOLTRAINER_M => sym::Route12TrainerHeader2,
        TEXT_ROUTE12_SUPER_NERD => sym::Route12TrainerHeader3,
        TEXT_ROUTE12_FISHER3 => sym::Route12TrainerHeader4,
        TEXT_ROUTE12_FISHER4 => sym::Route12TrainerHeader5,
        TEXT_ROUTE12_FISHER5 => sym::Route12TrainerHeader6,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route12.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::WokeUp => {
            let next = SNORLAX.wake_up(rt);
            set_script(rt, next);
            Flow::Return
        }
        Label::CalmedDown => SNORLAX.beaten(rt).then(Label::Beaten),
        Label::Beaten => {
            set_script(rt, SCRIPT_ROUTE12_DEFAULT);
            Flow::Return
        }
    }
}
