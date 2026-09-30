//! `SilphCo3F_Script`: a Rocket and a scientist, the floor's two card key doors, and the worker
//! whose line changes once Giovanni has been beaten.

use poke_core::tables::trainers;
use poke_core::symbols::pokered_events::{EVENT_BEAT_SILPH_CO_GIOVANNI, EVENT_SILPH_CO_3_UNLOCKED_DOOR1,
    EVENT_SILPH_CO_3_UNLOCKED_DOOR2};
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO3F_ROCKET, TEXT_SILPHCO3F_SCIENTIST,
    TEXT_SILPHCO3F_SILPH_WORKER_M};
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

/// The block a card key door is drawn as while it is still shut. The third floor's is not the
/// second floor's, because the two floors are tiled differently.
const CLOSED_DOOR: u8 = 0x5F;
/// `SilphCo3FGateCallbackScript.GateCoordinates`, in blocks, and the event each gate has.
const GATES: [(u8, u8); 2] = [(4, 4), (8, 4)];
const DOORS: [u16; 2] = [EVENT_SILPH_CO_3_UNLOCKED_DOOR1, EVENT_SILPH_CO_3_UNLOCKED_DOOR2];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo3FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo3FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo3FGateCallbackScript`.
    super::silph_co::gate_callback(rt, &GATES, &DOORS, CLOSED_DOOR);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_3f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, trainers::SilphCo3TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SILPHCO3F_SILPH_WORKER_M => {
            let said = match rt.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
                true => "SilphCo3FSilphWorkerMText.YouSavedUsText",
                false => "SilphCo3FSilphWorkerMText.WhatShouldIDoText",
            };
            return Some(rt.print_text(text_named(said)).ret());
        }
        TEXT_SILPHCO3F_ROCKET => trainers::SilphCo3TrainerHeader0,
        TEXT_SILPHCO3F_SCIENTIST => trainers::SilphCo3TrainerHeader1,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_3f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
