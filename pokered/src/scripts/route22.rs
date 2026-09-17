//! `Route22_Script`: the rival waiting west of Viridian, once before Brock and once before the
//! League, on the same two squares both times.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_1ST_ROUTE22_RIVAL_BATTLE, EVENT_2ND_ROUTE22_RIVAL_BATTLE,
    EVENT_BEAT_ROUTE22_RIVAL_1ST_BATTLE, EVENT_BEAT_ROUTE22_RIVAL_2ND_BATTLE, EVENT_ROUTE22_RIVAL_WANTS_BATTLE};
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::{ROUTE22_RIVAL1, ROUTE22_RIVAL2};
use poke_core::symbols::pokered_toggles::{TOGGLE_ROUTE_22_RIVAL_1, TOGGLE_ROUTE_22_RIVAL_2};
use poke_core::trainer_headers::OPP_ID_OFFSET;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP};
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_RIGHT, SPRITE_FACING_UP};
use super::{text_at, Flow, Script};

/// `PLAYER_DIR_LEFT` and `PLAYER_DIR_DOWN`.
const PLAYER_DIR_LEFT: u8 = 2;
const PLAYER_DIR_DOWN: u8 = 4;
/// `EXCLAMATION_BUBBLE`.
const EXCLAMATION_BUBBLE: u8 = 0;
const END: u8 = 0xFF;
/// `OPP_RIVAL1` and `OPP_RIVAL2`.
const OPP_RIVAL1: u8 = OPP_ID_OFFSET + 25;
const OPP_RIVAL2: u8 = OPP_ID_OFFSET + 42;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `Route22DefaultScript.Route22RivalBattleCoords`, as (x, y). The lower of the two is index 1, and
/// everything the rival does afterwards is measured from which of them the player is standing on.
const BATTLE_COORDS: [(u8, u8); 2] = [(29, 4), (29, 5)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute22CurScript`.
    pub cur_script: u8,
    /// `wSavedCoordIndex`.
    pub coord_index: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Route22MoveRivalRightScript`'s `jp SetSpriteFacingDirectionAndDelay`, which returns.
    MoveDone,
    Rival1Bubble,
    Rival1Faces,
    Rival1Text,
    Rival1AfterFaces,
    Rival1AfterText,
    Rival2Bubble,
    Rival2Faces,
    Rival2Text,
    Rival2AfterFaces,
    Rival2AfterText,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().route22.cur_script {
        SCRIPT_ROUTE22_DEFAULT => default_script(rt),
        SCRIPT_ROUTE22_RIVAL1_START_BATTLE => rival_start_battle(rt, ROUTE22_RIVAL1, Label::Rival1Faces),
        SCRIPT_ROUTE22_RIVAL1_AFTER_BATTLE => rival1_after_battle(rt),
        SCRIPT_ROUTE22_RIVAL1_EXIT => rival_exit(rt, TOGGLE_ROUTE_22_RIVAL_1, EVENT_1ST_ROUTE22_RIVAL_BATTLE, SCRIPT_ROUTE22_DEFAULT),
        SCRIPT_ROUTE22_RIVAL2_START_BATTLE => rival_start_battle(rt, ROUTE22_RIVAL2, Label::Rival2Faces),
        SCRIPT_ROUTE22_RIVAL2_AFTER_BATTLE => rival2_after_battle(rt),
        SCRIPT_ROUTE22_RIVAL2_EXIT => rival_exit(rt, TOGGLE_ROUTE_22_RIVAL_2, EVENT_2ND_ROUTE22_RIVAL_BATTLE, SCRIPT_ROUTE22_NOOP),
        _ => Flow::Return,
    }
}

/// `Route22DefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_ROUTE22_RIVAL_WANTS_BATTLE) {
        return Flow::Return;
    }
    let Some(index) = rt.are_player_coords_in_array(&BATTLE_COORDS) else {
        return Flow::Return;
    };
    rt.maps().route22.coord_index = index;
    rt.clear_joy_held();
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_player_moving_direction(PLAYER_DIR_LEFT);
    if rt.check_event(EVENT_1ST_ROUTE22_RIVAL_BATTLE) {
        return rt.emotion_bubble(ROUTE22_RIVAL1, EXCLAMATION_BUBBLE).then(Label::Rival1Bubble);
    }
    if rt.check_event(EVENT_2ND_ROUTE22_RIVAL_BATTLE) {
        return rt.emotion_bubble(ROUTE22_RIVAL2, EXCLAMATION_BUBBLE).then(Label::Rival2Bubble);
    }
    Flow::Return
}

