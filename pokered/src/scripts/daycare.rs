//! `Daycare_Script`: the day care man, whose text is `DaycareGentlemanText`.

use poke_core::symbols::pokered_map_scripts::TEXT_DAYCARE_GENTLEMAN;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_DAYCARE_GENTLEMAN).then(|| rt.day_care_gentleman().ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
