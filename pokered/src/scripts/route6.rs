//! `Route6_Script`: six trainers on the path down to Vermilion, and nothing else.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE6_COOLTRAINER_F1, TEXT_ROUTE6_COOLTRAINER_F2,
    TEXT_ROUTE6_COOLTRAINER_M1, TEXT_ROUTE6_COOLTRAINER_M2, TEXT_ROUTE6_YOUNGSTER1, TEXT_ROUTE6_YOUNGSTER2};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute6CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute6CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route6.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route6TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE6_COOLTRAINER_M1 => sym::Route6TrainerHeader0,
        TEXT_ROUTE6_COOLTRAINER_F1 => sym::Route6TrainerHeader1,
        TEXT_ROUTE6_YOUNGSTER1 => sym::Route6TrainerHeader2,
        TEXT_ROUTE6_COOLTRAINER_M2 => sym::Route6TrainerHeader3,
        TEXT_ROUTE6_COOLTRAINER_F2 => sym::Route6TrainerHeader4,
        TEXT_ROUTE6_YOUNGSTER2 => sym::Route6TrainerHeader5,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route6.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
