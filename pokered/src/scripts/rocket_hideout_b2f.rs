//! `RocketHideoutB2F_Script`: the arrow tiles that slide the player across the floor, and the Rocket
//! who guards it.

use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROCKETHIDEOUTB2F_DEFAULT, SCRIPT_ROCKETHIDEOUTB2F_PLAYER_SPINNING,
    TEXT_ROCKETHIDEOUTB2F_ROCKET};
use poke_core::symbols::pokered_symbols::{RocketHideout2ArrowTilePlayerMovement, RocketHideout2TrainerHeader0,
    RocketHideout2TrainerHeaders};
use serde::{Deserialize, Serialize};
use crate::modes::overworld::spinners::{arrow_tile_default, player_spinning};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRocketHideoutB2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRocketHideoutB2FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().rocket_hideout_b2f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, RocketHideout2TrainerHeaders);
    let then = match index {
        SCRIPT_ROCKETHIDEOUTB2F_DEFAULT => {
            arrow_tile_default(rt, RocketHideout2ArrowTilePlayerMovement, SCRIPT_ROCKETHIDEOUTB2F_PLAYER_SPINNING)
        }
        SCRIPT_ROCKETHIDEOUTB2F_PLAYER_SPINNING => {
            player_spinning(rt, SCRIPT_ROCKETHIDEOUTB2F_DEFAULT);
            None
        }
        _ => Some(rt.trainer_script(index)),
    };
    match then {
        Some(then) => then.then(Label::StoreCurScript),
        None => resume(rt, Label::StoreCurScript),
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_ROCKETHIDEOUTB2F_ROCKET).then(|| rt.talk_to_trainer(RocketHideout2TrainerHeader0).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().rocket_hideout_b2f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
