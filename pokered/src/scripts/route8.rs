//! `Route8_Script`: nine trainers between Lavender and Saffron, and nothing else.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute8CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute8CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route8.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route8TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE8_SUPER_NERD1 => sym::Route8TrainerHeader0,
        TEXT_ROUTE8_GAMBLER1 => sym::Route8TrainerHeader1,
        TEXT_ROUTE8_SUPER_NERD2 => sym::Route8TrainerHeader2,
        TEXT_ROUTE8_COOLTRAINER_F1 => sym::Route8TrainerHeader3,
        TEXT_ROUTE8_SUPER_NERD3 => sym::Route8TrainerHeader4,
        TEXT_ROUTE8_COOLTRAINER_F2 => sym::Route8TrainerHeader5,
        TEXT_ROUTE8_COOLTRAINER_F3 => sym::Route8TrainerHeader6,
        TEXT_ROUTE8_GAMBLER2 => sym::Route8TrainerHeader7,
        TEXT_ROUTE8_COOLTRAINER_F4 => sym::Route8TrainerHeader8,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route8.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
