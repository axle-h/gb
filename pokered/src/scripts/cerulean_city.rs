//! `CeruleanCity_Script`: the rival waiting on the bridge, the Rocket who breaks out of the house
//! behind him, and the Slowbro two people take turns failing to give orders to.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_BEAT_CERULEAN_RIVAL, EVENT_BEAT_CERULEAN_ROCKET_THIEF};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::{CERULEANCITY_RIVAL, CERULEANCITY_ROCKET};
use poke_core::symbols::pokered_toggles::{TOGGLE_CERULEAN_GUARD_1, TOGGLE_CERULEAN_GUARD_2,
    TOGGLE_CERULEAN_RIVAL, TOGGLE_CERULEAN_ROCKET};
use poke_core::trainer_headers::OPP_ID_OFFSET;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT, NPC_MOVEMENT_RIGHT};
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_UP};
use super::{text_at, Flow, Script};

/// `PLAYER_DIR_DOWN` and `PLAYER_DIR_UP`.
const PLAYER_DIR_UP: u8 = 8;
const PLAYER_DIR_DOWN: u8 = 4;
const END: u8 = 0xFF;
/// `OPP_RIVAL1`.
const OPP_RIVAL1: u8 = OPP_ID_OFFSET + 25;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `CeruleanCityCoords1`: the two squares either side of the Rocket's hole in the wall.
const ROCKET_COORDS: [(u8, u8); 2] = [(30, 7), (30, 9)];
/// `CeruleanCityCoords2`: the bridge's two middle squares, where the rival drops in from above.
const RIVAL_COORDS: [(u8, u8); 2] = [(20, 6), (21, 6)];
/// The right-hand of the two: the rival stands where he already is rather than being moved across.
const RIGHT_OF_BRIDGE: u8 = 20;
/// `CeruleanCityMovement1`, and the two he leaves by: from the left square he goes right first and
/// from the right one left, so either way he walks round the player and off south.
const RIVAL_ARRIVES: [u8; 4] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];
const RIVAL_LEAVES_LEFT: [u8; 8] = [NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];
const RIVAL_LEAVES_RIGHT: [u8; 8] = [NPC_MOVEMENT_LEFT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wCeruleanCityCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    RocketGreeting,
    RivalBattleText,
    RivalDefeatedFaces,
    RivalDefeatedText,
    RocketDefeatedText,
    RocketBattleText,
    RocketTmText,
    RocketTmReceived,
    RocketHidden,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().cerulean_city.cur_script {
        SCRIPT_CERULEANCITY_RIVAL_BATTLE => rival_battle(rt),
        SCRIPT_CERULEANCITY_RIVAL_DEFEATED => rival_defeated(rt),
        SCRIPT_CERULEANCITY_RIVAL_CLEANUP => rival_cleanup(rt),
        SCRIPT_CERULEANCITY_ROCKET_DEFEATED => rocket_defeated(rt),
        _ => default_script(rt),
    }
}

/// `CeruleanCityClearScripts`.
fn clear_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().cerulean_city.cur_script = SCRIPT_CERULEANCITY_DEFAULT;
    rt.hide_object(TOGGLE_CERULEAN_RIVAL);
    Flow::Return
}

/// `CeruleanCityDefaultScript`: the thief speaks first if he is still in the wall, and the rival
/// drops onto the bridge north of town on the way past.
fn default_script(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_CERULEAN_ROCKET_THIEF)
        && let Some(index) = rt.are_player_coords_in_array(&ROCKET_COORDS)
    {
        let (direction, facing) = match index {
            1 => (PLAYER_DIR_DOWN, SPRITE_FACING_UP),
            _ => (PLAYER_DIR_UP, SPRITE_FACING_DOWN),
        };
        rt.set_player_moving_direction(direction);
        rt.set_sprite_facing(CERULEANCITY_ROCKET, facing);
        return rt.delay3().then(Label::RocketGreeting);
    }
    if rt.check_event(EVENT_BEAT_CERULEAN_RIVAL) || rt.are_player_coords_in_array(&RIVAL_COORDS).is_none() {
        return Flow::Return;
    }
    if rt.walk_bike_surf() != 0 {
        rt.play_sound(SoundId::STOP_ALL_MUSIC);
    }
    rt.play_music(sounds::MUSIC_MEET_RIVAL);
    rt.clear_joy_held();
    rt.joy_ignore(PAD_CTRL_PAD);
    // He is one sprite for both squares: standing over the left one, he is moved across first.
    if rt.x() != RIGHT_OF_BRIDGE {
        let mut at = rt.sprite_position(CERULEANCITY_RIVAL);
        at.map_x = 25;
        rt.set_sprite_position(CERULEANCITY_RIVAL, at);
    }
    rt.show_object(TOGGLE_CERULEAN_RIVAL);
    rt.move_sprite(CERULEANCITY_RIVAL, &RIVAL_ARRIVES);
    rt.maps().cerulean_city.cur_script = SCRIPT_CERULEANCITY_RIVAL_BATTLE;
    Flow::Return
}

/// `CeruleanCityFaceRivalScript`.
fn face_rival(rt: &mut Script, then: Label) -> Flow {
    rt.set_sprite_facing_direction_and_delay(CERULEANCITY_RIVAL, SPRITE_FACING_DOWN).then(then)
}

/// `CeruleanCityRivalBattleScript`.
fn rival_battle(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.display_text_id(TEXT_CERULEANCITY_RIVAL).then(Label::RivalBattleText)
}

/// `CeruleanCityRivalDefeatedScript` up to the words.
fn rival_defeated(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return clear_scripts(rt);
    }
    face_rival(rt, Label::RivalDefeatedFaces)
}

