//! `RedsHouse2F_Script`: the new game's first pass faces the player up.

use poke_core::symbols::pokered_map_scripts::{SCRIPT_REDSHOUSE2F_DEFAULT, SCRIPT_REDSHOUSE2F_NOOP};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `PLAYER_DIR_UP`.
const PLAYER_DIR_UP: u8 = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRedsHouse2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    if rt.maps().reds_house_2f.cur_script == SCRIPT_REDSHOUSE2F_DEFAULT {
        // `RedsHouse2FDefaultScript`.
        rt.clear_joy_held();
        rt.set_player_moving_direction(PLAYER_DIR_UP);
        rt.maps().reds_house_2f.cur_script = SCRIPT_REDSHOUSE2F_NOOP;
    }
    Flow::Return
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
