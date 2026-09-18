//! `SSAnne1FRooms_Script`: the first-class cabins, four trainers among the passengers, and a
//! Wigglytuff that cries after its words.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_map_scripts::{TEXT_SSANNE1FROOMS_COOLTRAINER_F, TEXT_SSANNE1FROOMS_GENTLEMAN1,
    TEXT_SSANNE1FROOMS_GENTLEMAN2, TEXT_SSANNE1FROOMS_WIGGLYTUFF, TEXT_SSANNE1FROOMS_YOUNGSTER};
use poke_core::symbols::pokered_symbols::{SSAnne1FRoomsWigglytuffText, SSAnne8TrainerHeader0, SSAnne8TrainerHeader1,
    SSAnne8TrainerHeader2, SSAnne8TrainerHeader3, SSAnne8TrainerHeaders};
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSSAnne1FRoomsCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSSAnne1FRoomsCurScript], a` after the table's routine.
    StoreCurScript,
    WigglytuffCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().ss_anne_1f_rooms.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, SSAnne8TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SSANNE1FROOMS_GENTLEMAN1 => SSAnne8TrainerHeader0,
        TEXT_SSANNE1FROOMS_GENTLEMAN2 => SSAnne8TrainerHeader1,
        TEXT_SSANNE1FROOMS_YOUNGSTER => SSAnne8TrainerHeader2,
        TEXT_SSANNE1FROOMS_COOLTRAINER_F => SSAnne8TrainerHeader3,
        TEXT_SSANNE1FROOMS_WIGGLYTUFF => {
            return Some(rt.print_text(text_at(SSAnne1FRoomsWigglytuffText)).then(Label::WigglytuffCry));
        }
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().ss_anne_1f_rooms.cur_script = rt.cur_map_script();
            Flow::Return
        }
        // `PlayCry` waits for the cry itself.
        Label::WigglytuffCry => {
            rt.play_cry(PokemonSpecies::Wigglytuff);
            rt.wait_for_sound_to_finish().ret()
        }
    }
}
