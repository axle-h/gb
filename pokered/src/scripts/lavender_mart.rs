//! `LavenderMart_Script`: the shopper by the shelves, who recommends a different thing to buy once
//! Mr Fuji is home.

use poke_core::symbols::pokered_events::EVENT_RESCUED_MR_FUJI;
use poke_core::symbols::pokered_map_scripts::TEXT_LAVENDERMART_COOLTRAINER_M;
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    
    match text_id {
        TEXT_LAVENDERMART_COOLTRAINER_M => {
            let said = match rt.check_event(EVENT_RESCUED_MR_FUJI) {
                true => "LavenderMartCooltrainerMText.NuggetText",
                false => "LavenderMartCooltrainerMText.ReviveText",
            };
            Some(rt.print_text(text_named(said)).ret())
        }
        _ => None,
    }
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
