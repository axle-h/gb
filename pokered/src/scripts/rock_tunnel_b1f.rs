//! `RockTunnelB1F_Script`: eight trainers on the floor below, and nothing else.

use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRockTunnelB1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRockTunnelB1FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().rock_tunnel_b1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::RockTunnel2TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROCKTUNNELB1F_COOLTRAINER_F1 => sym::RockTunnel2TrainerHeader0,
        TEXT_ROCKTUNNELB1F_HIKER1 => sym::RockTunnel2TrainerHeader1,
        TEXT_ROCKTUNNELB1F_SUPER_NERD1 => sym::RockTunnel2TrainerHeader2,
        TEXT_ROCKTUNNELB1F_SUPER_NERD2 => sym::RockTunnel2TrainerHeader3,
        TEXT_ROCKTUNNELB1F_HIKER2 => sym::RockTunnel2TrainerHeader4,
        TEXT_ROCKTUNNELB1F_COOLTRAINER_F2 => sym::RockTunnel2TrainerHeader5,
        TEXT_ROCKTUNNELB1F_HIKER3 => sym::RockTunnel2TrainerHeader6,
        TEXT_ROCKTUNNELB1F_SUPER_NERD3 => sym::RockTunnel2TrainerHeader7,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().rock_tunnel_b1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
