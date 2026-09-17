//! `CeladonMartRoof_Script`: the thirsty girl, who trades a TM for each of the three drinks the
//! vending machines behind her sell.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_GOT_TM13, EVENT_GOT_TM48, EVENT_GOT_TM49};
use poke_core::symbols::pokered_local_labels::CeladonMartRoofLittleGirlText as girl;
use poke_core::symbols::pokered_map_scripts::TEXT_CELADONMARTROOF_LITTLE_GIRL;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// `CeladonMartRoofDrinkList`, each with the event and the TM it is worth.
const DRINKS: [(ItemId, u16, ItemId); 3] = [
    (ItemId::FreshWater, EVENT_GOT_TM13, ItemId::Tm13IceBeam),
    (ItemId::SodaPop, EVENT_GOT_TM48, ItemId::Tm48RockSlide),
    (ItemId::Lemonade, EVENT_GOT_TM49, ItemId::Tm49TriAttack),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `CeladonMartRoofLittleGirlText`, once she has asked for a drink.
    DrinkAsked,
    DrinkAnswered,
    /// `CeladonMartRoofScript_GiveDrinkToGirl`: its menu, then the drink picked off it.
    DrinkMenu,
    DrinkPicked,
    /// Carrying which of the three drinks was handed over.
    DrinkThanked(usize),
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

/// `CeladonMartRoofScript_GetDrinksInBag`, in the list's own order.
fn drinks_in_bag(rt: &Script) -> Vec<usize> {
    (0..DRINKS.len()).filter(|&i| rt.is_item_in_bag(DRINKS[i].0)).collect()
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_CELADONMARTROOF_LITTLE_GIRL {
        return None;
    }
    if drinks_in_bag(rt).is_empty() {
        return Some(rt.print_text(text_at(girl::ImThirstyText)).ret());
    }
    rt.set_do_not_wait_for_button_press(true);
    Some(rt.print_text(text_at(girl::GiveHerADrinkText)).then(Label::DrinkAsked))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DrinkAsked => rt.yes_no_choice().then(Label::DrinkAnswered),
        Label::DrinkAnswered => match rt.chose_yes() {
            true => rt.print_text(text_at(sym::CeladonMartRoofLittleGirlGiveHerWhichDrinkText)).then(Label::DrinkMenu),
            false => Flow::Return,
        },
        Label::DrinkMenu => {
            let drinks: Vec<ItemId> = drinks_in_bag(rt).iter().map(|&i| DRINKS[i].0).collect();
            rt.drink_menu(&drinks).then(Label::DrinkPicked)
        }
        // The menu lists only the drinks in the bag, so the row it answers with is an index into
        // that list rather than into the three drinks.
        Label::DrinkPicked => {
            let Some(row) = rt.chosen_row() else { return Flow::Return };
            let held = drinks_in_bag(rt);
            let Some(&drink) = held.get(row as usize) else { return Flow::Return };
            if rt.check_event(DRINKS[drink].1) {
                return rt.print_text(text_at(sym::CeladonMartRoofLittleGirlImNotThirstyText)).ret();
            }
            let yay = [sym::CeladonMartRoofLittleGirlYayFreshWaterText, sym::CeladonMartRoofLittleGirlYaySodaPopText,
                sym::CeladonMartRoofLittleGirlYayLemonadeText][drink];
            rt.print_text(text_at(yay)).then(Label::DrinkThanked(drink))
        }
        Label::DrinkThanked(drink) => {
            let (item, event, tm) = DRINKS[drink];
            rt.remove_item(item, 1);
            if !rt.give_item(tm, 1) {
                return rt.print_text(text_at(sym::CeladonMartRoofLittleGirlNoRoomText)).ret();
            }
            let received = [sym::CeladonMartRoofLittleGirlReceivedTM13Text, sym::CeladonMartRoofLittleGirlReceivedTM48Text,
                sym::CeladonMartRoofLittleGirlReceivedTM49Text][drink];
            rt.set_event(event);
            rt.print_text(text_at(received)).ret()
        }
    }
}
