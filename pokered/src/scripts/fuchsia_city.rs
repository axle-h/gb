//! `FuchsiaCity_Script`: the signs in front of the warden's exhibits, each of which opens the Pokédex
//! page of what stands behind it.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_GOT_DOME_FOSSIL, EVENT_GOT_HELIX_FOSSIL};
use poke_core::symbols::pokered_map_scripts::{TEXT_FUCHSIACITY_CHANSEY_SIGN, TEXT_FUCHSIACITY_FOSSIL_SIGN,
    TEXT_FUCHSIACITY_KANGASKHAN_SIGN, TEXT_FUCHSIACITY_LAPRAS_SIGN, TEXT_FUCHSIACITY_SLOWPOKE_SIGN,
    TEXT_FUCHSIACITY_VOLTORB_SIGN};
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `DisplayPokedex` after the sign's own words.
    ShowDex(PokemonSpecies),
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

fn sign(rt: &mut Script, words: &'static str, species: PokemonSpecies) -> Flow {
    rt.print_text(text_named(words)).then(Label::ShowDex(species))
}

/// `FuchsiaCityFossilSignText`: the pen holds whichever fossil the player left in Mt Moon, and reads
/// as undetermined until one of them has been picked up.
fn fossil_sign(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_GOT_DOME_FOSSIL) {
        return sign(rt, "FuchsiaCityFossilSignText.OmanyteText", PokemonSpecies::Omanyte);
    }
    if rt.check_event(EVENT_GOT_HELIX_FOSSIL) {
        return sign(rt, "FuchsiaCityFossilSignText.KabutoText", PokemonSpecies::Kabuto);
    }
    rt.print_text(text_named("FuchsiaCityFossilSignText.UndeterminedText")).ret()
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let (words, species) = match text_id {
        TEXT_FUCHSIACITY_CHANSEY_SIGN => ("FuchsiaCityChanseySignText.Text", PokemonSpecies::Chansey),
        TEXT_FUCHSIACITY_VOLTORB_SIGN => ("FuchsiaCityVoltorbSignText.Text", PokemonSpecies::Voltorb),
        TEXT_FUCHSIACITY_KANGASKHAN_SIGN => ("FuchsiaCityKangaskhanSignText.Text", PokemonSpecies::Kangaskhan),
        TEXT_FUCHSIACITY_SLOWPOKE_SIGN => ("FuchsiaCitySlowpokeSignText.Text", PokemonSpecies::Slowpoke),
        TEXT_FUCHSIACITY_LAPRAS_SIGN => ("FuchsiaCityLaprasSignText.Text", PokemonSpecies::Lapras),
        TEXT_FUCHSIACITY_FOSSIL_SIGN => return Some(fossil_sign(rt)),
        _ => return None,
    };
    Some(sign(rt, words, species))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::ShowDex(species) => rt.display_pokedex(species).ret(),
    }
}
