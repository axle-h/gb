//! `Route22Gate_Script`: the guard who turns back anyone without the Boulder Badge, and `wLastMap`
//! kept to whichever route the half of the gate the player stands in opens onto.

use poke_core::map::Map;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE22GATE_DEFAULT, SCRIPT_ROUTE22GATE_NOOP,
    SCRIPT_ROUTE22GATE_PLAYER_MOVING, TEXT_ROUTE22GATE_GUARD};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::input::Joypad;
use super::{text_at, Flow, Script};

/// `BIT_BOULDERBADGE`.
const BIT_BOULDERBADGE: u8 = 0;
/// `Route22GateScriptCoords`: the two squares in front of the guard's counter.
const GATE_COORDS: [(u8, u8); 2] = [(4, 2), (5, 2)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute22GateCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route22GatePlayerMovingScript` after its `Delay3`.
    MovedBack,
    /// `Route22GateGuardNoBoulderbadgeText`'s `text_asm`: `PlaySoundWaitForCurrent`'s wait, the
    /// sound's own, and the rest of the text.
    DeniedSound,
    DeniedSoundPlayed,
    CantLetYouPass,
    /// `Route22GateGuardText` after its `PrintText`, either way.
    GuardTextEnd,
    GoRightAhead,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    // Written before the table rather than after it: nothing the table runs moves the player
    // before this pass is over.
    rt.set_last_map(if rt.y() < 4 { Map::Route23 } else { Map::Route22 });
    match rt.maps().route22_gate.cur_script {
        SCRIPT_ROUTE22GATE_DEFAULT => default_script(rt),
        SCRIPT_ROUTE22GATE_PLAYER_MOVING => player_moving(rt),
        _ => Flow::Return,
    }
}

/// `Route22GateDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.are_player_coords_in_array(&GATE_COORDS).is_none() {
        return Flow::Return;
    }
    rt.clear_joy_held();
    rt.display_text_id(TEXT_ROUTE22GATE_GUARD).ret()
}

/// `Route22GateMovePlayerDownScript`: `PAD_DOWN` goes into the facing byte and `wJoyIgnore` as well
/// as the simulated press.
fn move_player_down(rt: &mut Script) {
    rt.simulate_joypad_presses(vec![Joypad::DOWN]);
    rt.set_player_facing(Joypad::DOWN.bits());
    rt.joy_ignore(Joypad::DOWN);
}

/// `Route22GatePlayerMovingScript`.
fn player_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.delay3().then(Label::MovedBack)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_ROUTE22GATE_GUARD {
        return None;
    }
    if rt.badges() & (1 << BIT_BOULDERBADGE) != 0 {
        return Some(rt.print_text(text_at(sym::Route22GateGuardGoRightAheadText)).then(Label::GoRightAhead));
    }
    Some(rt.print_text(text_at(sym::Route22GateGuardNoBoulderbadgeText)).then(Label::DeniedSound))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::MovedBack => {
            rt.maps().route22_gate.cur_script = SCRIPT_ROUTE22GATE_DEFAULT;
            Flow::Return
        }
        Label::DeniedSound => rt.wait_for_sound_to_finish().then(Label::DeniedSoundPlayed),
        Label::DeniedSoundPlayed => {
            rt.play_sound(sounds::SFX_DENIED);
            rt.wait_for_sound_to_finish().then(Label::CantLetYouPass)
        }
        Label::CantLetYouPass => {
            rt.print_text_from_asm(text_at(sym::Route22GateGuardICantLetYouPassText)).then(Label::GuardTextEnd)
        }
        Label::GoRightAhead => {
            rt.maps().route22_gate.cur_script = SCRIPT_ROUTE22GATE_NOOP;
            Flow::Return
        }
        Label::GuardTextEnd => {
            move_player_down(rt);
            rt.maps().route22_gate.cur_script = SCRIPT_ROUTE22GATE_PLAYER_MOVING;
            Flow::Return
        }
    }
}
