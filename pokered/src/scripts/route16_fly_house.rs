//! `Route16FlyHouse_Script`: the girl hiding behind the Cycling Road who pays for her secret with
//! HM02, and the Fearow beside her.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::EVENT_GOT_HM02;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_ROUTE16FLYHOUSE_BRUNETTE_GIRL, TEXT_ROUTE16FLYHOUSE_FEAROW};
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route16FlyHouseBrunetteGirlText` after `.Text`.
    Offered,
    FearowCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_ROUTE16FLYHOUSE_BRUNETTE_GIRL => {
            if rt.check_event(EVENT_GOT_HM02) {
                return Some(fly_explanation(rt));
            }
            rt.print_text(text_at(local::Route16FlyHouseBrunetteGirlText::Text)).then(Label::Offered)
        }
        TEXT_ROUTE16FLYHOUSE_FEAROW => {
            rt.print_text(text_at(local::Route16FlyHouseFearowText::Text)).then(Label::FearowCry)
        }
        _ => return None,
    })
}

/// `.HM02ExplanationText`, which the girl says ever after.
fn fly_explanation(rt: &mut Script) -> Flow {
    rt.print_text(text_at(local::Route16FlyHouseBrunetteGirlText::HM02ExplanationText)).ret()
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Offered => {
            if !rt.give_item(ItemId::Hm02Fly, 1) {
                return rt.print_text(text_at(local::Route16FlyHouseBrunetteGirlText::HM02NoRoomText)).ret();
            }
            rt.set_event(EVENT_GOT_HM02);
            rt.print_text(text_at(local::Route16FlyHouseBrunetteGirlText::ReceivedHM02Text)).ret()
        }
        Label::FearowCry => {
            rt.play_cry(PokemonSpecies::Fearow);
            rt.wait_for_sound_to_finish().ret()
        }
    }
}
