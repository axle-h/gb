//! `Route9_Script`: nine trainers on the road to Rock Tunnel, and nothing else.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute9CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute9CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route9.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route9TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE9_COOLTRAINER_F1 => sym::Route9TrainerHeader0,
        TEXT_ROUTE9_COOLTRAINER_M1 => sym::Route9TrainerHeader1,
        TEXT_ROUTE9_COOLTRAINER_M2 => sym::Route9TrainerHeader2,
        TEXT_ROUTE9_COOLTRAINER_F2 => sym::Route9TrainerHeader3,
        TEXT_ROUTE9_HIKER1 => sym::Route9TrainerHeader4,
        TEXT_ROUTE9_HIKER2 => sym::Route9TrainerHeader5,
        TEXT_ROUTE9_YOUNGSTER1 => sym::Route9TrainerHeader6,
        TEXT_ROUTE9_HIKER3 => sym::Route9TrainerHeader7,
        TEXT_ROUTE9_YOUNGSTER2 => sym::Route9TrainerHeader8,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route9.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
