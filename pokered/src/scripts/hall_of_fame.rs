//! `HallOfFame_Script`: the walk in, Oak's last words, and the ceremony that ends the game.

use poke_core::symbols::pokered_map_scripts::{SCRIPT_HALLOFFAME_DEFAULT,
    SCRIPT_HALLOFFAME_OAK_CONGRATULATIONS, SCRIPT_HALLOFFAME_RESET_EVENTS_AND_SAVE, TEXT_HALLOFFAME_OAK};
use poke_core::symbols::pokered_symbols::HALLOFFAME_OAK;
use poke_core::symbols::pokered_toggles::TOGGLE_CERULEAN_CAVE_GUY;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::systems::overworld::sprites::SPRITE_FACING_LEFT;
use super::{Code, Flow, Script};

/// `PLAYER_DIR_RIGHT`.
const PLAYER_DIR_RIGHT: u8 = 1;
const PAD_BUTTONS: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START);
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// `HallOfFameEntryMovement`.
const WALK_IN: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wHallOfFameCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    OakCongratulations,
    ResetEventsAndSave,
    OakTurned,
    OakHeld,
    OakSpoke,
    Saving,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let entry: Code = match rt.maps().hall_of_fame.cur_script {
        SCRIPT_HALLOFFAME_DEFAULT => Label::DefaultScript.into(),
        SCRIPT_HALLOFFAME_OAK_CONGRATULATIONS => Label::OakCongratulations.into(),
        SCRIPT_HALLOFFAME_RESET_EVENTS_AND_SAVE => Label::ResetEventsAndSave.into(),
        // `HallOfFameNoopScript`.
        _ => return Flow::Return,
    };
    Flow::Jump(entry)
}

fn set_script(rt: &mut Script, index: u8) {
    rt.maps().hall_of_fame.cur_script = index;
}

/// `HallOfFameDefaultScript`: five squares up to the machine, walked for the player.
fn default_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
    rt.simulate_joypad_presses(vec![Joypad::UP; WALK_IN]);
    set_script(rt, SCRIPT_HALLOFFAME_OAK_CONGRATULATIONS);
    Flow::Return
}

/// `HallOfFameOakCongratulationsScript`.
fn oak_congratulations(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.set_player_moving_direction(PLAYER_DIR_RIGHT);
    rt.set_sprite_movement_bytes_to_ff(HALLOFFAME_OAK);
    rt.set_sprite_facing_direction_and_delay(HALLOFFAME_OAK, SPRITE_FACING_LEFT).then(Label::OakTurned)
}

/// `HallOfFameResetEventsAndSaveScript`. The League's rooms are put back to their first script and
/// the map music is let play again before the ceremony, because the ceremony writes the save the
/// game restarts on and never comes back.
fn reset_events_and_save(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.set_no_map_music(false);
    rt.maps().loreleis_room.cur_script = 0;
    rt.maps().brunos_room.cur_script = 0;
    rt.maps().agathas_room.cur_script = 0;
    rt.maps().lances_room.cur_script = 0;
    set_script(rt, SCRIPT_HALLOFFAME_DEFAULT);
    rt.hall_of_fame_pc().ret()
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::OakCongratulations => oak_congratulations(rt),
        Label::ResetEventsAndSave => rt.delay3().then(Label::Saving),
        // Oak is held turned to the player for the `Delay3` before he speaks.
        Label::OakTurned => rt.delay3().then(Label::OakHeld),
        Label::OakHeld => {
            rt.joy_ignore(Joypad::empty());
            rt.set_player_moving_direction(PLAYER_DIR_RIGHT);
            rt.display_text_id(TEXT_HALLOFFAME_OAK).then(Label::OakSpoke)
        }
        Label::OakSpoke => {
            rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
            rt.hide_object(TOGGLE_CERULEAN_CAVE_GUY);
            set_script(rt, SCRIPT_HALLOFFAME_RESET_EVENTS_AND_SAVE);
            Flow::Return
        }
        Label::Saving => reset_events_and_save(rt),
    }
}
