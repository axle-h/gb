//! `LavenderTown_Script`: nothing but the little girl, who asks whether ghosts are real and has an
//! answer either way.

use poke_core::symbols::pokered_map_scripts::TEXT_LAVENDERTOWN_LITTLE_GIRL;
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    LittleGirlYesNo,
    LittleGirlAnswered,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    
    match text_id {
        TEXT_LAVENDERTOWN_LITTLE_GIRL => {
            Some(rt.print_text(text_named("LavenderTownLittleGirlText.DoYouBelieveInGhostsText")).then(Label::LittleGirlYesNo))
        }
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    
    match label {
        Label::LittleGirlYesNo => rt.yes_no_choice().then(Label::LittleGirlAnswered),
        Label::LittleGirlAnswered => {
            let said = match rt.chose_yes() {
                true => "LavenderTownLittleGirlText.SoThereAreBelieversText",
                false => "LavenderTownLittleGirlText.HaHaGuessNotText",
            };
            rt.print_text(text_named(said)).ret()
        }
    }
}
