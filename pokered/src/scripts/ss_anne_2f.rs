//! `SSAnne2F_Script`: the rival waiting in the corridor, who has to be beaten before the captain's
//! cabin at the end of it is worth walking to.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SSANNE2F_DEFAULT, SCRIPT_SSANNE2F_NOOP,
    SCRIPT_SSANNE2F_RIVAL_AFTER_BATTLE, SCRIPT_SSANNE2F_RIVAL_EXIT, SCRIPT_SSANNE2F_RIVAL_START_BATTLE,
    TEXT_SSANNE2F_RIVAL, TEXT_SSANNE2F_RIVAL_CUT_MASTER};
use poke_core::symbols::pokered_local_labels::SSAnne2FRivalText;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::SSANNE2F_RIVAL;
use poke_core::symbols::pokered_toggles::TOGGLE_SS_ANNE_2F_RIVAL;
use poke_core::trainer_headers::OPP_ID_OFFSET;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT};
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_RIGHT};
use super::{text_at, Flow, Script};

/// `OPP_RIVAL2`.
const OPP_RIVAL2: u8 = OPP_ID_OFFSET + 0x2A;
/// `PLAYER_DIR_LEFT`.
const PLAYER_DIR_LEFT: u8 = 2;
const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `.PlayerCoordinatesArray`: the two squares of the corridor he drops into.
const RIVAL_COORDS: [(u8, u8); 2] = [(36, 8), (37, 8)];
/// The right-hand square, which he walks down beside rather than in front of.
const RIGHT_OF_CORRIDOR: u8 = 37;
const RIVAL_DOWN_THREE: [u8; 4] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];
const RIVAL_DOWN_FOUR: [u8; 5] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];
const RIVAL_AROUND_PLAYER: [u8; 7] = [NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN,
    NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSSAnne2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `SSAnne2FSetFacingDirectionScript`, whose delay every step of the cutscene waits out.
    FacedBeforeText,
    RivalText,
    FacedBeforeBattle,
    FacedAfterBattle,
    CutMasterText,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().ss_anne_2f.cur_script {
        SCRIPT_SSANNE2F_RIVAL_START_BATTLE => rival_start_battle(rt),
        SCRIPT_SSANNE2F_RIVAL_AFTER_BATTLE => rival_after_battle(rt),
        SCRIPT_SSANNE2F_RIVAL_EXIT => rival_exit(rt),
        SCRIPT_SSANNE2F_NOOP => Flow::Return,
        _ => default_script(rt),
    }
}

/// `SSAnne2FResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().ss_anne_2f.cur_script = SCRIPT_SSANNE2F_DEFAULT;
    Flow::Return
}

/// `SSAnne2FDefaultScript`: he comes down the corridor as far as the square the player is on lets him.
fn default_script(rt: &mut Script) -> Flow {
    let Some(index) = rt.are_player_coords_in_array(&RIVAL_COORDS) else {
        return Flow::Return;
    };
    rt.play_sound(SoundId::STOP_ALL_MUSIC);
    rt.play_music(sounds::MUSIC_MEET_RIVAL);
    rt.show_object(TOGGLE_SS_ANNE_2F_RIVAL);
    rt.set_sprite_movement_bytes_to_ff(SSANNE2F_RIVAL);
    rt.clear_joy_held();
    rt.joy_ignore(PAD_CTRL_PAD);
    let path: &[u8] = match index {
        2 => &RIVAL_DOWN_FOUR,
        _ => &RIVAL_DOWN_THREE,
    };
    rt.move_sprite(SSANNE2F_RIVAL, path);
    rt.maps().ss_anne_2f.cur_script = SCRIPT_SSANNE2F_RIVAL_START_BATTLE;
    Flow::Return
}

/// `SSAnne2FSetFacingDirectionScript`: beside the player he turns to face left, in front of them down.
fn set_facing_direction(rt: &mut Script, then: Label) -> Flow {
    let facing = match rt.x() == RIGHT_OF_CORRIDOR {
        true => {
            rt.set_player_moving_direction(PLAYER_DIR_LEFT);
            SPRITE_FACING_RIGHT
        }
        false => SPRITE_FACING_DOWN,
    };
    rt.set_sprite_facing_direction_and_delay(SSANNE2F_RIVAL, facing).then(then)
}

/// `SSAnne2FRivalStartBattleScript`.
fn rival_start_battle(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    set_facing_direction(rt, Label::FacedBeforeText)
}

/// `SSAnne2FRivalAfterBattleScript`.
fn rival_after_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    set_facing_direction(rt, Label::FacedAfterBattle)
}

/// `SSAnne2FRivalExitScript`.
fn rival_exit(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.hide_object(TOGGLE_SS_ANNE_2F_RIVAL);
    rt.maps().ss_anne_2f.cur_script = SCRIPT_SSANNE2F_NOOP;
    rt.play_default_music().ret()
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_SSANNE2F_RIVAL => {
            // His words arm the battle the script starts a pass later, with different words for
            // either outcome; only the one for a win is kept.
            rt.save_end_battle_text(sym::SSAnne2FRivalDefeatedText);
            Some(rt.print_text(text_at(SSAnne2FRivalText::Text)).ret())
        }
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::FacedBeforeText => {
            rt.joy_ignore(Joypad::empty());
            rt.display_text_id(TEXT_SSANNE2F_RIVAL).then(Label::RivalText)
        }
        Label::RivalText => {
            let set = match PokemonSpecies::from_repr(rt.globals().rival_starter) {
                Some(PokemonSpecies::Squirtle) => 1,
                Some(PokemonSpecies::Bulbasaur) => 2,
                _ => 3,
            };
            rt.start_trainer_battle(OPP_RIVAL2, set, sym::SSAnne2FRivalDefeatedText);
            set_facing_direction(rt, Label::FacedBeforeBattle)
        }
        Label::FacedBeforeBattle => {
            rt.maps().ss_anne_2f.cur_script = SCRIPT_SSANNE2F_RIVAL_AFTER_BATTLE;
            Flow::Return
        }
        Label::FacedAfterBattle => {
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.display_text_id(TEXT_SSANNE2F_RIVAL_CUT_MASTER).then(Label::CutMasterText)
        }
        Label::CutMasterText => {
            rt.set_sprite_movement_bytes_to_ff(SSANNE2F_RIVAL);
            let path: &[u8] = match rt.x() == RIGHT_OF_CORRIDOR {
                true => &RIVAL_DOWN_FOUR,
                false => &RIVAL_AROUND_PLAYER,
            };
            rt.move_sprite(SSANNE2F_RIVAL, path);
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_start();
            rt.maps().ss_anne_2f.cur_script = SCRIPT_SSANNE2F_RIVAL_EXIT;
            Flow::Return
        }
    }
}
