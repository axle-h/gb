//! `CinnabarLabMetronomeRoom_Script`: the scientist who hands over TM35, and explains what a
//! Metronome does to anyone who already has it.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_TM35;
use poke_core::symbols::pokered_local_labels::CinnabarLabMetronomeRoomScientist1Text as scientist;
use poke_core::symbols::pokered_map_scripts::TEXT_CINNABARLABMETRONOMEROOM_SCIENTIST1;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `CinnabarLabMetronomeRoomScientist1Text` after `.Text`, and after `.ReceivedTM35Text`.
    Offered,
    Received,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_CINNABARLABMETRONOMEROOM_SCIENTIST1 {
        return None;
    }
    if rt.check_event(EVENT_GOT_TM35) {
        return Some(rt.print_text(text_at(scientist::TM35ExplanationText)).ret());
    }
    Some(rt.print_text(text_at(scientist::Text)).then(Label::Offered))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Offered => {
            if !rt.give_item(ItemId::Tm35Metronome, 1) {
                return rt.print_text(text_at(scientist::TM35NoRoomText)).ret();
            }
            rt.print_text(text_at(scientist::ReceivedTM35Text)).then(Label::Received)
        }
        Label::Received => {
            rt.set_event(EVENT_GOT_TM35);
            Flow::Return
        }
    }
}
