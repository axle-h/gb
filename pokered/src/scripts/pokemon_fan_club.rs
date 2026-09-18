//! `PokemonFanClub_Script`: the two members who out-boast each other, their Pokémon's cries, and
//! the chairman, who gives a Bike Voucher to anyone who hears his story out.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_GOT_BIKE_VOUCHER, EVENT_PIKACHU_FAN_BOAST, EVENT_SEEL_FAN_BOAST};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_POKEMONFANCLUB_CHAIRMAN, TEXT_POKEMONFANCLUB_PIKACHU,
    TEXT_POKEMONFANCLUB_PIKACHU_FAN, TEXT_POKEMONFANCLUB_SEEL, TEXT_POKEMONFANCLUB_SEEL_FAN};
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};
use local::PokemonFanClubChairmanText as chairman;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// After each fan's `.NormalText` and `.BetterText`.
    PikachuFanNormal,
    PikachuFanBetter,
    SeelFanNormal,
    SeelFanBetter,
    /// After `PokemonFanClubPikachuText.Text` and `PokemonFanClubSeelText.Text`.
    PikachuCry,
    SeelCry,
    /// After the chairman's `.IntroText`, after the yes/no, and after `.StoryText`.
    ChairmanIntro,
    ChairmanAnswered,
    StoryTold,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        // Each fan's plain boast arms the other's comeback, and the comeback disarms itself, each after
        // its text is closed.
        TEXT_POKEMONFANCLUB_PIKACHU_FAN => match rt.check_event(EVENT_PIKACHU_FAN_BOAST) {
            true => rt.print_text(text_at(local::PokemonFanClubPikachuFanText::BetterText)).then(Label::PikachuFanBetter),
            false => rt.print_text(text_at(local::PokemonFanClubPikachuFanText::NormalText)).then(Label::PikachuFanNormal),
        },
        TEXT_POKEMONFANCLUB_SEEL_FAN => match rt.check_event(EVENT_SEEL_FAN_BOAST) {
            true => rt.print_text(text_at(local::PokemonFanClubSeelFanText::BetterText)).then(Label::SeelFanBetter),
            false => rt.print_text(text_at(local::PokemonFanClubSeelFanText::NormalText)).then(Label::SeelFanNormal),
        },
        TEXT_POKEMONFANCLUB_PIKACHU => {
            rt.print_text(text_at(local::PokemonFanClubPikachuText::Text)).then(Label::PikachuCry)
        }
        TEXT_POKEMONFANCLUB_SEEL => rt.print_text(text_at(local::PokemonFanClubSeelText::Text)).then(Label::SeelCry),
        TEXT_POKEMONFANCLUB_CHAIRMAN => {
            if check_bike_in_bag(rt) {
                return Some(rt.print_text(text_at(chairman::FinalText)).ret());
            }
            rt.print_text(text_at(chairman::IntroText)).then(Label::ChairmanIntro)
        }
        _ => return None,
    })
}

/// `PokemonFanClub_CheckBikeInBag`: the voucher had once, or either bike paraphernalia still held.
fn check_bike_in_bag(rt: &Script) -> bool {
    rt.check_event(EVENT_GOT_BIKE_VOUCHER) || rt.is_item_in_bag(ItemId::Bicycle) || rt.is_item_in_bag(ItemId::BikeVoucher)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::PikachuFanNormal => {
            rt.set_event(EVENT_SEEL_FAN_BOAST);
            Flow::Return
        }
        Label::PikachuFanBetter => {
            rt.reset_event(EVENT_PIKACHU_FAN_BOAST);
            Flow::Return
        }
        Label::SeelFanNormal => {
            rt.set_event(EVENT_PIKACHU_FAN_BOAST);
            Flow::Return
        }
        Label::SeelFanBetter => {
            rt.reset_event(EVENT_SEEL_FAN_BOAST);
            Flow::Return
        }
        Label::PikachuCry => {
            rt.play_cry(PokemonSpecies::Pikachu);
            rt.wait_for_sound_to_finish().ret()
        }
        Label::SeelCry => {
            rt.play_cry(PokemonSpecies::Seel);
            rt.wait_for_sound_to_finish().ret()
        }
        Label::ChairmanIntro => rt.yes_no_choice().then(Label::ChairmanAnswered),
        Label::ChairmanAnswered => {
            if !rt.chose_yes() {
                return rt.print_text(text_at(chairman::NoStoryText)).ret();
            }
            rt.print_text(text_at(chairman::StoryText)).then(Label::StoryTold)
        }
        Label::StoryTold => {
            if !rt.give_item(ItemId::BikeVoucher, 1) {
                return rt.print_text(text_at(chairman::BagFullText)).ret();
            }
            rt.set_event(EVENT_GOT_BIKE_VOUCHER);
            rt.print_text(text_at(chairman::BikeVoucherText)).ret()
        }
    }
}
