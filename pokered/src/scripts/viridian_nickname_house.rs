//! `ViridianNicknameHouse_Script`: Speary, who cries after its name.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::TEXT_VIRIDIANNICKNAMEHOUSE_SPEAROW;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    SpearowCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_VIRIDIANNICKNAMEHOUSE_SPEAROW)
        .then(|| rt.print_text(text_at(local::ViridianNicknameHouseSpearowText::Text)).then(Label::SpearowCry))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::SpearowCry => {
            rt.play_cry(PokemonSpecies::Spearow);
            rt.wait_for_sound_to_finish().ret()
        }
    }
}
