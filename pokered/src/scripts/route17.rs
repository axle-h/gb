//! `Route17_Script`: ten bikers down the length of the Cycling Road, and nothing else.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE17_BIKER1, TEXT_ROUTE17_BIKER10, TEXT_ROUTE17_BIKER2,
    TEXT_ROUTE17_BIKER3, TEXT_ROUTE17_BIKER4, TEXT_ROUTE17_BIKER5, TEXT_ROUTE17_BIKER6, TEXT_ROUTE17_BIKER7,
    TEXT_ROUTE17_BIKER8, TEXT_ROUTE17_BIKER9};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute17CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute17CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route17.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route17TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE17_BIKER1 => sym::Route17TrainerHeader0,
        TEXT_ROUTE17_BIKER2 => sym::Route17TrainerHeader1,
        TEXT_ROUTE17_BIKER3 => sym::Route17TrainerHeader2,
        TEXT_ROUTE17_BIKER4 => sym::Route17TrainerHeader3,
        TEXT_ROUTE17_BIKER5 => sym::Route17TrainerHeader4,
        TEXT_ROUTE17_BIKER6 => sym::Route17TrainerHeader5,
        TEXT_ROUTE17_BIKER7 => sym::Route17TrainerHeader6,
        TEXT_ROUTE17_BIKER8 => sym::Route17TrainerHeader7,
        TEXT_ROUTE17_BIKER9 => sym::Route17TrainerHeader8,
        TEXT_ROUTE17_BIKER10 => sym::Route17TrainerHeader9,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route17.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
