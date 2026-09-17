//! `Route5Gate_Script`: the guard who wants a drink before anyone walks south into Saffron, and the
//! `SaffronGateGuardText` all four gates share.

use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE5GATE_DEFAULT, SCRIPT_ROUTE5GATE_PLAYER_MOVING,
    TEXT_ROUTE5GATE_GUARD, TEXT_ROUTE5GATE_GUARD_GEE_IM_THIRSTY, TEXT_ROUTE5GATE_GUARD_GIVE_DRINK};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `PLAYER_DIR_LEFT`.
const PLAYER_DIR_LEFT: u8 = 2;
/// `.PlayerInCoordsArray`: the two squares below the counter.
const GATE_COORDS: [(u8, u8); 2] = [(3, 3), (4, 3)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute5GateCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Thirsty,
    MovedBack,
    /// The shared text's own branches, whichever gate it was reached from.
    SharedThirsty,
    GaveDrink,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().route5_gate.cur_script {
        SCRIPT_ROUTE5GATE_PLAYER_MOVING => player_moving(rt),
        _ => default_script(rt),
    }
}

/// `Route5GateMovePlayerUpScript`: whoever has no drink is walked back the square they came from.
pub fn move_player_up(rt: &mut Script) {
    rt.simulate_joypad_presses(vec![Joypad::UP]);
}

/// `Route5GateDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.gave_saffron_guards_drink() || rt.are_player_coords_in_array(&GATE_COORDS).is_none() {
        return Flow::Return;
    }
    rt.set_player_moving_direction(PLAYER_DIR_LEFT);
    rt.clear_joy_held();
    match rt.remove_guard_drink() {
        Some(_) => rt.display_text_id(TEXT_ROUTE5GATE_GUARD_GIVE_DRINK).then(Label::GaveDrink),
        None => rt.display_text_id(TEXT_ROUTE5GATE_GUARD_GEE_IM_THIRSTY).then(Label::Thirsty),
    }
}

/// `Route5GatePlayerMovingScript`.
fn player_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::MovedBack)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_ROUTE5GATE_GUARD => Some(guard_text(rt)),
        _ => None,
    }
}

/// `SaffronGateGuardText`, which every Saffron gate's text table points at: it walks the player up
/// and arms Route 5's gate script whichever of the four gates they are standing in.
pub fn guard_text(rt: &mut Script) -> Flow {
    if rt.gave_saffron_guards_drink() {
        return rt.print_text(text_at(sym::SaffronGateGuardThanksForTheDrinkText)).ret();
    }
    match rt.remove_guard_drink() {
        Some(_) => rt.print_text(text_at(sym::SaffronGateGuardGiveDrinkText)).then(Label::GaveDrink),
        None => rt.print_text(text_at(sym::SaffronGateGuardGeeImThirstyText)).then(Label::SharedThirsty),
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Thirsty | Label::SharedThirsty => {
            move_player_up(rt);
            rt.maps().route5_gate.cur_script = SCRIPT_ROUTE5GATE_PLAYER_MOVING;
            Flow::Return
        }
        Label::MovedBack => {
            rt.joy_ignore(Joypad::empty());
            rt.maps().route5_gate.cur_script = SCRIPT_ROUTE5GATE_DEFAULT;
            Flow::Return
        }
        Label::GaveDrink => {
            rt.set_gave_saffron_guards_drink();
            Flow::Return
        }
    }
}
