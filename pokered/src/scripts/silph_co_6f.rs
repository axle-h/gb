//! `SilphCo6F_Script`: two Rockets and a scientist, one card key door, and five Silph workers whose
//! lines all turn once Giovanni has been beaten.

use poke_core::tables::trainers;
use poke_core::symbols::pokered_events::EVENT_SILPH_CO_6_UNLOCKED_DOOR;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO6F_ROCKET1, TEXT_SILPHCO6F_ROCKET2,
    TEXT_SILPHCO6F_SCIENTIST, TEXT_SILPHCO6F_SILPH_WORKER_F1, TEXT_SILPHCO6F_SILPH_WORKER_F2,
    TEXT_SILPHCO6F_SILPH_WORKER_M1, TEXT_SILPHCO6F_SILPH_WORKER_M2, TEXT_SILPHCO6F_SILPH_WORKER_M3};
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
    let index = rt.execute_cur_map_script_in_table(index, trainers::SilphCo6TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let (held, freed) = match text_id {
        TEXT_SILPHCO6F_SILPH_WORKER_M1 => ("SilphCo6FSilphWorkerM1Text.TookOverTheBuildingText", "SilphCo6FSilphWorkerM1Text.BackToWorkText"),
        TEXT_SILPHCO6F_SILPH_WORKER_M2 => ("SilphCo6FSilphWorkerM2Text.HelpMePleaseText", "SilphCo6FSilphWorkerM2Text.WeGotEngagedText"),
        TEXT_SILPHCO6F_SILPH_WORKER_F1 => ("SilphCo6FSilphWorkerF1Text.SuchACowardText", "SilphCo6FSilphWorkerF1Text.HaveToMarryHimText"),
        TEXT_SILPHCO6F_SILPH_WORKER_F2 => ("SilphCo6FSilphWorkerF2Text.TeamRocketConquerWorldText", "SilphCo6FSilphWorkerF2Text.TeamRocketRanText"),
        TEXT_SILPHCO6F_SILPH_WORKER_M3 => ("SilphCo6FSilphWorkerM3Text.TargetedSilphText", "SilphCo6FSilphWorkerM3Text.WorkForSilphText"),
        TEXT_SILPHCO6F_ROCKET1 => return Some(rt.talk_to_trainer(trainers::SilphCo6TrainerHeader0).ret()),
        TEXT_SILPHCO6F_SCIENTIST => return Some(rt.talk_to_trainer(trainers::SilphCo6TrainerHeader1).ret()),
        TEXT_SILPHCO6F_ROCKET2 => return Some(rt.talk_to_trainer(trainers::SilphCo6TrainerHeader2).ret()),
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
