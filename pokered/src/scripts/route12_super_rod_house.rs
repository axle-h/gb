//! `Route12SuperRodHouse_Script`: the Fishing Guru's brother, who gives the Super Rod to a player
//! who likes to fish.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_map_scripts::TEXT_ROUTE12SUPERRODHOUSE_FISHING_GURU;
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// After `.DoYouLikeToFishText`, and after the yes/no.
    AskedToFish,
    Answered,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_ROUTE12SUPERRODHOUSE_FISHING_GURU {
        return None;
    }
    if rt.got_super_rod() {
        return Some(rt.print_text(text_named("Route12SuperRodHouseFishingGuruText.TryFishingText")).ret());
    }
    Some(rt.print_text(text_named("Route12SuperRodHouseFishingGuruText.DoYouLikeToFishText")).then(Label::AskedToFish))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::AskedToFish => rt.yes_no_choice().then(Label::Answered),
        Label::Answered => {
            let words = if !rt.chose_yes() {
                "Route12SuperRodHouseFishingGuruText.ThatsDisappointingText"
            } else if rt.give_item(ItemId::SuperRod, 1) {
                rt.set_got_super_rod();
                "Route12SuperRodHouseFishingGuruText.ReceivedSuperRodText"
            } else {
                "Route12SuperRodHouseFishingGuruText.NoRoomText"
            };
            rt.print_text(text_named(words)).ret()
        }
    }
}
