//! `FuchsiaGoodRodHouse_Script`: the Fishing Guru's older brother, who gives the Good Rod to a
//! player who likes to fish.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_local_labels::FuchsiaGoodRodHouseFishingGuruText as guru;
use poke_core::symbols::pokered_map_scripts::TEXT_FUCHSIAGOODRODHOUSE_FISHING_GURU;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// After `.Text`, and after the yes/no.
    AskedToFish,
    Answered,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_FUCHSIAGOODRODHOUSE_FISHING_GURU {
        return None;
    }
    if rt.got_good_rod() {
        return Some(rt.print_text(text_at(guru::HowAreTheFishText)).ret());
    }
    Some(rt.print_text(text_at(guru::Text)).then(Label::AskedToFish))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::AskedToFish => rt.yes_no_choice().then(Label::Answered),
        Label::Answered => {
            let words = if !rt.chose_yes() {
                guru::ThatsSoDisappointingText
            } else if rt.give_item(ItemId::GoodRod, 1) {
                rt.set_got_good_rod();
                guru::ReceivedGoodRodText
            } else {
                guru::NoRoomText
            };
            rt.print_text(text_at(words)).ret()
        }
    }
}
