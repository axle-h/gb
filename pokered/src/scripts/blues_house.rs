//! `BluesHouse_Script`: the visit it remembers, and Daisy, who hands over the Town Map once Oak has
//! handed out the Pokédex.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_ENTERED_BLUES_HOUSE, EVENT_GOT_POKEDEX, EVENT_GOT_TOWN_MAP};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_BLUESHOUSE_DEFAULT, SCRIPT_BLUESHOUSE_NOOP,
    TEXT_BLUESHOUSE_DAISY_SITTING};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::TOGGLE_TOWN_MAP;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wBluesHouseCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `BluesHouseDaisySittingText.give_town_map` after `BluesHouseDaisyOfferMapText`.
    GiveTownMap,
    /// After `GotMapText`: the event is set once the words are read.
    GotMap,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    if rt.maps().blues_house.cur_script == SCRIPT_BLUESHOUSE_DEFAULT {
        rt.set_event(EVENT_ENTERED_BLUES_HOUSE);
        rt.maps().blues_house.cur_script = SCRIPT_BLUESHOUSE_NOOP;
    }
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_BLUESHOUSE_DAISY_SITTING {
        return None;
    }
    Some(if rt.check_event(EVENT_GOT_TOWN_MAP) {
        rt.print_text(text_at(sym::BluesHouseDaisyUseMapText)).ret()
    } else if rt.check_event(EVENT_GOT_POKEDEX) {
        rt.print_text(text_at(sym::BluesHouseDaisyOfferMapText)).then(Label::GiveTownMap)
    } else {
        rt.print_text(text_at(sym::BluesHouseDaisyRivalAtLabText)).ret()
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::GiveTownMap => {
            if !rt.give_item(ItemId::TownMap, 1) {
                return rt.print_text(text_at(sym::BluesHouseDaisyBagFullText)).ret();
            }
            rt.hide_object(TOGGLE_TOWN_MAP);
            rt.print_text(text_at(sym::GotMapText)).then(Label::GotMap)
        }
        Label::GotMap => {
            rt.set_event(EVENT_GOT_TOWN_MAP);
            Flow::Return
        }
    }
}
