//! `MtMoon1F_Script`: seven trainers in the first cave, and the item balls `PickUpItemText` takes
//! care of.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{MtMoon1TrainerHeader0, MtMoon1TrainerHeader1, MtMoon1TrainerHeader2,
    MtMoon1TrainerHeader3, MtMoon1TrainerHeader4, MtMoon1TrainerHeader5, MtMoon1TrainerHeader6,
    MtMoon1TrainerHeaders};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wMtMoon1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wMtMoon1FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().mt_moon_1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, MtMoon1TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_MTMOON1F_HIKER => MtMoon1TrainerHeader0,
        TEXT_MTMOON1F_YOUNGSTER1 => MtMoon1TrainerHeader1,
        TEXT_MTMOON1F_COOLTRAINER_F1 => MtMoon1TrainerHeader2,
        TEXT_MTMOON1F_SUPER_NERD => MtMoon1TrainerHeader3,
        TEXT_MTMOON1F_COOLTRAINER_F2 => MtMoon1TrainerHeader4,
        TEXT_MTMOON1F_YOUNGSTER2 => MtMoon1TrainerHeader5,
        TEXT_MTMOON1F_YOUNGSTER3 => MtMoon1TrainerHeader6,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().mt_moon_1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
