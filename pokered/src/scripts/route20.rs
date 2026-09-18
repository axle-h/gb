//! `Route20_Script`: the ten swimmers along the water past the Seafoam Islands, and the boulder
//! objects the islands leave behind, which this road puts back where they started.

use poke_core::symbols::pokered_events::{EVENT_IN_SEAFOAM_ISLANDS, EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE,
    EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE, EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE};
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE20_COOLTRAINER_M, TEXT_ROUTE20_SWIMMER1,
    TEXT_ROUTE20_SWIMMER2, TEXT_ROUTE20_SWIMMER3, TEXT_ROUTE20_SWIMMER4, TEXT_ROUTE20_SWIMMER5,
    TEXT_ROUTE20_SWIMMER6, TEXT_ROUTE20_SWIMMER7, TEXT_ROUTE20_SWIMMER8, TEXT_ROUTE20_SWIMMER9};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::{TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_2, TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1,
    TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_2, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_4, TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_1,
    TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_2};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `Route20BoulderScript.ToggleableObjectIDs`: every boulder object a shove down a hole showed,
/// between the top pair and where they came to rest.
const MOVED_BOULDERS: [u16; 6] = [
    TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_1,
    TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1,
    TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3,
    TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_4,
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute20CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute20CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_event(EVENT_IN_SEAFOAM_ISLANDS) {
        boulder_script(rt);
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route20.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route20TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `Route20BoulderScript`, run on the first pass after leaving the islands: a completed pair of
/// holes puts its boulders back at the top of the puzzle and takes away the copies below. The
/// events stay set, so the currents stay slowed and the walk back through is only scenery.
fn boulder_script(rt: &mut Script) {
    if rt.check_event(EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE) && rt.check_event(EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE) {
        rt.show_object(TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_1);
        rt.show_object(TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_2);
        for boulder in MOVED_BOULDERS {
            rt.hide_object(boulder);
        }
    }
    if rt.check_event(EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE) && rt.check_event(EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE) {
        rt.show_object(TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_1);
        rt.show_object(TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_2);
        rt.hide_object(TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_1);
        rt.hide_object(TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_2);
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE20_SWIMMER1 => sym::Route20TrainerHeader0,
        TEXT_ROUTE20_SWIMMER2 => sym::Route20TrainerHeader1,
        TEXT_ROUTE20_SWIMMER3 => sym::Route20TrainerHeader2,
        TEXT_ROUTE20_SWIMMER4 => sym::Route20TrainerHeader3,
        TEXT_ROUTE20_SWIMMER5 => sym::Route20TrainerHeader4,
        TEXT_ROUTE20_SWIMMER6 => sym::Route20TrainerHeader5,
        TEXT_ROUTE20_COOLTRAINER_M => sym::Route20TrainerHeader6,
        TEXT_ROUTE20_SWIMMER7 => sym::Route20TrainerHeader7,
        TEXT_ROUTE20_SWIMMER8 => sym::Route20TrainerHeader8,
        TEXT_ROUTE20_SWIMMER9 => sym::Route20TrainerHeader9,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route20.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
