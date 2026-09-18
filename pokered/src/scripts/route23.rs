//! `Route23_Script`: the seven gates up to the League, each asking for one badge, and the boulder
//! Victory Road's top two floors share.

use poke_core::symbols::pokered_events::{EVENT_PASSED_CASCADEBADGE_CHECK, EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH1,
    EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH2, EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH1,
    EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH2};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROUTE23_DEFAULT, SCRIPT_ROUTE23_PLAYER_MOVING,
    SCRIPT_ROUTE23_RESET_TO_DEFAULT, TEXT_ROUTE23_GUARD1, TEXT_ROUTE23_GUARD2, TEXT_ROUTE23_GUARD3,
    TEXT_ROUTE23_GUARD4, TEXT_ROUTE23_GUARD5, TEXT_ROUTE23_SWIMMER1, TEXT_ROUTE23_SWIMMER2};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::{TOGGLE_VICTORY_ROAD_2F_BOULDER, TOGGLE_VICTORY_ROAD_3F_BOULDER};
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::input::Joypad;
use crate::systems::overworld::sprites::SPRITE_FACING_DOWN;
use super::{text_at, Flow, Script};

/// `Route23GuardsYCoords`: the row each gate's guard stands on, the League end first.
const GUARDS_Y: [u8; 7] = [35, 56, 85, 96, 105, 119, 136];
/// `BadgeTextPointers`, by `wWhichBadge`: the guard on the southernmost row wants the first badge.
const BADGES: [&str; 7] = ["CASCADEBADGE", "THUNDERBADGE", "RAINBOWBADGE", "SOULBADGE", "MARSHBADGE",
    "VOLCANOBADGE", "EARTHBADGE"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute23CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route23DefaultScript` after the guard's own text has closed.
    GuardAsked,
    /// `Route23CheckForBadgeScript`'s `.have_badge` after its `PrintText`, carrying `wWhichBadge`.
    HaveBadge(u8),
    /// `Route23YouDontHaveTheBadgeYetText`'s `text_asm`: `PlaySoundWaitForCurrent`'s wait, the
    /// sound's own, and then the walk back down.
    DeniedSound,
    DeniedSoundPlayed,
    DeniedDone,
}

pub fn script(rt: &mut Script) -> Flow {
    set_victory_road_boulders(rt);
    rt.enable_auto_text_box_drawing();
    match rt.maps().route23.cur_script {
        SCRIPT_ROUTE23_DEFAULT => default_script(rt),
        SCRIPT_ROUTE23_PLAYER_MOVING => player_moving(rt),
        _ => reset_to_default(rt),
    }
}

/// `Route23SetVictoryRoadBoulders`: coming out onto the road puts Victory Road's boulder back on 3F,
/// so the one boulder the top two floors share is always on the floor the player has not solved.
fn set_victory_road_boulders(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(2) {
        return;
    }
    for event in [EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH1, EVENT_VICTORY_ROAD_2_BOULDER_ON_SWITCH2,
        EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH1, EVENT_VICTORY_ROAD_3_BOULDER_ON_SWITCH2]
    {
        rt.reset_event(event);
    }
    rt.show_object(TOGGLE_VICTORY_ROAD_3F_BOULDER);
    rt.hide_object(TOGGLE_VICTORY_ROAD_2F_BOULDER);
}

/// `Route23DefaultScript`: standing on a guard's row is what asks for the badge, and the guard's own
/// text answers because `hSpriteIndex` and `hTextID` are the same byte.
fn default_script(rt: &mut Script) -> Flow {
    let Some(index) = GUARDS_Y.iter().position(|&row| row == rt.y()) else {
        return Flow::Return;
    };
    // The top row carries on east past Victory Road's 2F door, and the guard there has no say over
    // anyone already on that side of it.
    if rt.y() == GUARDS_Y[0] && rt.x() >= 14 {
        return Flow::Return;
    }
    let which = (GUARDS_Y.len() - 1 - index) as u8;
    if rt.check_event(EVENT_PASSED_CASCADEBADGE_CHECK + which as u16) {
        return Flow::Return;
    }
    rt.set_name_buffer(BADGES[which as usize]);
    rt.display_text_id(index as u8 + 1).then(Label::GuardAsked)
}

/// `Route23MovePlayerDownScript`: one simulated press south, with the player turned that way first
/// so the press is a step rather than a turn.
fn move_player_down(rt: &mut Script) {
    rt.simulate_joypad_presses(vec![Joypad::DOWN]);
    rt.set_player_facing(SPRITE_FACING_DOWN);
    rt.joy_ignore(Joypad::empty());
}

/// `Route23PlayerMovingScript`, which falls through into the reset below.
fn player_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    reset_to_default(rt)
}

/// `Route23ResetToDefaultScript`.
fn reset_to_default(rt: &mut Script) -> Flow {
    rt.maps().route23.cur_script = SCRIPT_ROUTE23_DEFAULT;
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let which = match text_id {
        TEXT_ROUTE23_GUARD1 => 6,
        TEXT_ROUTE23_GUARD2 => 5,
        TEXT_ROUTE23_SWIMMER1 => 4,
        TEXT_ROUTE23_SWIMMER2 => 3,
        TEXT_ROUTE23_GUARD3 => 2,
        TEXT_ROUTE23_GUARD4 => 1,
        TEXT_ROUTE23_GUARD5 => 0,
        _ => return None,
    };
    Some(check_for_badge(rt, which))
}

/// `Route23CheckForBadgeScript`: the badge for `which` gate, one bit above it in `wObtainedBadges`.
fn check_for_badge(rt: &mut Script, which: u8) -> Flow {
    rt.set_name_buffer(BADGES[which as usize]);
    if rt.badges() & (1 << (which + 1)) == 0 {
        return rt.print_text(text_at(sym::Route23YouDontHaveTheBadgeYetText)).then(Label::DeniedSound);
    }
    rt.print_text(text_at(sym::Route23OhThatIsTheBadgeText)).then(Label::HaveBadge(which))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::GuardAsked => {
            rt.clear_joy_held();
            Flow::Return
        }
        Label::HaveBadge(which) => {
            rt.set_event(EVENT_PASSED_CASCADEBADGE_CHECK + which as u16);
            rt.maps().route23.cur_script = SCRIPT_ROUTE23_RESET_TO_DEFAULT;
            Flow::Return
        }
        Label::DeniedSound => rt.wait_for_sound_to_finish().then(Label::DeniedSoundPlayed),
        Label::DeniedSoundPlayed => {
            rt.play_sound(sounds::SFX_DENIED);
            rt.wait_for_sound_to_finish().then(Label::DeniedDone)
        }
        Label::DeniedDone => {
            move_player_down(rt);
            rt.maps().route23.cur_script = SCRIPT_ROUTE23_PLAYER_MOVING;
            Flow::Return
        }
    }
}
