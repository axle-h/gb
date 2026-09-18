//! `SilphCo4F_Script`: two Rockets and a scientist, the floor's two card key doors, and the Silph
//! worker hiding from them.

use poke_core::symbols::pokered_events::{EVENT_SILPH_CO_4_UNLOCKED_DOOR1, EVENT_SILPH_CO_4_UNLOCKED_DOOR2};
use poke_core::symbols::pokered_local_labels::SilphCo4FSilphWorkerMText as worker;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO4F_ROCKET1, TEXT_SILPHCO4F_ROCKET2,
    TEXT_SILPHCO4F_SCIENTIST, TEXT_SILPHCO4F_SILPH_WORKER_M};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::silph_co::beat_giovanni_print_de_or_print_hl as print_held_or_freed;
use super::{Flow, Script};

/// The block a card key door is drawn as while it is still shut.
const CLOSED_DOOR: u8 = 0x54;
/// `SilphCo4FGateCallbackScript.GateCoordinates`, in blocks, and the event each gate has.
const GATES: [(u8, u8); 2] = [(2, 6), (6, 4)];
const DOORS: [u16; 2] = [EVENT_SILPH_CO_4_UNLOCKED_DOOR1, EVENT_SILPH_CO_4_UNLOCKED_DOOR2];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo4FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo4FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo4FGateCallbackScript`.
    super::silph_co::gate_callback(rt, &GATES, &DOORS, CLOSED_DOOR);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_4f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo4TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        // `SilphCo4FSilphWorkerMText`.
        TEXT_SILPHCO4F_SILPH_WORKER_M => {
            return Some(print_held_or_freed(rt, worker::ImHidingText, worker::TeamRocketIsGoneText));
        }
        TEXT_SILPHCO4F_ROCKET1 => sym::SilphCo4TrainerHeader0,
        TEXT_SILPHCO4F_SCIENTIST => sym::SilphCo4TrainerHeader1,
        TEXT_SILPHCO4F_ROCKET2 => sym::SilphCo4TrainerHeader2,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_4f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
