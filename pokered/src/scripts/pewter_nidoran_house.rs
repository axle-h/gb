//! `PewterNidoranHouse_Script`: the Nidoran♂ that cries after being told to sit.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_map_scripts::TEXT_PEWTERNIDORANHOUSE_NIDORAN;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    NidoranCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    // The words come from the table as they are, and only the cry after them is code.
    (text_id == TEXT_PEWTERNIDORANHOUSE_NIDORAN)
        .then(|| rt.print_text(text_at(sym::PewterNidoranHouseNidoranText)).then(Label::NidoranCry))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::NidoranCry => {
            rt.play_cry(PokemonSpecies::NidoranMale);
            rt.wait_for_sound_to_finish().ret()
        }
    }
}
