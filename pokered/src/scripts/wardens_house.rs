//! `WardensHouse_Script`: the warden, who can only be understood once his gold teeth are back in,
//! and the two display cases behind him.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_GAVE_GOLD_TEETH, EVENT_GOT_HM04};
use poke_core::symbols::pokered_local_labels::{WardensHouseDisplayText as display,
    WardensHouseWardenText as warden};
use poke_core::symbols::pokered_map_scripts::{TEXT_WARDENSHOUSE_DISPLAY_LEFT, TEXT_WARDENSHOUSE_DISPLAY_RIGHT,
    TEXT_WARDENSHOUSE_WARDEN};
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    GibberishAsked,
    GibberishAnswered,
    /// `.have_gold_teeth`: the teeth are only taken once the text about them has closed.
    TeethHandedOver,
    Thanked,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

/// `.gave_gold_teeth`, which the warden also reaches on a second visit with a bag that was full.
fn thanks(rt: &mut Script) -> Flow {
    rt.print_text(text_at(warden::ThanksText)).then(Label::Thanked)
}

/// `WardensHouseWardenText`.
fn warden_text(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_GOT_HM04) {
        return rt.print_text(text_at(warden::HM04ExplanationText)).ret();
    }
    if rt.is_item_in_bag(ItemId::GoldTeeth) {
        // `.GaveTheGoldTeethText` has no `text_end`, so it runs straight on into the line the
        // disassembly calls unreferenced: he pops the teeth in and speaks plainly from there.
        return rt.print_text(text_at(warden::GaveTheGoldTeethText)).then(Label::TeethHandedOver);
    }
    if rt.check_event(EVENT_GAVE_GOLD_TEETH) {
        return thanks(rt);
    }
    rt.print_text(text_at(warden::Gibberish1Text)).then(Label::GibberishAsked)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let words = match text_id {
        TEXT_WARDENSHOUSE_WARDEN => return Some(warden_text(rt)),
        TEXT_WARDENSHOUSE_DISPLAY_LEFT => display::PhotosAndFossilsText,
        TEXT_WARDENSHOUSE_DISPLAY_RIGHT => display::MerchandiseText,
        _ => return None,
    };
    Some(rt.print_text(text_at(words)).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::GibberishAsked => rt.yes_no_choice().then(Label::GibberishAnswered),
        Label::GibberishAnswered => {
            let said = match rt.chose_yes() {
                true => warden::Gibberish2Text,
                false => warden::Gibberish3Text,
            };
            rt.print_text(text_at(said)).ret()
        }
        Label::TeethHandedOver => {
            rt.remove_item(ItemId::GoldTeeth, 1);
            rt.set_event(EVENT_GAVE_GOLD_TEETH);
            thanks(rt)
        }
        Label::Thanked => {
            let said = match rt.give_item(ItemId::Hm04Strength, 1) {
                true => {
                    rt.set_event(EVENT_GOT_HM04);
                    warden::ReceivedHM04Text
                }
                false => warden::HM04NoRoomText,
            };
            rt.print_text(text_at(said)).ret()
        }
    }
}
