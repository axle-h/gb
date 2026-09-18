//! `Route18_Script`: three bird keepers on the road below the Cycling Road, and nothing else.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE18_COOLTRAINER_M1, TEXT_ROUTE18_COOLTRAINER_M2,
    TEXT_ROUTE18_COOLTRAINER_M3};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute18CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute18CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route18.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route18TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE18_COOLTRAINER_M1 => sym::Route18TrainerHeader0,
        TEXT_ROUTE18_COOLTRAINER_M2 => sym::Route18TrainerHeader1,
        TEXT_ROUTE18_COOLTRAINER_M3 => sym::Route18TrainerHeader2,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route18.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
