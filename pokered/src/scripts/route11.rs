//! `Route11_Script`: ten trainers on the road east out of Vermilion, and nothing else.

use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE11_GAMBLER1, TEXT_ROUTE11_GAMBLER2,
    TEXT_ROUTE11_GAMBLER3, TEXT_ROUTE11_GAMBLER4, TEXT_ROUTE11_SUPER_NERD1, TEXT_ROUTE11_SUPER_NERD2,
    TEXT_ROUTE11_YOUNGSTER1, TEXT_ROUTE11_YOUNGSTER2, TEXT_ROUTE11_YOUNGSTER3, TEXT_ROUTE11_YOUNGSTER4};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute11CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRoute11CurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route11.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::Route11TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE11_GAMBLER1 => sym::Route11TrainerHeader0,
        TEXT_ROUTE11_GAMBLER2 => sym::Route11TrainerHeader1,
        TEXT_ROUTE11_YOUNGSTER1 => sym::Route11TrainerHeader2,
        TEXT_ROUTE11_SUPER_NERD1 => sym::Route11TrainerHeader3,
        TEXT_ROUTE11_YOUNGSTER2 => sym::Route11TrainerHeader4,
        TEXT_ROUTE11_GAMBLER3 => sym::Route11TrainerHeader5,
        TEXT_ROUTE11_GAMBLER4 => sym::Route11TrainerHeader6,
        TEXT_ROUTE11_YOUNGSTER3 => sym::Route11TrainerHeader7,
        TEXT_ROUTE11_SUPER_NERD2 => sym::Route11TrainerHeader8,
        TEXT_ROUTE11_YOUNGSTER4 => sym::Route11TrainerHeader9,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().route11.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
