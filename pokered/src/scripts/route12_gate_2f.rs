//! `Route12Gate2F_Script`: the girl who gives TM39 away, and the binoculars beside her.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_TM39;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE12GATE2F_BRUNETTE_GIRL, TEXT_ROUTE12GATE2F_LEFT_BINOCULARS,
    TEXT_ROUTE12GATE2F_RIGHT_BINOCULARS};
use poke_core::symbols::DmgPointer;
use serde::{Deserialize, Serialize};
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route12Gate2FBrunetteGirlText` after `.YouCanHaveThisText`, and after `.ReceivedTM39Text`.
    Offered,
    Received,
}

/// `GateUpstairsScript_PrintIfFacingUp`, which the binoculars of five gates jump to: a player who
/// walked past without turning is not kept waiting for a button, since nothing was printed.
pub fn print_if_facing_up(rt: &mut Script, at: DmgPointer) -> Flow {
    if rt.player_facing() != SPRITE_FACING_UP {
        rt.set_do_not_wait_for_button_press(true);
        return Flow::Return;
    }
    rt.set_do_not_wait_for_button_press(false);
    rt.print_text(text_at(at)).ret()
}

pub fn script(rt: &mut Script) -> Flow {
    rt.disable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_ROUTE12GATE2F_BRUNETTE_GIRL => {
            if rt.check_event(EVENT_GOT_TM39) {
                return Some(rt.print_text(text_at(local::Route12Gate2FBrunetteGirlText::TM39ExplanationText)).ret());
            }
            rt.print_text(text_at(local::Route12Gate2FBrunetteGirlText::YouCanHaveThisText)).then(Label::Offered)
        }
        TEXT_ROUTE12GATE2F_LEFT_BINOCULARS => {
            print_if_facing_up(rt, local::Route12Gate2FLeftBinocularsText::Text)
        }
        TEXT_ROUTE12GATE2F_RIGHT_BINOCULARS => {
            print_if_facing_up(rt, local::Route12Gate2FRightBinocularsText::Text)
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Offered => {
            if !rt.give_item(ItemId::Tm39Swift, 1) {
                return rt.print_text(text_at(local::Route12Gate2FBrunetteGirlText::TM39NoRoomText)).ret();
            }
            rt.print_text(text_at(local::Route12Gate2FBrunetteGirlText::ReceivedTM39Text)).then(Label::Received)
        }
        Label::Received => {
            rt.set_event(EVENT_GOT_TM39);
            Flow::Return
        }
    }
}
