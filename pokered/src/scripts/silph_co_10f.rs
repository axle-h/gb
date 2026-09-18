//! `SilphCo10F_Script`: a Rocket and a scientist, one card key door, and the Silph worker whose line
//! turns once Giovanni has been beaten.

use poke_core::symbols::pokered_events::EVENT_SILPH_CO_10_UNLOCKED_DOOR;
use poke_core::symbols::pokered_local_labels::SilphCo10FSilphWorkerFText as worker;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO10F_ROCKET, TEXT_SILPHCO10F_SCIENTIST,
    TEXT_SILPHCO10F_SILPH_WORKER_F};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::silph_co::beat_giovanni_print_de_or_print_hl as print_held_or_freed;
use super::{Flow, Script};

/// The block the card key door is drawn as while it is still shut.
const CLOSED_DOOR: u8 = 0x54;
/// `SilphCo10FGateCallbackScript.GateCoordinates`, in blocks, and the event that remembers it.
const GATES: [(u8, u8); 1] = [(5, 4)];
const DOORS: [u16; 1] = [EVENT_SILPH_CO_10_UNLOCKED_DOOR];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo10FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo10FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo10FGateCallbackScript`.
    super::silph_co::gate_callback(rt, &GATES, &DOORS, CLOSED_DOOR);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_10f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo10TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SILPHCO10F_SILPH_WORKER_F => {
            return Some(print_held_or_freed(rt, worker::ImScaredText, worker::QuietAboutMyCryingText));
        }
        TEXT_SILPHCO10F_ROCKET => sym::SilphCo10TrainerHeader0,
        TEXT_SILPHCO10F_SCIENTIST => sym::SilphCo10TrainerHeader1,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_10f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
