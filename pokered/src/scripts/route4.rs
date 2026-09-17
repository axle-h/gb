//! `Route4_Script`: the one trainer on the ledge above Cerulean. Her twin at the Mt. Moon end has
//! no header and only talks.

use poke_core::symbols::pokered_map_scripts::TEXT_ROUTE4_COOLTRAINER_F2;
use poke_core::symbols::pokered_symbols::{Route4TrainerHeader0, Route4TrainerHeaders};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute4CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute4CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route4.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, Route4TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_ROUTE4_COOLTRAINER_F2 => Some(rt.talk_to_trainer(Route4TrainerHeader0).ret()),
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route4.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
