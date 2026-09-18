//! `CeruleanTradeHouse_Script`: the gambler who trades a Jynx for a Poliwhirl.

use poke_core::symbols::pokered_map_scripts::TEXT_CERULEANTRADEHOUSE_GAMBLER;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `TRADE_FOR_LOLA`.
const TRADE_FOR_LOLA: u8 = 6;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_CERULEANTRADEHOUSE_GAMBLER).then(|| rt.do_in_game_trade_dialogue(TRADE_FOR_LOLA).ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
