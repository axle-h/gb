//! `Route12SuperRodHouse_Script`: the Fishing Guru's brother, who gives the Super Rod to a player
//! who likes to fish.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_local_labels::Route12SuperRodHouseFishingGuruText as guru;
use poke_core::symbols::pokered_map_scripts::TEXT_ROUTE12SUPERRODHOUSE_FISHING_GURU;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

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
        return Some(rt.print_text(text_at(guru::TryFishingText)).ret());
    }
    Some(rt.print_text(text_at(guru::DoYouLikeToFishText)).then(Label::AskedToFish))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::AskedToFish => rt.yes_no_choice().then(Label::Answered),
        Label::Answered => {
            let words = if !rt.chose_yes() {
                guru::ThatsDisappointingText
            } else if rt.give_item(ItemId::SuperRod, 1) {
                rt.set_got_super_rod();
                guru::ReceivedSuperRodText
            } else {
                guru::NoRoomText
            };
            rt.print_text(text_at(words)).ret()
        }
    }
}