/// `Route22MoveRivalRightScript`: the player on the upper square is met a square sooner, so the
/// list's first step is skipped.
fn move_rival_right(rt: &mut Script, slot: u8) -> Flow {
    let steps = if rt.maps().route22.coord_index == 1 { 4 } else { 3 };
    let mut path = vec![NPC_MOVEMENT_RIGHT; steps];
    path.push(END);
    rt.move_sprite(slot, &path);
    rt.set_sprite_facing_direction_and_delay(slot, SPRITE_FACING_RIGHT).then(Label::MoveDone)
}

/// `Route22Rival1StartBattleScript` and `Route22Rival2StartBattleScript`, which differ only in the
/// player's direction: the rival walks alongside one square and in front of the other.
fn rival_start_battle(rt: &mut Script, slot: u8, then: Label) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    let facing = if rt.maps().route22.coord_index == 1 {
        rt.set_player_moving_direction(PLAYER_DIR_DOWN);
        SPRITE_FACING_UP
    } else {
        rt.set_player_moving_direction(PLAYER_DIR_LEFT);
        SPRITE_FACING_RIGHT
    };
    rt.set_sprite_facing_direction_and_delay(slot, facing).then(then)
}

/// `Route22GetRivalTrainerNoByStarterScript` over a start battle script's `.StarterTable`.
fn trainer_no(rt: &mut Script, table: [u8; 3]) -> u8 {
    match PokemonSpecies::from_repr(rt.globals().rival_starter) {
        Some(PokemonSpecies::Squirtle) => table[0],
        Some(PokemonSpecies::Bulbasaur) => table[1],
        _ => table[2],
    }
}

/// `Route22Rival1AfterBattleScript` up to its words: a battle lost puts the map back as it was.
fn rival1_after_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return set_default_script(rt);
    }
    let facing = match rt.player_facing() == SPRITE_FACING_DOWN {
        true => SPRITE_FACING_UP,
        false => SPRITE_FACING_RIGHT,
    };
    rt.set_sprite_facing_direction_and_delay(ROUTE22_RIVAL1, facing).then(Label::Rival1AfterFaces)
}

/// `Route22Rival2AfterBattleScript`, which turns the player as well as the rival.
fn rival2_after_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return set_default_script(rt);
    }
    let facing = if rt.maps().route22.coord_index == 1 {
        rt.set_player_moving_direction(PLAYER_DIR_DOWN);
        SPRITE_FACING_UP
    } else {
        rt.set_player_moving_direction(PLAYER_DIR_LEFT);
        SPRITE_FACING_RIGHT
    };
    rt.set_sprite_facing_direction_and_delay(ROUTE22_RIVAL2, facing).then(Label::Rival2AfterFaces)
}

/// `Route22SetDefaultScript`.
fn set_default_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().route22.cur_script = SCRIPT_ROUTE22_DEFAULT;
    Flow::Return
}

/// `Route22Rival1ExitMovementData1` and `2`, and the second rival's pair: from the lower square he
/// leaves to the right and down, from the upper one he has to come round the player first.
fn rival1_exit_path(index: u8) -> Vec<u8> {
    match index == 1 {
        true => vec![NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
            NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END],
        false => vec![NPC_MOVEMENT_UP, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT,
            NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
            NPC_MOVEMENT_DOWN, END],
    }
}

/// The second rival walks off west, a square further from the lower square than the upper.
fn rival2_exit_path(index: u8) -> Vec<u8> {
    let steps = if index == 1 { 4 } else { 3 };
    let mut path = vec![NPC_MOVEMENT_LEFT; steps];
    path.push(END);
    path
}

