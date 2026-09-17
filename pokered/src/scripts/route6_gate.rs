//! `Route6Gate_Script`: the same drink the other three Saffron gates want, asked for down of the counter.

use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE6GATE_DEFAULT, SCRIPT_ROUTE6GATE_PLAYER_MOVING,
    TEXT_ROUTE6GATE_GUARD, TEXT_ROUTE6GATE_GUARD_GEE_IM_THIRSTY, TEXT_ROUTE6GATE_GUARD_GIVE_DRINK};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{Flow, Script};

/// `PLAYER_DIR_RIGHT`.
const PLAYER_DIR_RIGHT: u8 = 1;
/// `.PlayerInCoordsArray`: the two squares above the counter.
const GATE_COORDS: [(u8, u8); 2] = [(3, 2), (4, 2)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute6GateCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Thirsty,
    MovedBack,
    GaveDrink,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().route6_gate.cur_script {
        SCRIPT_ROUTE6GATE_PLAYER_MOVING => player_moving(rt),
        _ => default_script(rt),
    }
}

/// `Route6GateDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.gave_saffron_guards_drink() || rt.are_player_coords_in_array(&GATE_COORDS).is_none() {
        return Flow::Return;
    }
    rt.set_player_moving_direction(PLAYER_DIR_RIGHT);
    rt.clear_joy_held();
    match rt.remove_guard_drink() {
        Some(_) => {
            rt.set_gave_saffron_guards_drink();
            rt.display_text_id(TEXT_ROUTE6GATE_GUARD_GIVE_DRINK).then(Label::GaveDrink)
        }
        None => rt.display_text_id(TEXT_ROUTE6GATE_GUARD_GEE_IM_THIRSTY).then(Label::Thirsty),
    }
}

/// `Route6GatePlayerMovingScript`.
fn player_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::MovedBack)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        // The guard's own words are `SaffronGateGuardText`, which every gate shares: it walks the
        // player up and arms Route 5's gate script wherever it is read from.
        TEXT_ROUTE6GATE_GUARD => Some(super::route5_gate::guard_text(rt)),
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        // `Route6GateMovePlayerDownScript`.
        Label::Thirsty => {
            rt.simulate_joypad_presses(vec![Joypad::DOWN]);
            rt.maps().route6_gate.cur_script = SCRIPT_ROUTE6GATE_PLAYER_MOVING;
            Flow::Return
        }
        Label::MovedBack => {
            rt.joy_ignore(Joypad::empty());
            rt.maps().route6_gate.cur_script = SCRIPT_ROUTE6GATE_DEFAULT;
            Flow::Return
        }
        Label::GaveDrink => Flow::Return,
    }
}
