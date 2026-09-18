//! `Route19_Script`: the ten swimmers in the water south of Fuchsia, and nothing else.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE19_COOLTRAINER_M1, TEXT_ROUTE19_COOLTRAINER_M2,
    TEXT_ROUTE19_SWIMMER1, TEXT_ROUTE19_SWIMMER2, TEXT_ROUTE19_SWIMMER3, TEXT_ROUTE19_SWIMMER4,
    TEXT_ROUTE19_SWIMMER5, TEXT_ROUTE19_SWIMMER6, TEXT_ROUTE19_SWIMMER7, TEXT_ROUTE19_SWIMMER8};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute19CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute19CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route19.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route19TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE19_COOLTRAINER_M1 => sym::Route19TrainerHeader0,
        TEXT_ROUTE19_COOLTRAINER_M2 => sym::Route19TrainerHeader1,
        TEXT_ROUTE19_SWIMMER1 => sym::Route19TrainerHeader2,
        TEXT_ROUTE19_SWIMMER2 => sym::Route19TrainerHeader3,
        TEXT_ROUTE19_SWIMMER3 => sym::Route19TrainerHeader4,
        TEXT_ROUTE19_SWIMMER4 => sym::Route19TrainerHeader5,
        TEXT_ROUTE19_SWIMMER5 => sym::Route19TrainerHeader6,
        TEXT_ROUTE19_SWIMMER6 => sym::Route19TrainerHeader7,
        TEXT_ROUTE19_SWIMMER7 => sym::Route19TrainerHeader8,
        TEXT_ROUTE19_SWIMMER8 => sym::Route19TrainerHeader9,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route19.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
