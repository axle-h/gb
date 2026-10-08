//! `NameRatersHouse_Script`: the name rater, who renames one party mon and refuses any whose
//! original trainer is not the player.

use poke_core::symbols::pokered_map_scripts::TEXT_NAMERATERSHOUSE_NAME_RATER;
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    RateYesNo,
    RateAnswered,
    PartyAsked,
    MonChosen,
    NiceNameYesNo(u8),
    NiceNameAnswered(u8),
    NameAsked(u8),
    Named(u8),
    DidNotRename,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    
    match text_id {
        TEXT_NAMERATERSHOUSE_NAME_RATER => {
            rt.save_screen_tiles();
            Some(rt.print_text(text_named("NameRatersHouseNameRaterText.WantMeToRateText")).then(Label::RateYesNo))
        }
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    
    match label {
        Label::RateYesNo => rt.yes_no_choice().then(Label::RateAnswered),
        Label::RateAnswered => match rt.chose_yes() {
            true => rt.print_text(text_named("NameRatersHouseNameRaterText.WhichPokemonText")).then(Label::PartyAsked),
            false => Flow::Jump(Label::DidNotRename.into()),
        },
        Label::PartyAsked => rt.display_party_menu().then(Label::MonChosen),
        Label::MonChosen => {
            let chosen = rt.chosen_party_mon();
            rt.restore_screen_tiles_and_reload_sprites();
            let Some(slot) = chosen else {
                return Flow::Jump(Label::DidNotRename.into());
            };
            rt.get_party_mon_name2(slot);
            if !rt.party_mon_is_players(slot) {
                return rt.print_text(text_named("NameRatersHouseNameRaterText.ATrulyImpeccableNameText")).ret();
            }
            rt.print_text(text_named("NameRatersHouseNameRaterText.GiveItANiceNameText")).then(Label::NiceNameYesNo(slot))
        }
        Label::NiceNameYesNo(slot) => rt.yes_no_choice().then(Label::NiceNameAnswered(slot)),
        Label::NiceNameAnswered(slot) => match rt.chose_yes() {
            true => rt.print_text(text_named("NameRatersHouseNameRaterText.WhatShouldWeNameItText")).then(Label::NameAsked(slot)),
            false => Flow::Jump(Label::DidNotRename.into()),
        },
        Label::NameAsked(slot) => rt.name_rater_screen(slot).then(Label::Named(slot)),
        Label::Named(slot) => {
            rt.restore_screen_tiles_and_reload_sprites();
            match rt.rename_party_mon(slot) {
                true => rt.print_text(text_named("NameRatersHouseNameRaterText.PokemonHasBeenRenamedText")).ret(),
                false => Flow::Jump(Label::DidNotRename.into()),
            }
        }
        Label::DidNotRename => rt.print_text(text_named("NameRatersHouseNameRaterText.ComeAnyTimeYouLikeText")).ret(),
    }
}
