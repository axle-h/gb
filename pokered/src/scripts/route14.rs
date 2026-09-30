//! `Route14_Script`: ten trainers on the road between the Silence Bridge and Fuchsia, and nothing
//! else.

use poke_core::tables::trainers;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE14_BIKER1, TEXT_ROUTE14_BIKER2, TEXT_ROUTE14_BIKER3,
    TEXT_ROUTE14_BIKER4, TEXT_ROUTE14_COOLTRAINER_M1, TEXT_ROUTE14_COOLTRAINER_M2, TEXT_ROUTE14_COOLTRAINER_M3,
    TEXT_ROUTE14_COOLTRAINER_M4, TEXT_ROUTE14_COOLTRAINER_M5, TEXT_ROUTE14_COOLTRAINER_M6};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute14CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute14CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route14.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, trainers::Route14TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE14_COOLTRAINER_M1 => trainers::Route14TrainerHeader0,
        TEXT_ROUTE14_COOLTRAINER_M2 => trainers::Route14TrainerHeader1,
        TEXT_ROUTE14_COOLTRAINER_M3 => trainers::Route14TrainerHeader2,
        TEXT_ROUTE14_COOLTRAINER_M4 => trainers::Route14TrainerHeader3,
        TEXT_ROUTE14_COOLTRAINER_M5 => trainers::Route14TrainerHeader4,
        TEXT_ROUTE14_COOLTRAINER_M6 => trainers::Route14TrainerHeader5,
        TEXT_ROUTE14_BIKER1 => trainers::Route14TrainerHeader6,
        TEXT_ROUTE14_BIKER2 => trainers::Route14TrainerHeader7,
        TEXT_ROUTE14_BIKER3 => trainers::Route14TrainerHeader8,
        TEXT_ROUTE14_BIKER4 => trainers::Route14TrainerHeader9,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route14.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
