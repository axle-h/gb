//! `RockTunnel1F_Script`: seven trainers in the dark, and nothing else.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRockTunnel1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRockTunnel1FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().rock_tunnel_1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::RockTunnel1TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROCKTUNNEL1F_HIKER1 => sym::RockTunnel1TrainerHeader0,
        TEXT_ROCKTUNNEL1F_HIKER2 => sym::RockTunnel1TrainerHeader1,
        TEXT_ROCKTUNNEL1F_HIKER3 => sym::RockTunnel1TrainerHeader2,
        TEXT_ROCKTUNNEL1F_SUPER_NERD => sym::RockTunnel1TrainerHeader3,
        TEXT_ROCKTUNNEL1F_COOLTRAINER_F1 => sym::RockTunnel1TrainerHeader4,
        TEXT_ROCKTUNNEL1F_COOLTRAINER_F2 => sym::RockTunnel1TrainerHeader5,
        TEXT_ROCKTUNNEL1F_COOLTRAINER_F3 => sym::RockTunnel1TrainerHeader6,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().rock_tunnel_1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
