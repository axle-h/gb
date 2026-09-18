//! `Route15_Script`: ten trainers on the road east out of Fuchsia, and the TM20 ball
//! `PickUpItemText` takes care of.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE15_BEAUTY1, TEXT_ROUTE15_BEAUTY2, TEXT_ROUTE15_BIKER1,
    TEXT_ROUTE15_BIKER2, TEXT_ROUTE15_COOLTRAINER_F1, TEXT_ROUTE15_COOLTRAINER_F2, TEXT_ROUTE15_COOLTRAINER_F3,
    TEXT_ROUTE15_COOLTRAINER_F4, TEXT_ROUTE15_COOLTRAINER_M1, TEXT_ROUTE15_COOLTRAINER_M2};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute15CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute15CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route15.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route15TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE15_COOLTRAINER_F1 => sym::Route15TrainerHeader0,
        TEXT_ROUTE15_COOLTRAINER_F2 => sym::Route15TrainerHeader1,
        TEXT_ROUTE15_COOLTRAINER_M1 => sym::Route15TrainerHeader2,
        TEXT_ROUTE15_COOLTRAINER_M2 => sym::Route15TrainerHeader3,
        TEXT_ROUTE15_BEAUTY1 => sym::Route15TrainerHeader4,
        TEXT_ROUTE15_BEAUTY2 => sym::Route15TrainerHeader5,
        TEXT_ROUTE15_BIKER1 => sym::Route15TrainerHeader6,
        TEXT_ROUTE15_BIKER2 => sym::Route15TrainerHeader7,
        TEXT_ROUTE15_COOLTRAINER_F3 => sym::Route15TrainerHeader8,
        TEXT_ROUTE15_COOLTRAINER_F4 => sym::Route15TrainerHeader9,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route15.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
