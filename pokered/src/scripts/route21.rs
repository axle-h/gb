//! `Route21_Script`: the nine fishermen and swimmers on the sea between Pallet Town and Cinnabar,
//! and nothing else.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE21_FISHER1, TEXT_ROUTE21_FISHER2, TEXT_ROUTE21_FISHER3,
    TEXT_ROUTE21_FISHER4, TEXT_ROUTE21_SWIMMER1, TEXT_ROUTE21_SWIMMER2, TEXT_ROUTE21_SWIMMER3,
    TEXT_ROUTE21_SWIMMER4, TEXT_ROUTE21_SWIMMER5};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute21CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute21CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route21.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route21TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE21_FISHER1 => sym::Route21TrainerHeader0,
        TEXT_ROUTE21_FISHER2 => sym::Route21TrainerHeader1,
        TEXT_ROUTE21_SWIMMER1 => sym::Route21TrainerHeader2,
        TEXT_ROUTE21_SWIMMER2 => sym::Route21TrainerHeader3,
        TEXT_ROUTE21_SWIMMER3 => sym::Route21TrainerHeader4,
        TEXT_ROUTE21_SWIMMER4 => sym::Route21TrainerHeader5,
        TEXT_ROUTE21_SWIMMER5 => sym::Route21TrainerHeader6,
        TEXT_ROUTE21_FISHER3 => sym::Route21TrainerHeader7,
        TEXT_ROUTE21_FISHER4 => sym::Route21TrainerHeader8,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route21.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
