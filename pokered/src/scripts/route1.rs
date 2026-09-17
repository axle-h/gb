//! `Route1_Script` and the mart clerk who hands out a sample.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_POTION_SAMPLE;
use poke_core::symbols::pokered_local_labels::Route1Youngster1Text;
use poke_core::symbols::pokered_map_scripts::TEXT_ROUTE1_YOUNGSTER1;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route1Youngster1Text` after `.MartSampleText`.
    GiveItem,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_ROUTE1_YOUNGSTER1 => {
            if rt.check_and_set_event(EVENT_GOT_POTION_SAMPLE) {
                return Some(rt.print_text(text_at(Route1Youngster1Text::AlsoGotPokeballsText)).ret());
            }
            rt.print_text(text_at(Route1Youngster1Text::MartSampleText)).then(Label::GiveItem)
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::GiveItem => {
            let words = if rt.give_item(ItemId::Potion, 1) {
                Route1Youngster1Text::GotPotionText
            } else {
                Route1Youngster1Text::NoRoomText
            };
            rt.print_text(text_at(words)).ret()
        }
    }
}