/// `Route22Rival1ExitScript` and `Route22Rival2ExitScript`.
fn rival_exit(rt: &mut Script, toggle: u16, battle_event: u16, next: u8) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.hide_object(toggle);
    rt.reset_event(battle_event);
    rt.reset_event(EVENT_ROUTE22_RIVAL_WANTS_BATTLE);
    rt.maps().route22.cur_script = next;
    rt.play_default_music().ret()
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let words = match text_id {
        TEXT_ROUTE22_RIVAL1 => match rt.check_event(EVENT_BEAT_ROUTE22_RIVAL_1ST_BATTLE) {
            true => sym::Route22RivalAfterBattleText1,
            false => sym::Route22RivalBeforeBattleText1,
        },
        TEXT_ROUTE22_RIVAL2 => match rt.check_event(EVENT_BEAT_ROUTE22_RIVAL_2ND_BATTLE) {
            true => sym::Route22RivalAfterBattleText2,
            false => sym::Route22RivalBeforeBattleText2,
        },
        _ => return None,
    };
    Some(rt.print_text(text_at(words)).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::MoveDone => Flow::Return,
        Label::Rival1Bubble => {
            if rt.walk_bike_surf() != 0 {
                rt.play_sound(SoundId::STOP_ALL_MUSIC);
            }
            rt.play_music(sounds::MUSIC_MEET_RIVAL);
            rt.maps().route22.cur_script = SCRIPT_ROUTE22_RIVAL1_START_BATTLE;
            move_rival_right(rt, ROUTE22_RIVAL1)
        }
        Label::Rival1Faces => {
            rt.joy_ignore(Joypad::empty());
            rt.display_text_id(TEXT_ROUTE22_RIVAL1).then(Label::Rival1Text)
        }
        Label::Rival1Text => {
            let set = trainer_no(rt, [4, 5, 6]);
            rt.start_trainer_battle(OPP_RIVAL1, set, sym::Route22Rival1DefeatedText);
            rt.maps().route22.cur_script = SCRIPT_ROUTE22_RIVAL1_AFTER_BATTLE;
            Flow::Return
        }
        Label::Rival1AfterFaces => {
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.set_event(EVENT_BEAT_ROUTE22_RIVAL_1ST_BATTLE);
            rt.display_text_id(TEXT_ROUTE22_RIVAL1).then(Label::Rival1AfterText)
        }
        Label::Rival1AfterText => {
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_start();
            let path = rival1_exit_path(rt.maps().route22.coord_index);
            rt.move_sprite(ROUTE22_RIVAL1, &path);
            rt.maps().route22.cur_script = SCRIPT_ROUTE22_RIVAL1_EXIT;
            Flow::Return
        }

        Label::Rival2Bubble => {
            if rt.walk_bike_surf() != 0 {
                rt.play_sound(SoundId::STOP_ALL_MUSIC);
            }
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_tempo();
            rt.maps().route22.cur_script = SCRIPT_ROUTE22_RIVAL2_START_BATTLE;
            move_rival_right(rt, ROUTE22_RIVAL2)
        }
        Label::Rival2Faces => {
            rt.joy_ignore(Joypad::empty());
            rt.display_text_id(TEXT_ROUTE22_RIVAL2).then(Label::Rival2Text)
        }
        Label::Rival2Text => {
            let set = trainer_no(rt, [10, 11, 12]);
            rt.start_trainer_battle(OPP_RIVAL2, set, sym::Route22Rival2DefeatedText);
            rt.maps().route22.cur_script = SCRIPT_ROUTE22_RIVAL2_AFTER_BATTLE;
            Flow::Return
        }
        Label::Rival2AfterFaces => {
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.set_event(EVENT_BEAT_ROUTE22_RIVAL_2ND_BATTLE);
            rt.display_text_id(TEXT_ROUTE22_RIVAL2).then(Label::Rival2AfterText)
        }
        Label::Rival2AfterText => {
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_start_and_tempo();
            let path = rival2_exit_path(rt.maps().route22.coord_index);
            rt.move_sprite(ROUTE22_RIVAL2, &path);
            rt.maps().route22.cur_script = SCRIPT_ROUTE22_RIVAL2_EXIT;
            Flow::Return
        }
    }
}
