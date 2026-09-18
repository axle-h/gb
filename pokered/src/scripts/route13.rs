//! `Route13_Script`: ten trainers on the bird stretch north of the Silence Bridge, and nothing else.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE13_BEAUTY1, TEXT_ROUTE13_BEAUTY2, TEXT_ROUTE13_BIKER,
    TEXT_ROUTE13_COOLTRAINER_F1, TEXT_ROUTE13_COOLTRAINER_F2, TEXT_ROUTE13_COOLTRAINER_F3,
    TEXT_ROUTE13_COOLTRAINER_F4, TEXT_ROUTE13_COOLTRAINER_M1, TEXT_ROUTE13_COOLTRAINER_M2,
    TEXT_ROUTE13_COOLTRAINER_M3};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute13CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute13CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route13.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route13TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE13_COOLTRAINER_M1 => sym::Route13TrainerHeader0,
        TEXT_ROUTE13_COOLTRAINER_F1 => sym::Route13TrainerHeader1,
        TEXT_ROUTE13_COOLTRAINER_F2 => sym::Route13TrainerHeader2,
        TEXT_ROUTE13_COOLTRAINER_F3 => sym::Route13TrainerHeader3,
        TEXT_ROUTE13_COOLTRAINER_F4 => sym::Route13TrainerHeader4,
        TEXT_ROUTE13_COOLTRAINER_M2 => sym::Route13TrainerHeader5,
        TEXT_ROUTE13_BEAUTY1 => sym::Route13TrainerHeader6,
        TEXT_ROUTE13_BEAUTY2 => sym::Route13TrainerHeader7,
        TEXT_ROUTE13_BIKER => sym::Route13TrainerHeader8,
        TEXT_ROUTE13_COOLTRAINER_M3 => sym::Route13TrainerHeader9,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route13.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
