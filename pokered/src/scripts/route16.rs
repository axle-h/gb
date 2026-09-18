//! `Route16_Script`: the six bikers at the top of the Cycling Road, and the Snorlax asleep across
//! the road out of Celadon.

use poke_core::symbols::pokered_events::{EVENT_BEAT_ROUTE16_SNORLAX, EVENT_FIGHT_ROUTE16_SNORLAX};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE16_DEFAULT, SCRIPT_ROUTE16_SNORLAX_POST_BATTLE,
    TEXT_ROUTE16_BIKER1, TEXT_ROUTE16_BIKER2, TEXT_ROUTE16_BIKER3, TEXT_ROUTE16_BIKER4, TEXT_ROUTE16_BIKER5,
    TEXT_ROUTE16_BIKER6, TEXT_ROUTE16_SNORLAX_RETURNED_TO_MOUNTAINS, TEXT_ROUTE16_SNORLAX_WOKE_UP};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::TOGGLE_ROUTE_16_SNORLAX;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::snorlax::{PostBattle, Snorlax};
use super::{Flow, Script};

const SNORLAX: Snorlax = Snorlax {
    beat: EVENT_BEAT_ROUTE16_SNORLAX,
    fight: EVENT_FIGHT_ROUTE16_SNORLAX,
    woke_up_text: TEXT_ROUTE16_SNORLAX_WOKE_UP,
    calmed_down_text: TEXT_ROUTE16_SNORLAX_RETURNED_TO_MOUNTAINS,
    toggle: TOGGLE_ROUTE_16_SNORLAX,
    post_battle_script: SCRIPT_ROUTE16_SNORLAX_POST_BATTLE,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute16CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute16CurScript], a` after the table's routine.
    StoreCurScript,
    WokeUp,
    CalmedDown,
    Beaten,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route16.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route16TrainerHeaders);
    match index {
        SCRIPT_ROUTE16_DEFAULT => default_script(rt),
        SCRIPT_ROUTE16_SNORLAX_POST_BATTLE => post_battle(rt),
        _ => rt.trainer_script(index).then(Label::StoreCurScript),
    }
}

/// `Route16DefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    match SNORLAX.woken(rt) {
        Some(then) => then.then(Label::WokeUp),
        None => rt.trainer_script(0).then(Label::StoreCurScript),
    }
}

/// `Route16SnorlaxPostBattleScript`.
fn post_battle(rt: &mut Script) -> Flow {
    match SNORLAX.post_battle(rt) {
        PostBattle::Lost => reset_scripts(rt),
        PostBattle::CalmedDown(then) => then.then(Label::CalmedDown),
        PostBattle::Caught => SNORLAX.beaten(rt).then(Label::Beaten),
    }
}

/// `Route16ResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_ROUTE16_DEFAULT);
    Flow::Return
}

fn set_script(rt: &mut Script, index: u8) {
    rt.maps().route16.cur_script = index;
    rt.set_cur_map_script(index);
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE16_BIKER1 => sym::Route16TrainerHeader0,
        TEXT_ROUTE16_BIKER2 => sym::Route16TrainerHeader1,
        TEXT_ROUTE16_BIKER3 => sym::Route16TrainerHeader2,
        TEXT_ROUTE16_BIKER4 => sym::Route16TrainerHeader3,
        TEXT_ROUTE16_BIKER5 => sym::Route16TrainerHeader4,
        TEXT_ROUTE16_BIKER6 => sym::Route16TrainerHeader5,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route16.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::WokeUp => {
            let next = SNORLAX.wake_up(rt);
            // Route 12 leaves the sprite where it was until something else redraws it; this road
            // takes it off the screen itself.
            rt.update_sprites();
            set_script(rt, next);
            Flow::Return
        }
        Label::CalmedDown => SNORLAX.beaten(rt).then(Label::Beaten),
        Label::Beaten => {
            set_script(rt, SCRIPT_ROUTE16_DEFAULT);
            Flow::Return
        }
    }
}
