//! `Route18Gate1F_Script`: the northern gate of the Cycling Road, the southern one's script over
//! again with its own coordinates and its own two lines from the guard.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_local_labels::Route18Gate1FGuardText;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE18GATE1F_DEFAULT, SCRIPT_ROUTE18GATE1F_GUARD,
    SCRIPT_ROUTE18GATE1F_PLAYER_MOVING_RIGHT, SCRIPT_ROUTE18GATE1F_PLAYER_MOVING_UP, TEXT_ROUTE18GATE1F_GUARD,
    TEXT_ROUTE18GATE1F_GUARD_EXCUSE_ME};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Flow, Script};

const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// `.StopsPlayerCoords`, as (x, y). The row the player is on says how many steps up to the counter.
const STOPS_PLAYER: [(u8, u8); 4] = [(4, 3), (4, 4), (4, 5), (4, 6)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute18Gate1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// The guard's shout has closed, with `wCoordIndex` as it was.
    WaitUpTextDone(u8),
    /// The guard has had his say about the road.
    GuardTextDone,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.set_always_on_bike(false);
    rt.enable_auto_text_box_drawing();
    match rt.maps().route18_gate_1f.cur_script {
        SCRIPT_ROUTE18GATE1F_DEFAULT => default_script(rt),
        SCRIPT_ROUTE18GATE1F_PLAYER_MOVING_UP => player_moving_up(rt),
        SCRIPT_ROUTE18GATE1F_GUARD => guard_script(rt),
        _ => player_moving_right(rt),
    }
}

/// `Route18Gate1FDefaultScript`: the guard lets a player with a bicycle by and stops one on foot.
fn default_script(rt: &mut Script) -> Flow {
    if rt.is_item_in_bag(ItemId::Bicycle) {
        return Flow::Return;
    }
    let Some(index) = rt.are_player_coords_in_array(&STOPS_PLAYER) else {
        return Flow::Return;
    };
    rt.display_text_id(TEXT_ROUTE18GATE1F_GUARD_EXCUSE_ME).then(Label::WaitUpTextDone(index))
}

/// `Route18Gate1FPlayerMovingUpScript`, which falls through into the guard's.
fn player_moving_up(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    guard_script(rt)
}

fn guard_script(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_ROUTE18GATE1F_GUARD).then(Label::GuardTextDone)
}

/// `Route18Gate1FPlayerMovingRightScript`: the step onto the road is the last the gate simulates.
fn player_moving_right(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.stop_simulating_joypad_states();
    rt.maps().route18_gate_1f.cur_script = SCRIPT_ROUTE18GATE1F_DEFAULT;
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_ROUTE18GATE1F_GUARD {
        return None;
    }
    let words = if rt.is_item_in_bag(ItemId::Bicycle) {
        Route18Gate1FGuardText::CyclingRoadUphillText
    } else {
        Route18Gate1FGuardText::YouNeedABicycleText
    };
    Some(rt.print_text(text_at(words)).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::WaitUpTextDone(index) => {
            rt.clear_joy_held();
            let next = if index == 1 {
                SCRIPT_ROUTE18GATE1F_GUARD
            } else {
                rt.simulate_joypad_presses(vec![Joypad::UP; index as usize - 1]);
                SCRIPT_ROUTE18GATE1F_PLAYER_MOVING_UP
            };
            rt.maps().route18_gate_1f.cur_script = next;
            Flow::Return
        }
        Label::GuardTextDone => {
            rt.simulate_joypad_presses(vec![Joypad::RIGHT]);
            rt.maps().route18_gate_1f.cur_script = SCRIPT_ROUTE18GATE1F_PLAYER_MOVING_RIGHT;
            Flow::Return
        }
    }
}
