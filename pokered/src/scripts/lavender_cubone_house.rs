//! `LavenderCuboneHouse_Script`: the orphaned Cubone and the girl looking after it, who has
//! something else to say once Mr Fuji is home.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::EVENT_RESCUED_MR_FUJI;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_LAVENDERCUBONEHOUSE_BRUNETTE_GIRL, TEXT_LAVENDERCUBONEHOUSE_CUBONE};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    CuboneCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    use local::LavenderCuboneHouseBrunetteGirlText as girl;
    Some(match text_id {
        // The words come from the table as they are, and only the cry after them is code.
        TEXT_LAVENDERCUBONEHOUSE_CUBONE => {
            rt.print_text(text_at(sym::LavenderCuboneHouseCuboneText)).then(Label::CuboneCry)
        }
        TEXT_LAVENDERCUBONEHOUSE_BRUNETTE_GIRL => {
            let said = match rt.check_event(EVENT_RESCUED_MR_FUJI) {
                true => girl::TheGhostIsGoneText,
                false => girl::PoorCubonesMotherText,
            };
            rt.print_text(text_at(said)).ret()
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::CuboneCry => {
            rt.play_cry(PokemonSpecies::Cubone);
            rt.wait_for_sound_to_finish().ret()
        }
    }
}
