//! `SSAnne2FRooms_Script`: the second-class cabins, which draw no box of their own for a text, four
//! trainers, the passengers who print their words in a box from code, and a gentleman who shows the
//! Snorlax he saw.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_SSANNE2FROOMS_BEAUTY, TEXT_SSANNE2FROOMS_BRUNETTE_GIRL,
    TEXT_SSANNE2FROOMS_COOLTRAINER_F, TEXT_SSANNE2FROOMS_FISHER, TEXT_SSANNE2FROOMS_GENTLEMAN1,
    TEXT_SSANNE2FROOMS_GENTLEMAN2, TEXT_SSANNE2FROOMS_GENTLEMAN3, TEXT_SSANNE2FROOMS_GENTLEMAN4,
    TEXT_SSANNE2FROOMS_GENTLEMAN5, TEXT_SSANNE2FROOMS_GRAMPS, TEXT_SSANNE2FROOMS_LITTLE_BOY};
use poke_core::symbols::pokered_symbols::{SSAnne9TrainerHeader0, SSAnne9TrainerHeader1, SSAnne9TrainerHeader2,
    SSAnne9TrainerHeader3, SSAnne9TrainerHeaders};
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSSAnne2FRoomsCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSSAnne2FRoomsCurScript], a` after the table's routine.
    StoreCurScript,
    /// `LoadScreenTilesFromBuffer1` and `DisplayPokedex` after the gentleman's words.
    ShowSnorlax,
}

pub fn script(rt: &mut Script) -> Flow {
    // `BIT_NO_AUTO_TEXT_BOX` and `wDoNotWaitForButtonPressAfterDisplayingText` written by hand.
    rt.disable_auto_text_box_drawing();
    let index = rt.maps().ss_anne_2f_rooms.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, SSAnne9TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SSANNE2FROOMS_GENTLEMAN1 => SSAnne9TrainerHeader0,
        TEXT_SSANNE2FROOMS_FISHER => SSAnne9TrainerHeader1,
        TEXT_SSANNE2FROOMS_GENTLEMAN2 => SSAnne9TrainerHeader2,
        TEXT_SSANNE2FROOMS_COOLTRAINER_F => SSAnne9TrainerHeader3,
        TEXT_SSANNE2FROOMS_GENTLEMAN3 => {
            // The screen he saves is put back before the Pokédex opens, so the box he printed in goes
            // with it; buffer 2 stands in for the cartridge's buffer 1, nothing else holding it here.
            rt.save_screen_tiles();
            let words = text_at(local::SSAnne2FRoomsGentleman3Text::Text);
            return Some(rt.print_text(words).then(Label::ShowSnorlax));
        }
        _ => {
            let words = match text_id {
                TEXT_SSANNE2FROOMS_GENTLEMAN4 => local::SSAnne2FRoomsGentleman4Text::Text,
                TEXT_SSANNE2FROOMS_GRAMPS => local::SSAnne2FRoomsGrampsText::Text,
                TEXT_SSANNE2FROOMS_GENTLEMAN5 => local::SSAnne2FRoomsGentleman5Text::Text,
                TEXT_SSANNE2FROOMS_LITTLE_BOY => local::SSAnne2FRoomsLittleBoyText::Text,
                TEXT_SSANNE2FROOMS_BRUNETTE_GIRL => local::SSAnne2FRoomsBrunetteGirlText::Text,
                TEXT_SSANNE2FROOMS_BEAUTY => local::SSAnne2FRoomsBeautyText::Text,
                _ => return None,
            };
            return Some(rt.print_text(text_at(words)).ret());
        }
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().ss_anne_2f_rooms.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ShowSnorlax => {
            rt.restore_screen_tiles();
            rt.display_pokedex(PokemonSpecies::Snorlax).ret()
        }
    }
}
