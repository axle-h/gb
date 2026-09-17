//! `UndergroundPathRoute5_Script`: `wLastMap` held to Route 5, and the girl who trades a Nidoran♀ for
//! a Nidoran♂.

use poke_core::map::Map;
use poke_core::symbols::pokered_map_scripts::TEXT_UNDERGROUNDPATHROUTE5_LITTLE_GIRL;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `TRADE_FOR_SPOT`.
const TRADE_FOR_SPOT: u8 = 9;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

/// Not `EnableAutoTextBoxDrawing`: this map's script is only the `wLastMap` write.
pub fn script(rt: &mut Script) -> Flow {
    rt.set_last_map(Map::Route5);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_UNDERGROUNDPATHROUTE5_LITTLE_GIRL).then(|| rt.do_in_game_trade_dialogue(TRADE_FOR_SPOT).ret())
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
