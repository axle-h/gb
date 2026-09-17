//! `Route25_Script`: nine trainers on the path to Bill's, and the pass that swaps Bill's sprites
//! over once he is himself again.

use poke_core::symbols::pokered_events::{EVENT_BILL_SAID_USE_CELL_SEPARATOR, EVENT_GOT_SS_TICKET,
    EVENT_LEFT_BILLS_HOUSE_AFTER_HELPING, EVENT_MET_BILL_2};
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{Route25TrainerHeader0, Route25TrainerHeader1, Route25TrainerHeader2,
    Route25TrainerHeader3, Route25TrainerHeader4, Route25TrainerHeader5, Route25TrainerHeader6,
    Route25TrainerHeader7, Route25TrainerHeader8, Route25TrainerHeaders};
use poke_core::symbols::pokered_toggles::{TOGGLE_BILL_1, TOGGLE_BILL_2, TOGGLE_BILL_POKEMON,
    TOGGLE_NUGGET_BRIDGE_GUY};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute25CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute25CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    toggle_bills(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route25.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, Route25TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `Route25ToggleBillsScript`, run on the first pass after the map loads: the route carries Bill's
/// own sprites, so what the player did in his house a moment ago is put on the map out here.
fn toggle_bills(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(2) || rt.check_event(EVENT_LEFT_BILLS_HOUSE_AFTER_HELPING) {
        return;
    }
    if !rt.check_event(EVENT_MET_BILL_2) {
        rt.reset_event(EVENT_BILL_SAID_USE_CELL_SEPARATOR);
        rt.show_object(TOGGLE_BILL_POKEMON);
        return;
    }
    if !rt.check_event(EVENT_GOT_SS_TICKET) {
        return;
    }
    rt.set_event(EVENT_LEFT_BILLS_HOUSE_AFTER_HELPING);
    rt.hide_object(TOGGLE_NUGGET_BRIDGE_GUY);
    rt.hide_object(TOGGLE_BILL_1);
    rt.show_object(TOGGLE_BILL_2);
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE25_YOUNGSTER1 => Route25TrainerHeader0,
        TEXT_ROUTE25_YOUNGSTER2 => Route25TrainerHeader1,
        TEXT_ROUTE25_COOLTRAINER_M => Route25TrainerHeader2,
        TEXT_ROUTE25_COOLTRAINER_F1 => Route25TrainerHeader3,
        TEXT_ROUTE25_YOUNGSTER3 => Route25TrainerHeader4,
        TEXT_ROUTE25_COOLTRAINER_F2 => Route25TrainerHeader5,
        TEXT_ROUTE25_HIKER1 => Route25TrainerHeader6,
        TEXT_ROUTE25_HIKER2 => Route25TrainerHeader7,
        TEXT_ROUTE25_HIKER3 => Route25TrainerHeader8,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route25.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
