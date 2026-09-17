//! `Route16Gate2F_Script`: the two children upstairs and the binoculars they are watching Kanto
//! through, which show nothing to anybody not facing them.

use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE16GATE2F_LEFT_BINOCULARS, TEXT_ROUTE16GATE2F_LITTLE_BOY,
    TEXT_ROUTE16GATE2F_LITTLE_GIRL, TEXT_ROUTE16GATE2F_RIGHT_BINOCULARS};
use poke_core::symbols::DmgPointer;
use serde::{Deserialize, Serialize};
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.disable_auto_text_box_drawing();
    Flow::Return
}

/// `GateUpstairsScript_PrintIfFacingUp`: a player who walked past without turning is not kept
/// waiting for a button, since nothing was printed.
fn print_if_facing_up(rt: &mut Script, at: DmgPointer) -> Flow {
    if rt.player_facing() != SPRITE_FACING_UP {
        rt.set_do_not_wait_for_button_press(true);
        return Flow::Return;
    }
    rt.set_do_not_wait_for_button_press(false);
    rt.print_text(text_at(at)).ret()
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
