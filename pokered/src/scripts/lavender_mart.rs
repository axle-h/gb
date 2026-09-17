//! `LavenderMart_Script`: the shopper by the shelves, who recommends a different thing to buy once
//! Mr Fuji is home.

use poke_core::symbols::pokered_events::EVENT_RESCUED_MR_FUJI;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::TEXT_LAVENDERMART_COOLTRAINER_M;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    use local::LavenderMartCooltrainerMText as words;
    match text_id {
        TEXT_LAVENDERMART_COOLTRAINER_M => {
            let said = match rt.check_event(EVENT_RESCUED_MR_FUJI) {
                true => words::NuggetText,
                false => words::ReviveText,
            };
            Some(rt.print_text(text_at(said)).ret())
        }
        _ => None,
    }
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
