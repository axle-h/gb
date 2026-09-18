//! `SeafoamIslandsB3F_Script`: the two holes a boulder goes down to slow the current below, and the
//! current itself, which carries a surfing player back to the steps rather than letting them cross.

use poke_core::map::Map;
use poke_core::symbols::pokered_events::{EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE,
    EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE};
use poke_core::symbols::pokered_local_labels::SeafoamIslandsB3FMoveObjectScript;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SEAFOAMISLANDSB3F_DEFAULT, SCRIPT_SEAFOAMISLANDSB3F_MOVE_OBJECT,
    SCRIPT_SEAFOAMISLANDSB3F_OBJECT_MOVING1, SCRIPT_SEAFOAMISLANDSB3F_OBJECT_MOVING2};
use poke_core::symbols::pokered_symbols::RLEList_ForcedSurfingStrongCurrentNearSteps;
use poke_core::symbols::pokered_toggles::{TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_2};
use serde::{Deserialize, Serialize};
use super::seafoam_islands::{boulders_and_holes, Floor};
use super::{Flow, Script};

/// `Seafoam4HolesCoords`: the floor's own two holes, which a boulder or the player falls down.
const FLOOR: Floor = Floor {
    holes: [(3, 16), (6, 16)],
    down_hole: [EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE],
    hide: [TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_2],
    show: [TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B4F_BOULDER_2],
    below: Map::SeafoamIslandsB4F,
};
/// The square at the foot of the steps, where the current takes over.
const NEAR_STEPS: (u8, u8) = (15, 8);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSeafoamIslandsB3FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    if !boulders_and_holes(rt, &FLOOR) {
        return Flow::Return;
    }
    run_current_map_script(rt)
}

fn run_current_map_script(rt: &mut Script) -> Flow {
    match rt.maps().seafoam_islands_b3f.cur_script {
        SCRIPT_SEAFOAMISLANDSB3F_DEFAULT => default_script(rt),
        SCRIPT_SEAFOAMISLANDSB3F_MOVE_OBJECT => move_object_script(rt),
        // Both `ObjectMoving` scripts do nothing but wait for the presses to run out.
        _ => {
            if rt.simulated_joypad_states_index() == 0 {
                rt.maps().seafoam_islands_b3f.cur_script = SCRIPT_SEAFOAMISLANDSB3F_DEFAULT;
            }
            Flow::Return
        }
    }
}

/// `SeafoamIslandsB3FDefaultScript`: once both of this floor's boulders are down its own holes, a
/// player who surfs up to the steps is swept back down to them.
fn default_script(rt: &mut Script) -> Flow {
    if !boulders_down(rt) || (rt.x(), rt.y()) != NEAR_STEPS {
        return Flow::Return;
    }
    rt.simulate_joypad_rle(RLEList_ForcedSurfingStrongCurrentNearSteps);
    rt.set_forced_warp(true);
    rt.maps().seafoam_islands_b3f.cur_script = SCRIPT_SEAFOAMISLANDSB3F_OBJECT_MOVING1;
    Flow::Return
}

/// `SeafoamIslandsB3FMoveObjectScript`, which `CheckForceBikeOrSurf` arms as the player lands in the
/// water beside one of the boulders: the column they fell down says which way the current runs.
fn move_object_script(rt: &mut Script) -> Flow {
    if !boulders_down(rt) {
        return Flow::Return;
    }
    let list = match rt.x() {
        18 => SeafoamIslandsB3FMoveObjectScript::RLEList_StrongCurrentNearLeftBoulder,
        19 => SeafoamIslandsB3FMoveObjectScript::RLEList_StrongCurrentNearRightBoulder,
        _ => {
            rt.maps().seafoam_islands_b3f.cur_script = SCRIPT_SEAFOAMISLANDSB3F_DEFAULT;
            return Flow::Return;
        }
    };
    rt.simulate_joypad_rle(list);
    rt.set_forced_warp(true);
    rt.maps().seafoam_islands_b3f.cur_script = SCRIPT_SEAFOAMISLANDSB3F_OBJECT_MOVING2;
    Flow::Return
}

/// The boulders of the floor above, which are the ones lying in this floor's water.
fn boulders_down(rt: &Script) -> bool {
    rt.check_event(EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE) && rt.check_event(EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE)
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
