//! `PewterMart_Script`: a mart that draws no box of its own for a text, and two shoppers whose words
//! are code only to print them in one.

use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_PEWTERMART_SUPER_NERD, TEXT_PEWTERMART_YOUNGSTER};
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    // `EnableAutoTextBoxDrawing` with `BIT_NO_AUTO_TEXT_BOX` written after it by hand.
    rt.disable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let words = match text_id {
        TEXT_PEWTERMART_YOUNGSTER => local::PewterMartYoungsterText::Text,
        TEXT_PEWTERMART_SUPER_NERD => local::PewterMartSuperNerdText::Text,
        _ => return None,
    };
    Some(rt.print_text(text_at(words)).ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
