//! `SSAnneB1FRooms_Script`: the crew's cabins, five Sailors and a Fisherman, and a Machoke that cries
//! after its words.

use poke_core::tables::trainers;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_map_scripts::{TEXT_SSANNEB1FROOMS_FISHER, TEXT_SSANNEB1FROOMS_MACHOKE,
    TEXT_SSANNEB1FROOMS_SAILOR1, TEXT_SSANNEB1FROOMS_SAILOR2, TEXT_SSANNEB1FROOMS_SAILOR3, TEXT_SSANNEB1FROOMS_SAILOR4,
    TEXT_SSANNEB1FROOMS_SAILOR5};
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSSAnneB1FRoomsCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSSAnneB1FRoomsCurScript], a` after the table's routine.
    StoreCurScript,
    MachokeCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().ss_anne_b1f_rooms.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, trainers::SSAnne10TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SSANNEB1FROOMS_SAILOR1 => trainers::SSAnne10TrainerHeader0,
        TEXT_SSANNEB1FROOMS_SAILOR2 => trainers::SSAnne10TrainerHeader1,
        TEXT_SSANNEB1FROOMS_SAILOR3 => trainers::SSAnne10TrainerHeader2,
        TEXT_SSANNEB1FROOMS_SAILOR4 => trainers::SSAnne10TrainerHeader3,
        TEXT_SSANNEB1FROOMS_SAILOR5 => trainers::SSAnne10TrainerHeader4,
        TEXT_SSANNEB1FROOMS_FISHER => trainers::SSAnne10TrainerHeader5,
        TEXT_SSANNEB1FROOMS_MACHOKE => {
            return Some(rt.print_text(text_named("SSAnneB1FRoomsMachokeText")).then(Label::MachokeCry));
        }
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().ss_anne_b1f_rooms.cur_script = rt.cur_map_script();
            Flow::Return
        }
        // `PlayCry` waits for the cry itself.
        Label::MachokeCry => {
            rt.play_cry(PokemonSpecies::Machoke);
            rt.wait_for_sound_to_finish().ret()
        }
    }
}
