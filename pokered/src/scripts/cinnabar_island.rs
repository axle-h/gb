//! `CinnabarIsland_Script`: the gym's locked door, which pushes the player back down until the
//! Secret Key is in the bag.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_LAB_STILL_REVIVING_FOSSIL, EVENT_MANSION_SWITCH_ON};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_CINNABARISLAND_DEFAULT, SCRIPT_CINNABARISLAND_PLAYER_MOVING,
    TEXT_CINNABARISLAND_DOOR_IS_LOCKED};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::systems::overworld::sprites::SPRITE_FACING_DOWN;
use super::{Flow, Script};

const PLAYER_DIR_UP: u8 = 8;
/// The square in front of the gym door.
const GYM_DOOR: (u8, u8) = (18, 4);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wCinnabarIslandCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DoorIsLocked,
    PlayerMovedDelay,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    // Every pass of the island redraws the mansion's walls shut and finishes whatever the lab is
    // reviving: both are the map the player left, not this one.
    rt.set_cur_map_loaded(1);
    rt.reset_event(EVENT_MANSION_SWITCH_ON);
    rt.reset_event(EVENT_LAB_STILL_REVIVING_FOSSIL);
    match rt.maps().cinnabar_island.cur_script {
        SCRIPT_CINNABARISLAND_PLAYER_MOVING => player_moving(rt),
        _ => default_script(rt),
    }
}

/// `CinnabarIslandDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.is_item_in_bag(ItemId::SecretKey) || (rt.x(), rt.y()) != GYM_DOOR {
        return Flow::Return;
    }
    rt.set_player_moving_direction(PLAYER_DIR_UP);
    rt.display_text_id(TEXT_CINNABARISLAND_DOOR_IS_LOCKED).then(Label::DoorIsLocked)
}

/// `CinnabarIslandPlayerMovingScript`.
fn player_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::PlayerMovedDelay)
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DoorIsLocked => {
            rt.clear_joy_held();
            rt.simulate_joypad_presses(vec![Joypad::DOWN]);
            rt.set_player_facing(SPRITE_FACING_DOWN);
            rt.joy_ignore(Joypad::empty());
            rt.maps().cinnabar_island.cur_script = SCRIPT_CINNABARISLAND_PLAYER_MOVING;
            Flow::Return
        }
        Label::PlayerMovedDelay => {
            rt.maps().cinnabar_island.cur_script = SCRIPT_CINNABARISLAND_DEFAULT;
            Flow::Return
        }
    }
}
