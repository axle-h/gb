//! `SeafoamIslandsB4F_Script`: the bottom floor's current, which pushes a surfing player back out of
//! the water at the foot of the steps and, once both boulders are in, round the whirlpool to Articuno.

use poke_core::symbols::pokered_events::{EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE,
    EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE};
use poke_core::symbols::pokered_local_labels::SeafoamIslandsB4FMoveObjectScript;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SEAFOAMISLANDSB4F_DEFAULT, SCRIPT_SEAFOAMISLANDSB4F_MOVE_OBJECT,
    SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING1, SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING2,
    SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING3, TEXT_SEAFOAMISLANDSB4F_ARTICUNO};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::systems::overworld::location::WALKING;
use super::{Flow, Script};

/// `SeafoamIslandsB4FDefaultScript.Coords`, as (x, y): the water in front of the exit, one row of
/// which is two steps from dry land and the other one.
const SURF_EXIT: [(u8, u8); 4] = [(20, 17), (21, 17), (20, 16), (21, 16)];
/// `SeafoamIslandsB4FMoveObjectScript.Coords`: where a player who has just fallen down a hole lands.
const STRONG_CURRENT: [(u8, u8); 2] = [(4, 14), (5, 14)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSeafoamIslandsB4FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSeafoamIslandsB4FCurScript], a` after Articuno's text has armed its battle.
    ArticunoTalkedTo,
    /// The same store after `EndTrainerBattle`, back to the current's own script.
    BattleEnded,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().seafoam_islands_b4f.cur_script {
        SCRIPT_SEAFOAMISLANDSB4F_DEFAULT => default_script(rt),
        SCRIPT_SEAFOAMISLANDSB4F_MOVE_OBJECT => move_object_script(rt),
        SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING1 => object_moving1(rt),
        SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING2 => object_moving2(rt),
        SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING3 => object_moving3(rt),
        _ => Flow::Return,
    }
}

/// `SeafoamIslandsB4FResetScript`.
fn reset_script(rt: &mut Script) -> Flow {
    rt.maps().seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_DEFAULT;
    rt.joy_ignore(Joypad::empty());
    Flow::Return
}

/// `SeafoamIslandsB4FObjectMoving3Script`: Articuno's battle is over, so its flag is set and its
/// perch cleared, unless the player lost and the whole floor starts again.
fn object_moving3(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_script(rt);
    }
    rt.end_trainer_battle().then(Label::BattleEnded)
}

/// `SeafoamIslandsB4FDefaultScript`: the current pushes a player who surfs up to the exit out of the
/// water rather than letting them float there.
fn default_script(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE) || !rt.check_event(EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE) {
        return Flow::Return;
    }
    let Some(index) = rt.are_player_coords_in_array(&SURF_EXIT) else {
        return Flow::Return;
    };
    let steps = if index >= 3 { 1 } else { 2 };
    rt.simulate_joypad_presses(vec![Joypad::UP; steps]);
    rt.set_forced_warp(false);
    rt.maps().seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING1;
    Flow::Return
}

/// `SeafoamIslandsB4FMoveObjectScript`, armed by `CheckForceBikeOrSurf` as the player drops into the
/// water: the boulders have plugged the whirlpool, so the current carries them round it.
fn move_object_script(rt: &mut Script) -> Flow {
    let plugged = rt.check_event(EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE) && rt.check_event(EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE);
    let list = match rt.are_player_coords_in_array(&STRONG_CURRENT).filter(|_| plugged) {
        Some(1) => SeafoamIslandsB4FMoveObjectScript::RLEList_StrongCurrentNearLeftBoulder,
        Some(_) => SeafoamIslandsB4FMoveObjectScript::RLEList_StrongCurrentNearRightBoulder,
        None => {
            rt.maps().seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_DEFAULT;
            return Flow::Return;
        }
    };
    rt.simulate_joypad_rle(list);
    rt.maps().seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING2;
    Flow::Return
}

fn object_moving1(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.maps().seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_DEFAULT;
    Flow::Return
}

/// `SeafoamIslandsB4FObjectMoving2Script`: the player is put back on their feet one press before the
/// end, so the last of the current's presses is the step onto dry land.
fn object_moving2(rt: &mut Script) -> Flow {
    let index = rt.simulated_joypad_states_index();
    if index == 1 {
        return rt.force_bike_or_surf(WALKING).ret();
    }
    if index != 0 {
        return Flow::Return;
    }
    rt.maps().seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_DEFAULT;
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_SEAFOAMISLANDSB4F_ARTICUNO {
        return None;
    }
    // Articuno's object carries a species and a level rather than a trainer, so its header's zero
    // opponent starts a wild battle where a trainer's would start a trainer one.
    Some(rt.talk_to_trainer(sym::ArticunoTrainerHeader).then(Label::ArticunoTalkedTo))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    let next = match label {
        Label::ArticunoTalkedTo => SCRIPT_SEAFOAMISLANDSB4F_OBJECT_MOVING3,
        Label::BattleEnded => SCRIPT_SEAFOAMISLANDSB4F_DEFAULT,
    };
    rt.maps().seafoam_islands_b4f.cur_script = next;
    Flow::Return
}
