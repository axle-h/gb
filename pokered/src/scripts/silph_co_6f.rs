//! `SilphCo6F_Script`: two Rockets and a scientist, one card key door, and five Silph workers whose
//! lines all turn once Giovanni has been beaten.

use poke_core::symbols::pokered_events::EVENT_SILPH_CO_6_UNLOCKED_DOOR;
use poke_core::symbols::pokered_local_labels::{SilphCo6FSilphWorkerF1Text as worker_f1,
    SilphCo6FSilphWorkerF2Text as worker_f2, SilphCo6FSilphWorkerM1Text as worker_m1,
    SilphCo6FSilphWorkerM2Text as worker_m2, SilphCo6FSilphWorkerM3Text as worker_m3};
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO6F_ROCKET1, TEXT_SILPHCO6F_ROCKET2,
    TEXT_SILPHCO6F_SCIENTIST, TEXT_SILPHCO6F_SILPH_WORKER_F1, TEXT_SILPHCO6F_SILPH_WORKER_F2,
    TEXT_SILPHCO6F_SILPH_WORKER_M1, TEXT_SILPHCO6F_SILPH_WORKER_M2, TEXT_SILPHCO6F_SILPH_WORKER_M3};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::silph_co::beat_giovanni_print_de_or_print_hl as print_held_or_freed;
use super::{Flow, Script};

/// The block the card key door is drawn as while it is still shut.
const CLOSED_DOOR: u8 = 0x5F;
/// `SilphCo6F_GateCallbackScript.GateCoordinates`, in blocks, and the event that remembers it.
const GATES: [(u8, u8); 1] = [(2, 6)];
const DOORS: [u16; 1] = [EVENT_SILPH_CO_6_UNLOCKED_DOOR];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo6FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo6FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo6F_GateCallbackScript`.
    super::silph_co::gate_callback(rt, &GATES, &DOORS, CLOSED_DOOR);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_6f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo6TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let (held, freed) = match text_id {
        TEXT_SILPHCO6F_SILPH_WORKER_M1 => (worker_m1::TookOverTheBuildingText, worker_m1::BackToWorkText),
        TEXT_SILPHCO6F_SILPH_WORKER_M2 => (worker_m2::HelpMePleaseText, worker_m2::WeGotEngagedText),
        TEXT_SILPHCO6F_SILPH_WORKER_F1 => (worker_f1::SuchACowardText, worker_f1::HaveToMarryHimText),
        TEXT_SILPHCO6F_SILPH_WORKER_F2 => (worker_f2::TeamRocketConquerWorldText, worker_f2::TeamRocketRanText),
        TEXT_SILPHCO6F_SILPH_WORKER_M3 => (worker_m3::TargetedSilphText, worker_m3::WorkForSilphText),
        TEXT_SILPHCO6F_ROCKET1 => return Some(rt.talk_to_trainer(sym::SilphCo6TrainerHeader0).ret()),
        TEXT_SILPHCO6F_SCIENTIST => return Some(rt.talk_to_trainer(sym::SilphCo6TrainerHeader1).ret()),
        TEXT_SILPHCO6F_ROCKET2 => return Some(rt.talk_to_trainer(sym::SilphCo6TrainerHeader2).ret()),
        _ => return None,
    };
    Some(print_held_or_freed(rt, held, freed))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_6f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
