//! `Route2Gate_Script`: Oak's aide, who hands over HM05 to a player with ten Pokémon owned.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_HM05;
use poke_core::symbols::pokered_local_labels::Route2GateOaksAideText;
use poke_core::symbols::pokered_map_scripts::TEXT_ROUTE2GATE_OAKS_AIDE;
use serde::{Deserialize, Serialize};
use crate::systems::events::tables::OaksAideResult;
use super::{text_at, Flow, Script};

/// `hOaksAideRequirement`.
const REQUIREMENT: u8 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route2GateOaksAideText` after `OaksAideScript`.
    AideDone,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_ROUTE2GATE_OAKS_AIDE {
        return None;
    }
    if rt.check_event(EVENT_GOT_HM05) {
        return Some(flash_explanation(rt));
    }
    Some(rt.oaks_aide(REQUIREMENT, ItemId::Hm05Flash).then(Label::AideDone))
}

/// `.got_item`.
fn flash_explanation(rt: &mut Script) -> Flow {
    rt.print_text(text_at(Route2GateOaksAideText::FlashExplanationText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::AideDone => {
            if rt.oaks_aide_result() != Some(OaksAideResult::GotItem) {
                return Flow::Return;
            }
            rt.set_event(EVENT_GOT_HM05);
            flash_explanation(rt)
        }
    }
}
