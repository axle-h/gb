//! `SafariZoneSecretHouse_Script`: the fishing guru at the far end of the Safari Zone, who hands
//! over HM03 for reaching him.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_HM03;
use poke_core::symbols::pokered_map_scripts::TEXT_SAFARIZONESECRETHOUSE_FISHING_GURU;
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Congratulated,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_SAFARIZONESECRETHOUSE_FISHING_GURU {
        return None;
    }
    Some(match rt.check_event(EVENT_GOT_HM03) {
        true => rt.print_text(text_named("SafariZoneSecretHouseFishingGuruText.HM03ExplanationText")).ret(),
        false => rt.print_text(text_named("SafariZoneSecretHouseFishingGuruText.YouHaveWonText")).then(Label::Congratulated),
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Congratulated => {
            let said = match rt.give_item(ItemId::Hm03Surf, 1) {
                true => {
                    rt.set_event(EVENT_GOT_HM03);
                    "SafariZoneSecretHouseFishingGuruText.ReceivedHM03Text"
                }
                false => "SafariZoneSecretHouseFishingGuruText.HM03NoRoomText",
            };
            rt.print_text(text_named(said)).ret()
        }
    }
}
