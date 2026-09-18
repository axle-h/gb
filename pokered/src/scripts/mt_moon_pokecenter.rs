//! `MtMoonPokecenter_Script`: the man who sells a Magikarp for ¥500, once.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::EVENT_BOUGHT_MAGIKARP;
use poke_core::symbols::pokered_local_labels::MtMoonPokecenterMagikarpSalesmanText as salesman;
use poke_core::symbols::pokered_map_scripts::TEXT_MTMOONPOKECENTER_MAGIKARP_SALESMAN;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// `hMoney` and `wPriceTemp`, in BCD.
const PRICE: [u8; 3] = [0x00, 0x05, 0x00];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// After `.IGotADealText`, after the yes/no, and after `GivePokemon`.
    DealOffered,
    DealAnswered,
    MagikarpGiven,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_MTMOONPOKECENTER_MAGIKARP_SALESMAN {
        return None;
    }
    if rt.check_event(EVENT_BOUGHT_MAGIKARP) {
        return Some(rt.print_text(text_at(salesman::NoRefundsText)).ret());
    }
    Some(rt.print_text(text_at(salesman::IGotADealText)).then(Label::DealOffered))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DealOffered => {
            rt.money_box();
            rt.yes_no_choice().then(Label::DealAnswered)
        }
        Label::DealAnswered => {
            if !rt.chose_yes() {
                return rt.print_text(text_at(salesman::NoText)).ret();
            }
            if !rt.has_enough_money(PRICE) {
                return rt.print_text(text_at(salesman::NoMoneyText)).ret();
            }
            rt.give_pokemon(PokemonSpecies::Magikarp, 5).then(Label::MagikarpGiven)
        }
        Label::MagikarpGiven => {
            if !rt.gave_pokemon() {
                return Flow::Return;
            }
            rt.subtract_money(PRICE);
            rt.money_box();
            rt.set_event(EVENT_BOUGHT_MAGIKARP);
            Flow::Return
        }
    }
}
