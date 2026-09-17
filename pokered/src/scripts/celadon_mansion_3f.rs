//! `CeladonMansion3F_Script`: the game designer, who hands out the diploma for a full Pokédex.

use poke_core::symbols::pokered_local_labels::CeladonMansion3FGameDesignerText as designer;
use poke_core::symbols::pokered_map_scripts::TEXT_CELADONMANSION3F_GAME_DESIGNER;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// `NUM_POKEMON - 1`: Mew is not counted, since nothing in the game gives it out.
const COMPLETE_DEX: u8 = 150;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DiplomaShown,
    DiplomaRead,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_CELADONMANSION3F_GAME_DESIGNER {
        return None;
    }
    Some(match rt.pokedex_owned() >= COMPLETE_DEX {
        true => rt.print_text(text_at(designer::CompletedDexText)).then(Label::DiplomaShown),
        false => rt.print_text(text_at(designer::Text)).ret(),
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DiplomaShown => rt.display_diploma().then(Label::DiplomaRead),
        // The diploma is its own screen, so there is nothing left to press a button through.
        Label::DiplomaRead => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
    }
}
