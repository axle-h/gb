//! `VermilionTradeHouse_Script`: the girl who trades a Farfetch'd for a Spearow.

use poke_core::symbols::pokered_map_scripts::TEXT_VERMILIONTRADEHOUSE_LITTLE_GIRL;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `TRADE_FOR_DUX`.
const TRADE_FOR_DUX: u8 = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_VERMILIONTRADEHOUSE_LITTLE_GIRL).then(|| rt.do_in_game_trade_dialogue(TRADE_FOR_DUX).ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
