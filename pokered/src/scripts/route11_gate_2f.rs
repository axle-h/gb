//! `Route11Gate2F_Script`: the boy who trades his Nidorina for a Nidorino, Oak's aide with the
//! Itemfinder, and the binoculars, the left pair of which look out over Route 12.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_ROUTE12_SNORLAX, EVENT_GOT_ITEMFINDER};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE11GATE2F_LEFT_BINOCULARS, TEXT_ROUTE11GATE2F_OAKS_AIDE,
    TEXT_ROUTE11GATE2F_RIGHT_BINOCULARS, TEXT_ROUTE11GATE2F_YOUNGSTER};
use serde::{Deserialize, Serialize};
use crate::systems::events::tables::OaksAideResult;
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::route12_gate_2f::print_if_facing_up;
use super::{text_at, Flow, Script};

/// `TRADE_FOR_TERRY`.
const TRADE_FOR_TERRY: u8 = 0;
/// `hOaksAideRequirement`.
const REQUIREMENT: u8 = 30;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route11Gate2FOaksAideText` after `OaksAideScript`.
    AideDone,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.disable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_ROUTE11GATE2F_YOUNGSTER => rt.do_in_game_trade_dialogue(TRADE_FOR_TERRY).ret(),
        TEXT_ROUTE11GATE2F_OAKS_AIDE => {
            if rt.check_event(EVENT_GOT_ITEMFINDER) {
                return Some(itemfinder_description(rt));
            }
            rt.oaks_aide(REQUIREMENT, ItemId::Itemfinder).then(Label::AideDone)
        }
        // The left binoculars test the facing themselves and only then choose a text, so they reach
        // `GateUpstairsScript_PrintIfFacingUp` with nothing to print: all it does here is spare a
        // player who is not facing up the button press.
        TEXT_ROUTE11GATE2F_LEFT_BINOCULARS => {
            if rt.player_facing() != SPRITE_FACING_UP {
                rt.set_do_not_wait_for_button_press(true);
                return Some(Flow::Return);
            }
            let words = if rt.check_event(EVENT_BEAT_ROUTE12_SNORLAX) {
                local::Route11Gate2FLeftBinocularsText::NoSnorlaxText
            } else {
                local::Route11Gate2FLeftBinocularsText::SnorlaxText
            };
            rt.print_text(text_at(words)).ret()
        }
        TEXT_ROUTE11GATE2F_RIGHT_BINOCULARS => {
            print_if_facing_up(rt, local::Route11Gate2FRightBinocularsText::Text)
        }
        _ => return None,
    })
}

/// `.got_item`.
fn itemfinder_description(rt: &mut Script) -> Flow {
    rt.print_text(text_at(local::Route11Gate2FOaksAideText::ItemfinderDescriptionText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::AideDone => {
            if rt.oaks_aide_result() != Some(OaksAideResult::GotItem) {
                return Flow::Return;
            }
            rt.set_event(EVENT_GOT_ITEMFINDER);
            itemfinder_description(rt)
        }
    }
}
