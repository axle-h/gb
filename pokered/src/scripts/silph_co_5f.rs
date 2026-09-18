//! `SilphCo5F_Script`: two Rockets, a scientist and a rocker, the three card key doors this floor
//! draws shut, and the Silph worker who recognises the player.

use poke_core::symbols::pokered_events::{EVENT_SILPH_CO_5_UNLOCKED_DOOR1, EVENT_SILPH_CO_5_UNLOCKED_DOOR2,
    EVENT_SILPH_CO_5_UNLOCKED_DOOR3};
use poke_core::symbols::pokered_local_labels::SilphCo5FSilphWorkerMText as worker;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO5F_ROCKER, TEXT_SILPHCO5F_ROCKET1,
    TEXT_SILPHCO5F_ROCKET2, TEXT_SILPHCO5F_SCIENTIST, TEXT_SILPHCO5F_SILPH_WORKER_M};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::silph_co::beat_giovanni_print_de_or_print_hl as print_held_or_freed;
use super::{Flow, Script};

/// The block a card key door is drawn as while it is still shut.
const CLOSED_DOOR: u8 = 0x5F;
/// `SilphCo5FGateCallbackScript.GateCoordinates`, in blocks, and the event each gate has.
const GATES: [(u8, u8); 3] = [(3, 2), (3, 6), (7, 5)];
const DOORS: [u16; 3] = [EVENT_SILPH_CO_5_UNLOCKED_DOOR1, EVENT_SILPH_CO_5_UNLOCKED_DOOR2,
    EVENT_SILPH_CO_5_UNLOCKED_DOOR3];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo5FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo5FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo5FGateCallbackScript`.
    super::silph_co::gate_callback(rt, &GATES, &DOORS, CLOSED_DOOR);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_5f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo5TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        // `SilphCo5FSilphWorkerMText`.
        TEXT_SILPHCO5F_SILPH_WORKER_M => {
            return Some(print_held_or_freed(rt, worker::ThatsYouRightText, worker::YoureOurHeroText));
        }
        TEXT_SILPHCO5F_ROCKET1 => sym::SilphCo5TrainerHeader0,
        TEXT_SILPHCO5F_SCIENTIST => sym::SilphCo5TrainerHeader1,
        TEXT_SILPHCO5F_ROCKER => sym::SilphCo5TrainerHeader2,
        TEXT_SILPHCO5F_ROCKET2 => sym::SilphCo5TrainerHeader3,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_5f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
