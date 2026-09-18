//! `Route16Gate2F_Script`: the two children upstairs and the binoculars they are watching Kanto
//! through, which show nothing to anybody not facing them.

use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE16GATE2F_LEFT_BINOCULARS, TEXT_ROUTE16GATE2F_LITTLE_BOY,
    TEXT_ROUTE16GATE2F_LITTLE_GIRL, TEXT_ROUTE16GATE2F_RIGHT_BINOCULARS};
use serde::{Deserialize, Serialize};
use super::route12_gate_2f::print_if_facing_up;
use super::{text_at, Flow, Script};

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
        TEXT_ROUTE16GATE2F_LITTLE_BOY => rt.print_text(text_at(local::Route16Gate2FLittleBoyText::Text)).ret(),
        TEXT_ROUTE16GATE2F_LITTLE_GIRL => rt.print_text(text_at(local::Route16Gate2FLittleGirlText::Text)).ret(),
        TEXT_ROUTE16GATE2F_LEFT_BINOCULARS => {
            print_if_facing_up(rt, local::Route16Gate2FLeftBinocularsText::Text)
        }
        TEXT_ROUTE16GATE2F_RIGHT_BINOCULARS => {
            print_if_facing_up(rt, local::Route16Gate2FRightBinocularsText::Text)
        }
        _ => return None,
    })
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
