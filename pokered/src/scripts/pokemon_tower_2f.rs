//! `PokemonTower2F_Script`: the rival on the stairs, who fights and then leaves by whichever side
//! of the player he was standing on.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_BEAT_POKEMON_TOWER_RIVAL, EVENT_POKEMON_TOWER_RIVAL_ON_LEFT};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_POKEMONTOWER2F_DEFAULT, SCRIPT_POKEMONTOWER2F_DEFEATED_RIVAL,
    SCRIPT_POKEMONTOWER2F_RIVAL_EXITS, TEXT_POKEMONTOWER2F_RIVAL};
use poke_core::symbols::pokered_symbols::POKEMONTOWER2F_RIVAL;
use poke_core::symbols::pokered_toggles::TOGGLE_POKEMON_TOWER_2F_RIVAL;
use poke_core::trainer_headers::OPP_ID_OFFSET;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT};
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_RIGHT};
use super::{text_at, Flow, Script};

/// `PLAYER_DIR_UP` and `PLAYER_DIR_LEFT`.
const PLAYER_DIR_UP: u8 = 8;
const PLAYER_DIR_LEFT: u8 = 2;
const END: u8 = 0xFF;
/// `OPP_RIVAL2`.
const OPP_RIVAL2: u8 = OPP_ID_OFFSET + 42;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `PokemonTower2FRivalEncounterEventCoords`: the square east of the rival and the square south of
/// him. The cartridge's list ends in `$0F` rather than `$FF`, so its search runs off into the code
/// after it; no square it then reads is one the player can stand on.
const RIVAL_COORDS: [(u8, u8); 2] = [(15, 5), (14, 6)];

const RIVAL_RIGHT_THEN_DOWN: [u8; 9] = [NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, END];
const RIVAL_DOWN_THEN_RIGHT: [u8; 9] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT,
    NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPokemonTower2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    RivalFaced,
    RivalSpokeTo,
    RivalGreeted,
    RivalDefeatedText,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().pokemon_tower_2f.cur_script {
        SCRIPT_POKEMONTOWER2F_DEFEATED_RIVAL => defeated_rival(rt),
        SCRIPT_POKEMONTOWER2F_RIVAL_EXITS => rival_exits(rt),
        _ => default_script(rt),
    }
}

/// `PokemonTower2FResetRivalEncounter`.
fn reset_rival_encounter(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().pokemon_tower_2f.cur_script = SCRIPT_POKEMONTOWER2F_DEFAULT;
    Flow::Return
}

/// `PokemonTower2FDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_BEAT_POKEMON_TOWER_RIVAL) {
        return Flow::Return;
    }
    let Some(index) = rt.are_player_coords_in_array(&RIVAL_COORDS) else {
        return Flow::Return;
    };
    rt.play_sound(SoundId::STOP_ALL_MUSIC);
    rt.play_music(sounds::MUSIC_MEET_RIVAL);
    rt.reset_event(EVENT_POKEMON_TOWER_RIVAL_ON_LEFT);
    let (direction, facing) = match index {
        1 => {
            rt.set_event(EVENT_POKEMON_TOWER_RIVAL_ON_LEFT);
            (PLAYER_DIR_LEFT, SPRITE_FACING_RIGHT)
        }
        _ => (PLAYER_DIR_UP, SPRITE_FACING_DOWN),
    };
    rt.set_player_moving_direction(direction);
    rt.set_sprite_facing_direction_and_delay(POKEMONTOWER2F_RIVAL, facing).then(Label::RivalFaced)
}

/// `PokemonTower2FDefeatedRivalScript` up to the words.
fn defeated_rival(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_rival_encounter(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_event(EVENT_BEAT_POKEMON_TOWER_RIVAL);
    rt.display_text_id(TEXT_POKEMONTOWER2F_RIVAL).then(Label::RivalDefeatedText)
}

/// `PokemonTower2FRivalExitsScript`.
fn rival_exits(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.hide_object(TOGGLE_POKEMON_TOWER_2F_RIVAL);
    rt.joy_ignore(Joypad::empty());
    rt.maps().pokemon_tower_2f.cur_script = SCRIPT_POKEMONTOWER2F_DEFAULT;
    rt.play_default_music().ret()
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    use local::PokemonTower2FRivalText as words;
    match text_id {
        TEXT_POKEMONTOWER2F_RIVAL => Some(match rt.check_event(EVENT_BEAT_POKEMON_TOWER_RIVAL) {
            true => rt.print_text(text_at(words::HowsYourDexText)).ret(),
            false => rt.print_text(text_at(words::WhatBringsYouHereText)).then(Label::RivalGreeted),
        }),
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    use local::PokemonTower2FRivalText as words;
    match label {
        Label::RivalFaced => rt.display_text_id(TEXT_POKEMONTOWER2F_RIVAL).then(Label::RivalSpokeTo),
        Label::RivalSpokeTo => {
            rt.clear_joy_held();
            Flow::Return
        }
        Label::RivalGreeted => {
            let set = match PokemonSpecies::from_repr(rt.globals().rival_starter) {
                Some(PokemonSpecies::Squirtle) => 4,
                Some(PokemonSpecies::Bulbasaur) => 5,
                _ => 6,
            };
            rt.start_trainer_battle(OPP_RIVAL2, set, words::DefeatedText);
            rt.maps().pokemon_tower_2f.cur_script = SCRIPT_POKEMONTOWER2F_DEFEATED_RIVAL;
            Flow::Return
        }
        Label::RivalDefeatedText => {
            let path = match rt.check_event(EVENT_POKEMON_TOWER_RIVAL_ON_LEFT) {
                true => RIVAL_DOWN_THEN_RIGHT,
                false => RIVAL_RIGHT_THEN_DOWN,
            };
            rt.move_sprite(POKEMONTOWER2F_RIVAL, &path);
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_start();
            rt.maps().pokemon_tower_2f.cur_script = SCRIPT_POKEMONTOWER2F_RIVAL_EXITS;
            Flow::Return
        }
    }
}
