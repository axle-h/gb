//! `PewterCity_Script`: the two guides, who walk the player to the museum and to the gym and are
//! put back where they stood once they are out of sight.

use poke_core::symbols::pokered_events::{EVENT_BEAT_BROCK, EVENT_BOUGHT_MUSEUM_TICKET};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{PEWTERCITY_SUPER_NERD1, PEWTERCITY_YOUNGSTER};
use poke_core::symbols::pokered_toggles::{TOGGLE_GYM_GUY, TOGGLE_MUSEUM_GUY};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, PEWTER_GYM_GUY_MOVEMENT_SCRIPT,
    PEWTER_MUSEUM_GUY_MOVEMENT_SCRIPT};
use crate::modes::overworld::script::SpritePosition;
use crate::systems::overworld::sprites::{SPRITE_FACING_LEFT, SPRITE_FACING_UP};
use super::{text_at, Flow, Script};

const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `PewterCityPlayerLeavingEastCoords`, as (x, y): the way out east, which the gym guide blocks
/// until Brock is beaten.
const LEAVING_EAST: [(u8, u8); 4] = [(35, 17), (36, 17), (37, 18), (37, 19)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPewterCityCurScript`.
    pub cur_script: u8,
    /// `GetSpritePosition2`: where a guide stood before he led the player off, to put him back.
    pub guide_at: Option<SpritePosition>,
}

/// Which of the two guides has walked the player somewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Guide {
    Museum,
    Gym,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `SetSpriteFacingDirectionAndDelay`'s frames, then `PlayDefaultMusic` once the guide's own
    /// tune is over.
    GuideFaced(Guide),
    GuideMusicPlayed(Guide),
    MuseumGuideArrived,
    GymGuideArrived,
    /// `PewterCitySuperNerd1Text`'s and `PewterCitySuperNerd2Text`'s yes/no.
    SuperNerd1YesNo,
    SuperNerd1Answered,
    SuperNerd2YesNo,
    SuperNerd2Answered,
    YoungsterFollowMe,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().pewter_city.cur_script {
        SCRIPT_PEWTERCITY_DEFAULT => default_script(rt),
        SCRIPT_PEWTERCITY_SUPER_NERD1_SHOWS_PLAYER_MUSEUM => guide_arrives(rt, Guide::Museum),
        SCRIPT_PEWTERCITY_HIDE_SUPER_NERD1 => {
            hide_guide(rt, TOGGLE_MUSEUM_GUY, SCRIPT_PEWTERCITY_RESET_SUPER_NERD1)
        }
        SCRIPT_PEWTERCITY_RESET_SUPER_NERD1 => {
            reset_guide(rt, PEWTERCITY_SUPER_NERD1, TOGGLE_MUSEUM_GUY)
        }
        SCRIPT_PEWTERCITY_YOUNGSTER_SHOWS_PLAYER_GYM => guide_arrives(rt, Guide::Gym),
        SCRIPT_PEWTERCITY_HIDE_YOUNGSTER => hide_guide(rt, TOGGLE_GYM_GUY, SCRIPT_PEWTERCITY_RESET_YOUNGSTER),
        SCRIPT_PEWTERCITY_RESET_YOUNGSTER => reset_guide(rt, PEWTERCITY_YOUNGSTER, TOGGLE_GYM_GUY),
        _ => Flow::Return,
    }
}

/// `PewterCityDefaultScript` and `PewterCityCheckPlayerLeavingEastScript`. The museum's script is
/// put back to its start on every pass, which is what has its clerk charge again on a second visit.
fn default_script(rt: &mut Script) -> Flow {
    rt.maps().museum_1f.cur_script = 0;
    rt.reset_event(EVENT_BOUGHT_MUSEUM_TICKET);
    if rt.check_event(EVENT_BEAT_BROCK) || rt.are_player_coords_in_array(&LEAVING_EAST).is_none() {
        return Flow::Return;
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.display_text_id(TEXT_PEWTERCITY_YOUNGSTER).ret()
}

impl Guide {
    /// The guide's sprite, the way he faces the door, the frame he points in, his words there, and
    /// where his script carries on.
    fn arrival(self) -> (u8, u8, u8, u8, Label) {
        match self {
            Guide::Museum => (PEWTERCITY_SUPER_NERD1, SPRITE_FACING_UP, 0x30, TEXT_PEWTERCITY_SUPER_NERD1_ITS_RIGHT_HERE,
                Label::MuseumGuideArrived),
            Guide::Gym => (PEWTERCITY_YOUNGSTER, SPRITE_FACING_LEFT, 0x10, TEXT_PEWTERCITY_YOUNGSTER_GO_TAKE_ON_BROCK,
                Label::GymGuideArrived),
        }
    }
}

/// `PewterCitySuperNerd1ShowsPlayerMuseumScript` and `PewterCityYoungsterShowsPlayerGymScript`: the
/// guide turns to the door and points at it, and the town's own music comes back.
fn guide_arrives(rt: &mut Script, guide: Guide) -> Flow {
    if rt.npc_movement_script_running() {
        return Flow::Return;
    }
    let (slot, facing, ..) = guide.arrival();
    rt.set_sprite_facing_direction_and_delay(slot, facing).then(Label::GuideFaced(guide))
}

/// `PewterCityHideSuperNerd1Script` and `PewterCityHideYoungsterScript`.
fn hide_guide(rt: &mut Script, toggle: u16, next: u8) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.hide_object(toggle);
    rt.maps().pewter_city.cur_script = next;
    Flow::Return
}

/// `PewterCityResetSuperNerd1Script` and `PewterCityResetYoungsterScript`.
fn reset_guide(rt: &mut Script, slot: u8, toggle: u16) -> Flow {
    if let Some(at) = rt.maps().pewter_city.guide_at.take() {
        rt.set_sprite_position(slot, at);
    }
    rt.show_object(toggle);
    rt.joy_ignore(Joypad::empty());
    rt.maps().pewter_city.cur_script = SCRIPT_PEWTERCITY_DEFAULT;
    Flow::Return
}

/// The guide is put in front of the door he has walked the player to and then walked off screen,
/// which is what takes him out of the map rather than a warp.
fn walk_guide_away(rt: &mut Script, slot: u8, at: SpritePosition, path: &[u8], next: u8) -> Flow {
    rt.set_sprite_position(slot, at);
    rt.move_sprite(slot, path);
    rt.maps().pewter_city.cur_script = next;
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_PEWTERCITY_SUPER_NERD1 => {
            rt.print_text(text_at(local::PewterCitySuperNerd1Text::DidYouCheckOutMuseumText))
                .then(Label::SuperNerd1YesNo)
        }
        TEXT_PEWTERCITY_SUPER_NERD2 => {
            rt.print_text(text_at(local::PewterCitySuperNerd2Text::DoYouKnowWhatImDoingText))
                .then(Label::SuperNerd2YesNo)
        }
        TEXT_PEWTERCITY_YOUNGSTER => {
            rt.print_text(text_at(local::PewterCityYoungsterText::YoureATrainerFollowMeText))
                .then(Label::YoungsterFollowMe)
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::GuideFaced(guide) => {
            let (slot, facing, image_index, ..) = guide.arrival();
            rt.set_sprite_image_index(slot, image_index | facing);
            rt.play_default_music().then(Label::GuideMusicPlayed(guide))
        }
        Label::GuideMusicPlayed(guide) => {
            let (.., text_id, then) = guide.arrival();
            rt.set_no_sprite_updates();
            rt.display_text_id(text_id).then(then)
        }
        // `$3c`, `$30`, 12, 17: the square below the museum's door.
        Label::MuseumGuideArrived => {
            let at = SpritePosition { screen_y: 0x3C, screen_x: 0x30, map_y: 12, map_x: 17 };
            let path = [NPC_MOVEMENT_DOWN; 4];
            let path = [path.as_slice(), &[END]].concat();
            walk_guide_away(rt, PEWTERCITY_SUPER_NERD1, at, &path, SCRIPT_PEWTERCITY_HIDE_SUPER_NERD1)
        }
        // `$40` is the cartridge's own mistake: the gym guide is drawn half a square left of where
        // he stands, and `$50` is where the square is.
        Label::GymGuideArrived => {
            let at = SpritePosition { screen_y: 0x3C, screen_x: 0x40, map_y: 22, map_x: 16 };
            let path = [NPC_MOVEMENT_RIGHT; 5];
            let path = [path.as_slice(), &[END]].concat();
            walk_guide_away(rt, PEWTERCITY_YOUNGSTER, at, &path, SCRIPT_PEWTERCITY_HIDE_YOUNGSTER)
        }

        Label::SuperNerd1YesNo => rt.yes_no_choice().then(Label::SuperNerd1Answered),
        Label::SuperNerd1Answered => {
            if rt.chose_yes() {
                return rt.print_text(text_at(local::PewterCitySuperNerd1Text::WerentThoseFossilsAmazingText)).ret();
            }
            let at = rt.sprite_position(PEWTERCITY_SUPER_NERD1);
            rt.maps().pewter_city.guide_at = Some(at);
            rt.clear_joy_held();
            rt.start_npc_movement_script(PEWTER_MUSEUM_GUY_MOVEMENT_SCRIPT, PEWTERCITY_SUPER_NERD1);
            rt.maps().pewter_city.cur_script = SCRIPT_PEWTERCITY_SUPER_NERD1_SHOWS_PLAYER_MUSEUM;
            rt.print_text(text_at(local::PewterCitySuperNerd1Text::YouHaveToGoText)).ret()
        }

        Label::SuperNerd2YesNo => rt.yes_no_choice().then(Label::SuperNerd2Answered),
        Label::SuperNerd2Answered => {
            let words = match rt.chose_yes() {
                true => local::PewterCitySuperNerd2Text::ThatsRightText,
                false => local::PewterCitySuperNerd2Text::ImSprayingRepelText,
            };
            rt.print_text(text_at(words)).ret()
        }

        Label::YoungsterFollowMe => {
            let at = rt.sprite_position(PEWTERCITY_YOUNGSTER);
            rt.maps().pewter_city.guide_at = Some(at);
            rt.clear_joy_held();
            rt.start_npc_movement_script(PEWTER_GYM_GUY_MOVEMENT_SCRIPT, PEWTERCITY_YOUNGSTER);
            rt.maps().pewter_city.cur_script = SCRIPT_PEWTERCITY_YOUNGSTER_SHOWS_PLAYER_GYM;
            Flow::Return
        }
    }
}
