//! `MrPsychicsHouse_Script`: Mr Psychic, who knows what the player came for and hands over TM29.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_TM29;
use poke_core::symbols::pokered_local_labels::MrPsychicsHouseMrPsychicText as psychic;
use poke_core::symbols::pokered_map_scripts::TEXT_MRPSYCHICSHOUSE_MR_PSYCHIC;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `MrPsychicsHouseMrPsychicText` after `.YouWantedThisText`, and after `.ReceivedTM29Text`.
    Offered,
    Received,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_MRPSYCHICSHOUSE_MR_PSYCHIC {
        return None;
    }
    if rt.check_event(EVENT_GOT_TM29) {
        return Some(rt.print_text(text_at(psychic::TM29ExplanationText)).ret());
    }
    Some(rt.print_text(text_at(psychic::YouWantedThisText)).then(Label::Offered))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Offered => {
            if !rt.give_item(ItemId::Tm29Psychic, 1) {
                return rt.print_text(text_at(psychic::TM29NoRoomText)).ret();
            }
            rt.print_text(text_at(psychic::ReceivedTM29Text)).then(Label::Received)
        }
        Label::Received => {
            rt.set_event(EVENT_GOT_TM29);
            Flow::Return
        }
    }
}
