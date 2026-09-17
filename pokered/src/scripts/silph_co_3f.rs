//! `SilphCo3F_Script`: a Rocket and a scientist, the floor's two card key doors, and the worker
//! whose line changes once Giovanni has been beaten.

use poke_core::symbols::pokered_events::{EVENT_BEAT_SILPH_CO_GIOVANNI, EVENT_SILPH_CO_3_UNLOCKED_DOOR1,
    EVENT_SILPH_CO_3_UNLOCKED_DOOR2};
use poke_core::symbols::pokered_local_labels::SilphCo3FSilphWorkerMText as worker;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO3F_ROCKET, TEXT_SILPHCO3F_SCIENTIST,
    TEXT_SILPHCO3F_SILPH_WORKER_M};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// The block a card key door is drawn as while it is still shut. The third floor's is not the
/// second floor's, because the two floors are tiled differently.
const CLOSED_DOOR: u8 = 0x5F;
/// `SilphCo3FGateCallbackScript.GateCoordinates`, in blocks.
const GATES: [(u8, u8); 2] = [(4, 4), (8, 4)];

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
    gate_callback(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_3f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo3TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `SilphCo3FGateCallbackScript`.
fn gate_callback(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    match super::silph_co_2f::unlocked_door(rt, &GATES) {
        0 => {}
        1 => rt.set_event(EVENT_SILPH_CO_3_UNLOCKED_DOOR1),
        _ => rt.set_event(EVENT_SILPH_CO_3_UNLOCKED_DOOR2),
    }
    for (event, (x, y)) in [(EVENT_SILPH_CO_3_UNLOCKED_DOOR1, GATES[0]), (EVENT_SILPH_CO_3_UNLOCKED_DOOR2, GATES[1])] {
        if !rt.check_event(event) {
            rt.replace_tile_block(x, y, CLOSED_DOOR);
        }
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SILPHCO3F_SILPH_WORKER_M => {
            let said = match rt.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
                true => worker::YouSavedUsText,
                false => worker::WhatShouldIDoText,
            };
            return Some(rt.print_text(text_at(said)).ret());
        }
        TEXT_SILPHCO3F_ROCKET => sym::SilphCo3TrainerHeader0,
        TEXT_SILPHCO3F_SCIENTIST => sym::SilphCo3TrainerHeader1,
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
