//! `SilphCo8F_Script`: two Rockets and a scientist, one card key door, and the Silph worker whose
//! line turns once Giovanni has been beaten.

use poke_core::symbols::pokered_events::EVENT_SILPH_CO_8_UNLOCKED_DOOR;
use poke_core::symbols::pokered_local_labels::SilphCo8FSilphWorkerMText as worker;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO8F_ROCKET1, TEXT_SILPHCO8F_ROCKET2,
    TEXT_SILPHCO8F_SCIENTIST, TEXT_SILPHCO8F_SILPH_WORKER_M};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::silph_co::beat_giovanni_print_de_or_print_hl as print_held_or_freed;
use super::{Flow, Script};

/// The block the card key door is drawn as while it is still shut.
const CLOSED_DOOR: u8 = 0x5F;
/// `SilphCo8FGateCallbackScript.GateCoordinates`, in blocks, and the event that remembers it.
const GATES: [(u8, u8); 1] = [(3, 4)];
const DOORS: [u16; 1] = [EVENT_SILPH_CO_8_UNLOCKED_DOOR];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo8FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo8FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo8FGateCallbackScript`.
    super::silph_co::gate_callback(rt, &GATES, &DOORS, CLOSED_DOOR);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_8f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo8TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SILPHCO8F_SILPH_WORKER_M => {
            return Some(print_held_or_freed(rt, worker::SilphIsFinishedText, worker::ThanksForSavingUsText));
        }
        TEXT_SILPHCO8F_ROCKET1 => sym::SilphCo8TrainerHeader0,
        TEXT_SILPHCO8F_SCIENTIST => sym::SilphCo8TrainerHeader1,
        TEXT_SILPHCO8F_ROCKET2 => sym::SilphCo8TrainerHeader2,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_8f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
