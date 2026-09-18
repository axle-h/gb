//! `Route18Gate2F_Script`: the youngster who trades his Lickitung for a Slowbro, and the binoculars
//! looking west over Pallet Town and down on the swimmers.

use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE18GATE2F_LEFT_BINOCULARS,
    TEXT_ROUTE18GATE2F_RIGHT_BINOCULARS, TEXT_ROUTE18GATE2F_YOUNGSTER};
use serde::{Deserialize, Serialize};
use super::route12_gate_2f::print_if_facing_up;
use super::{Flow, Script};

/// `TRADE_FOR_MARC`.
const TRADE_FOR_MARC: u8 = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.disable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_ROUTE18GATE2F_YOUNGSTER => rt.do_in_game_trade_dialogue(TRADE_FOR_MARC).ret(),
        TEXT_ROUTE18GATE2F_LEFT_BINOCULARS => {
            print_if_facing_up(rt, local::Route18Gate2FLeftBinocularsText::Text)
        }
        TEXT_ROUTE18GATE2F_RIGHT_BINOCULARS => {
            print_if_facing_up(rt, local::Route18Gate2FRightBinocularsText::Text)
        }
        _ => return None,
    })
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
