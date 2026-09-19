//! `CeladonMansion1F_Script`: the manager's three Pokémon, each of which cries after its text.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_map_scripts::{TEXT_CELADONMANSION1F_CLEFAIRY, TEXT_CELADONMANSION1F_MEOWTH,
    TEXT_CELADONMANSION1F_NIDORANF};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    MeowthCry,
    ClefairyCry,
    NidoranFCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_CELADONMANSION1F_MEOWTH => rt.print_text(text_at(sym::CeladonMansion1FMeowthText)).then(Label::MeowthCry),
        TEXT_CELADONMANSION1F_CLEFAIRY => {
            rt.print_text(text_at(sym::CeladonMansion1FClefairyText)).then(Label::ClefairyCry)
        }
        TEXT_CELADONMANSION1F_NIDORANF => {
            rt.print_text(text_at(sym::CeladonMansion1FNidoranFText)).then(Label::NidoranFCry)
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    let species = match label {
        Label::MeowthCry => PokemonSpecies::Meowth,
        Label::ClefairyCry => PokemonSpecies::Clefairy,
        Label::NidoranFCry => PokemonSpecies::NidoranFemale,
    };
    rt.play_cry(species);
    rt.wait_for_sound_to_finish().ret()
}
