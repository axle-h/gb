//! `VictoryRoad3F_Script`: the switch that opens this floor's gate, the hole a boulder is dropped
//! down to land beside the switch on the floor below, and the four trainers.

use poke_core::map::Map;
use poke_core::symbols::pokered_events::{EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH1,
    EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH2};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_VICTORYROAD3F_DEFAULT, TEXT_VICTORYROAD3F_COOLTRAINER_F1,
    TEXT_VICTORYROAD3F_COOLTRAINER_F2, TEXT_VICTORYROAD3F_COOLTRAINER_M1, TEXT_VICTORYROAD3F_COOLTRAINER_M2};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::{TOGGLE_VICTORY_ROAD_2F_BOULDER, TOGGLE_VICTORY_ROAD_3F_BOULDER};
use serde::{Deserialize, Serialize};
use super::{Code, Flow, Script};

/// `.SwitchOrHoleCoords`, as (x, y): the switch first and the hole second. One list serves both the
/// boulder's squares and the player's, so the player standing on the switch has to be told apart
/// from the player standing on the hole.
const SWITCH: usize = 0;
const SWITCH_OR_HOLE: [(u8, u8); 2] = [(3, 5), (23, 15)];
/// `DungeonWarpList`'s row for this hole, which counts from one.
const HOLE_WARP: u8 = 2;
/// The block the gate becomes, and where it stands in blocks.
const OPEN_GATE: u8 = 0x1D;
const GATE_AT: (u8, u8) = (3, 5);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wVictoryRoad3FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    /// `ld [wVictoryRoad3FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(1) && rt.check_event(EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH1) {
        rt.replace_tile_block(GATE_AT.0, GATE_AT.1, OPEN_GATE);
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().victory_road_3f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::VictoryRoad3TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_VICTORYROAD3F_DEFAULT => Label::DefaultScript.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `VictoryRoad3FDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_pushed_boulder()
        && let Some(square) = rt.check_boulder_coords(&SWITCH_OR_HOLE)
    {
        if square == SWITCH {
            rt.set_cur_map_loaded(1);
            rt.set_event(EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH1);
            return Flow::Return;
        }
        // The boulder down the hole is this floor's own, hidden here and shown on the floor below,
        // where it is the one that reaches that floor's switch.
        if !rt.check_and_set_event(EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH2) {
            rt.hide_object(TOGGLE_VICTORY_ROAD_3F_BOULDER);
            rt.show_object(TOGGLE_VICTORY_ROAD_2F_BOULDER);
            return Flow::Return;
        }
    }
    // `IsPlayerOnDungeonWarp` over the same list: the player falls down the hole, and standing on
    // the switch leaves them where they are.
    match rt.are_player_coords_in_array(&SWITCH_OR_HOLE) {
        Some(HOLE_WARP) => {
            rt.fall_down_hole(Map::VictoryRoad2F, HOLE_WARP);
            Flow::Return
        }
        Some(_) => Flow::Return,
        None => rt.trainer_script(SCRIPT_VICTORYROAD3F_DEFAULT).ret(),
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_VICTORYROAD3F_COOLTRAINER_M1 => sym::VictoryRoad3TrainerHeader0,
        TEXT_VICTORYROAD3F_COOLTRAINER_F1 => sym::VictoryRoad3TrainerHeader1,
        TEXT_VICTORYROAD3F_COOLTRAINER_M2 => sym::VictoryRoad3TrainerHeader2,
        TEXT_VICTORYROAD3F_COOLTRAINER_F2 => sym::VictoryRoad3TrainerHeader3,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::StoreCurScript => {
            rt.maps().victory_road_3f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
