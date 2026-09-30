//! `VermilionPidgeyHouse_Script`: the Pidgey that cries after its words.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_map_scripts::TEXT_VERMILIONPIDGEYHOUSE_PIDGEY;
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    PidgeyCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    // The words come from the table as they are, and only the cry after them is code.
    (text_id == TEXT_VERMILIONPIDGEYHOUSE_PIDGEY)
        .then(|| rt.print_text(text_named("VermilionPidgeyHousePidgeyText")).then(Label::PidgeyCry))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::PidgeyCry => {
            rt.play_cry(PokemonSpecies::Pidgey);
            rt.wait_for_sound_to_finish().ret()
        }
    }
}
