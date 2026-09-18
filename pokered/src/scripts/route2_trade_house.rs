//! `Route2TradeHouse_Script`: the boy who trades a Mr. Mime for an Abra.

use poke_core::symbols::pokered_map_scripts::TEXT_ROUTE2TRADEHOUSE_GAMEBOY_KID;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `TRADE_FOR_MARCEL`.
const TRADE_FOR_MARCEL: u8 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_ROUTE2TRADEHOUSE_GAMEBOY_KID).then(|| rt.do_in_game_trade_dialogue(TRADE_FOR_MARCEL).ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
