//! `Route15Gate2F_Script`: Oak's aide, who hands over the EXP.ALL to a player with fifty Pokémon
//! owned, and the binoculars looking south over the sea.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_EXP_ALL;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE15GATE2F_BINOCULARS, TEXT_ROUTE15GATE2F_OAKS_AIDE};
use serde::{Deserialize, Serialize};
use crate::systems::events::tables::OaksAideResult;
use super::route12_gate_2f::print_if_facing_up;
use super::{text_at, Flow, Script};

/// `hOaksAideRequirement`.
const REQUIREMENT: u8 = 50;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route15Gate2FOaksAideText` after `OaksAideScript`.
    AideDone,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.disable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_ROUTE15GATE2F_OAKS_AIDE => {
            if rt.check_event(EVENT_GOT_EXP_ALL) {
                return Some(exp_all_explanation(rt));
            }
            rt.oaks_aide(REQUIREMENT, ItemId::ExpAll).then(Label::AideDone)
        }
        TEXT_ROUTE15GATE2F_BINOCULARS => {
            print_if_facing_up(rt, local::Route15Gate2FBinocularsText::Text)
        }
        _ => return None,
    })
}

/// `.got_item`.
fn exp_all_explanation(rt: &mut Script) -> Flow {
    rt.print_text(text_at(local::Route15Gate2FOaksAideText::ExpAllText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::AideDone => {
            if rt.oaks_aide_result() != Some(OaksAideResult::GotItem) {
                return Flow::Return;
            }
            rt.set_event(EVENT_GOT_EXP_ALL);
            exp_all_explanation(rt)
        }
    }
}
