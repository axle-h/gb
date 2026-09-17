//! `VictoryRoad1F_Script`: the switch a boulder has to be pushed onto to open the gate out, and the
//! two trainers in the way.

use poke_core::symbols::pokered_events::EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_VICTORYROAD1F_DEFAULT, TEXT_VICTORYROAD1F_COOLTRAINER_F,
    TEXT_VICTORYROAD1F_COOLTRAINER_M};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{Code, Flow, Script};

/// `.SwitchCoords`, as (x, y): the one square a boulder held down opens the gate.
const SWITCH: [(u8, u8); 1] = [(17, 13)];
/// The block the gate becomes, and where it stands in blocks.
const OPEN_GATE: u8 = 0x1D;
const GATE_AT: (u8, u8) = (4, 6);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wVictoryRoad1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    /// `ld [wVictoryRoad1FCurScript], a` after the table's routine.
    StoreCurScript,
}

pub fn script(rt: &mut Script) -> Flow {
    // `.next`: the gate is drawn open again on every load once the switch is held down, since a
    // replaced block is not saved.
    if rt.check_and_reset_cur_map_loaded(1) && rt.check_event(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH) {
        rt.replace_tile_block(GATE_AT.0, GATE_AT.1, OPEN_GATE);
    }
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().victory_road_1f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::VictoryRoad1TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_VICTORYROAD1F_DEFAULT => Label::DefaultScript.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `VictoryRoad1FDefaultScript`: the switch is read every pass until it is held down, and the map is
/// marked loaded again so the script above draws the gate open without a warp.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH) || rt.check_boulder_coords(&SWITCH).is_none() {
        return rt.trainer_script(SCRIPT_VICTORYROAD1F_DEFAULT).ret();
    }
    rt.set_cur_map_loaded(1);
    rt.set_event(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_VICTORYROAD1F_COOLTRAINER_F => sym::VictoryRoad1TrainerHeader0,
        TEXT_VICTORYROAD1F_COOLTRAINER_M => sym::VictoryRoad1TrainerHeader1,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::StoreCurScript => {
            rt.maps().victory_road_1f.cur_script = rt.cur_map_script();
            Flow::Return
        }
    }
}