/// `CeruleanCityRivalCleanupScript`.
fn rival_cleanup(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.hide_object(TOGGLE_CERULEAN_RIVAL);
    rt.joy_ignore(Joypad::empty());
    rt.maps().cerulean_city.cur_script = SCRIPT_CERULEANCITY_DEFAULT;
    rt.play_default_music().ret()
}

/// `CeruleanCityRocketDefeatedScript`.
fn rocket_defeated(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return clear_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_event(EVENT_BEAT_CERULEAN_ROCKET_THIEF);
    rt.display_text_id(TEXT_CERULEANCITY_ROCKET).then(Label::RocketDefeatedText)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_CERULEANCITY_RIVAL => {
            let words = match rt.check_event(EVENT_BEAT_CERULEAN_RIVAL) {
                true => sym::CeruleanCityRivalIWentToBillsText,
                false => local::CeruleanCityRivalText::PreBattleText,
            };
            rt.print_text(text_at(words)).ret()
        }
        TEXT_CERULEANCITY_ROCKET => rocket_text(rt),
        // Two people watch the same Slowbro and neither is obeyed; which line either says is rolled.
        TEXT_CERULEANCITY_COOLTRAINER_F1 => {
            use local::CeruleanCityCooltrainerF1Text as words;
            let roll = rt.random();
            let said = match roll {
                180.. => words::SlowbroUseSonicboomText,
                100.. => words::SlowbroPunchText,
                _ => words::SlowbroWithdrawText,
            };
            rt.print_text(text_at(said)).ret()
        }
        TEXT_CERULEANCITY_SLOWBRO => {
            use local::CeruleanCitySlowbroText as words;
            let roll = rt.random();
            let said = match roll {
                180.. => words::TookASnoozeText,
                120.. => words::IsLoafingAroundText,
                60.. => words::TurnedAwayText,
                _ => words::IgnoredOrdersText,
            };
            rt.print_text(text_at(said)).ret()
        }
        _ => return None,
    })
}

/// `CeruleanCityRocketText`: he fights on the spot, and once beaten hands over TM28 and goes.
fn rocket_text(rt: &mut Script) -> Flow {
    use local::CeruleanCityRocketText as words;
    if !rt.check_event(EVENT_BEAT_CERULEAN_ROCKET_THIEF) {
        return rt.print_text(text_at(words::Text)).then(Label::RocketBattleText);
    }
    rt.print_text(text_at(words::IllReturnTheTMText)).then(Label::RocketTmText)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    use local::CeruleanCityRocketText as rocket;
    match label {
        Label::RocketGreeting => rt.display_text_id(TEXT_CERULEANCITY_ROCKET).ret(),
        Label::RivalBattleText => {
            rt.save_end_battle_text(sym::CeruleanCityRivalDefeatedText);
            let set = match PokemonSpecies::from_repr(rt.globals().rival_starter) {
                Some(PokemonSpecies::Squirtle) => 7,
                Some(PokemonSpecies::Bulbasaur) => 8,
                _ => 9,
            };
            rt.start_trainer_battle(OPP_RIVAL1, set, sym::CeruleanCityRivalDefeatedText);
            rt.clear_joy_held();
            rt.maps().cerulean_city.cur_script = SCRIPT_CERULEANCITY_RIVAL_DEFEATED;
            rt.set_sprite_facing_direction_and_delay(CERULEANCITY_RIVAL, SPRITE_FACING_DOWN).ret()
        }
        Label::RivalDefeatedFaces => {
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.set_event(EVENT_BEAT_CERULEAN_RIVAL);
            rt.display_text_id(TEXT_CERULEANCITY_RIVAL).then(Label::RivalDefeatedText)
        }
        Label::RivalDefeatedText => {
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_start();
            let path = match rt.x() == RIGHT_OF_BRIDGE {
                true => RIVAL_LEAVES_RIGHT,
                false => RIVAL_LEAVES_LEFT,
            };
            rt.move_sprite(CERULEANCITY_RIVAL, &path);
            rt.maps().cerulean_city.cur_script = SCRIPT_CERULEANCITY_RIVAL_CLEANUP;
            Flow::Return
        }
        Label::RocketDefeatedText => {
            rt.joy_ignore(Joypad::empty());
            rt.maps().cerulean_city.cur_script = SCRIPT_CERULEANCITY_DEFAULT;
            Flow::Return
        }
        Label::RocketBattleText => {
            rt.save_end_battle_text(rocket::IGiveUpText);
            rt.engage_map_trainer(CERULEANCITY_ROCKET, 0);
            rt.maps().cerulean_city.cur_script = SCRIPT_CERULEANCITY_ROCKET_DEFEATED;
            Flow::Return
        }
        Label::RocketTmText => {
            if !rt.give_item(ItemId::Tm28Dig, 1) {
                return rt.print_text(text_at(rocket::TM28NoRoomText)).ret();
            }
            rt.set_do_not_wait_for_button_press(true);
            rt.print_text(text_at(rocket::ReceivedTM28Text)).then(Label::RocketTmReceived)
        }
        // `CeruleanHideRocket`: the hole in the wall is a guard again, behind a fade.
        Label::RocketTmReceived => rt.gb_fade_out_to_black().then(Label::RocketHidden),
        Label::RocketHidden => {
            rt.show_object(TOGGLE_CERULEAN_GUARD_1);
            rt.hide_object(TOGGLE_CERULEAN_GUARD_2);
            rt.hide_object(TOGGLE_CERULEAN_ROCKET);
            rt.gb_fade_in_from_black().ret()
        }
    }
}
