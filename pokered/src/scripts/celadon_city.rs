//! `CeladonCity_Script`: the old man outside the Pokémon Centre who hands over TM41, and the
//! Poliwrath in the window of the diner, which cries when it is read about.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_1B8, EVENT_1BF, EVENT_67F, EVENT_GOT_TM41};
use poke_core::symbols::pokered_local_labels::CeladonCityGramps3Text as gramps;
use poke_core::symbols::pokered_map_scripts::{TEXT_CELADONCITY_GRAMPS3, TEXT_CELADONCITY_POLIWRATH};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `CeladonCityGramps3Text`, after the offer and after whichever answer it gave.
    Gramps3Offered,
    Gramps3Done,
    PoliwrathCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    // `ResetEvents` takes a list, not a range: the three events between these two are the hideout
    // and the Game Corner's coin handouts, which a walk through the city must not clear.
    rt.reset_event(EVENT_1B8);
    rt.reset_event(EVENT_1BF);
    rt.reset_event(EVENT_67F);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_CELADONCITY_GRAMPS3 => Some(match rt.check_event(EVENT_GOT_TM41) {
            true => rt.print_text(text_at(gramps::TM41ExplanationText)).ret(),
            false => rt.print_text(text_at(gramps::Text)).then(Label::Gramps3Offered),
        }),
        TEXT_CELADONCITY_POLIWRATH => {
            Some(rt.print_text(text_at(sym::CeladonCityPoliwrathText)).then(Label::PoliwrathCry))
        }
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Gramps3Offered => {
            let said = match rt.give_item(ItemId::Tm41Softboiled, 1) {
                true => gramps::ReceivedTM41Text,
                false => gramps::TM41NoRoomText,
            };
            rt.print_text(text_at(said)).then(Label::Gramps3Done)
        }
        Label::Gramps3Done => {
            if rt.is_item_in_bag(ItemId::Tm41Softboiled) {
                rt.set_event(EVENT_GOT_TM41);
            }
            Flow::Return
        }
        Label::PoliwrathCry => {
            rt.play_cry(PokemonSpecies::Poliwrath);
            Flow::Return
        }
    }
}
