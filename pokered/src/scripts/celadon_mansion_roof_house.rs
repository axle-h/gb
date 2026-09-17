//! `CeladonMansionRoofHouse_Script`: the Eevee in its ball, which stays where it is if the party
//! has no room for it.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_map_scripts::TEXT_CELADONMANSION_ROOF_HOUSE_EEVEE_POKEBALL;
use poke_core::symbols::pokered_toggles::TOGGLE_CELADON_MANSION_EEVEE_GIFT;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    EeveeGiven,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_CELADONMANSION_ROOF_HOUSE_EEVEE_POKEBALL)
        .then(|| rt.give_pokemon(PokemonSpecies::Eevee, 25).then(Label::EeveeGiven))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::EeveeGiven => {
            if rt.added_to_party() {
                rt.hide_object(TOGGLE_CELADON_MANSION_EEVEE_GIFT);
            }
            Flow::Return
        }
    }
}
