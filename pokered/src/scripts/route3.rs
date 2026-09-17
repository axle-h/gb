//! `Route3_Script`: eight trainers on the climb to Mt. Moon.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{Route3TrainerHeader0, Route3TrainerHeader1, Route3TrainerHeader2,
    Route3TrainerHeader3, Route3TrainerHeader4, Route3TrainerHeader5, Route3TrainerHeader6, Route3TrainerHeader7,
    Route3TrainerHeaders};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute3CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute3CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route3.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, Route3TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE3_YOUNGSTER1 => Route3TrainerHeader0,
        TEXT_ROUTE3_YOUNGSTER2 => Route3TrainerHeader1,
        TEXT_ROUTE3_COOLTRAINER_F1 => Route3TrainerHeader2,
        TEXT_ROUTE3_YOUNGSTER3 => Route3TrainerHeader3,
        TEXT_ROUTE3_COOLTRAINER_F2 => Route3TrainerHeader4,
        TEXT_ROUTE3_YOUNGSTER4 => Route3TrainerHeader5,
        TEXT_ROUTE3_YOUNGSTER5 => Route3TrainerHeader6,
        TEXT_ROUTE3_COOLTRAINER_F3 => Route3TrainerHeader7,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route3.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
