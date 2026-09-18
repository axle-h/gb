//! `CinnabarLabTradeRoom_Script`: the lab's side room, where two of its staff trade.

use poke_core::symbols::pokered_map_scripts::{TEXT_CINNABARLABTRADEROOM_BEAUTY, TEXT_CINNABARLABTRADEROOM_GRAMPS};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `TRADE_FOR_DORIS` and `TRADE_FOR_CRINKLES`.
const TRADE_FOR_DORIS: u8 = 7;
const TRADE_FOR_CRINKLES: u8 = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let which = match text_id {
        TEXT_CINNABARLABTRADEROOM_GRAMPS => TRADE_FOR_DORIS,
        TEXT_CINNABARLABTRADEROOM_BEAUTY => TRADE_FOR_CRINKLES,
        _ => return None,
    };
    Some(rt.do_in_game_trade_dialogue(which).ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
