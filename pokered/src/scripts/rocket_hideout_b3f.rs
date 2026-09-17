//! `RocketHideoutB3F_Script`: the arrow tiles that slide the player across the floor, and the two
//! Rockets who guard it.

use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROCKETHIDEOUTB3F_DEFAULT, SCRIPT_ROCKETHIDEOUTB3F_PLAYER_SPINNING,
    TEXT_ROCKETHIDEOUTB3F_ROCKET1, TEXT_ROCKETHIDEOUTB3F_ROCKET2};
use poke_core::symbols::pokered_symbols::{RocketHideout3ArrowTilePlayerMovement, RocketHideout3TrainerHeader0,
    RocketHideout3TrainerHeader1, RocketHideout3TrainerHeaders};
use serde::{Deserialize, Serialize};
use crate::modes::overworld::spinners::{arrow_tile_default, player_spinning};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRocketHideoutB3FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRocketHideoutB3FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().rocket_hideout_b3f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, RocketHideout3TrainerHeaders);
    let then = match index {
        SCRIPT_ROCKETHIDEOUTB3F_DEFAULT => {
            arrow_tile_default(rt, RocketHideout3ArrowTilePlayerMovement, SCRIPT_ROCKETHIDEOUTB3F_PLAYER_SPINNING)
        }
        SCRIPT_ROCKETHIDEOUTB3F_PLAYER_SPINNING => {
            player_spinning(rt, SCRIPT_ROCKETHIDEOUTB3F_DEFAULT);
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
    let header = match text_id {
        TEXT_ROCKETHIDEOUTB3F_ROCKET1 => RocketHideout3TrainerHeader0,
        TEXT_ROCKETHIDEOUTB3F_ROCKET2 => RocketHideout3TrainerHeader1,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().rocket_hideout_b3f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
