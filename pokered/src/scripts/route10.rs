//! `Route10_Script`: six trainers either side of Rock Tunnel's north mouth, and nothing else.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute10CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute10CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route10.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route10TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE10_SUPER_NERD1 => sym::Route10TrainerHeader0,
        TEXT_ROUTE10_HIKER1 => sym::Route10TrainerHeader1,
        TEXT_ROUTE10_SUPER_NERD2 => sym::Route10TrainerHeader2,
        TEXT_ROUTE10_COOLTRAINER_F1 => sym::Route10TrainerHeader3,
        TEXT_ROUTE10_HIKER2 => sym::Route10TrainerHeader4,
        TEXT_ROUTE10_COOLTRAINER_F2 => sym::Route10TrainerHeader5,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route10.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
