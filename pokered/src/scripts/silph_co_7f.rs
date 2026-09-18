//! `SilphCo7F_Script`: the rival waiting in the middle of the floor, three Rockets and a scientist,
//! three card key doors, and the Silph worker who parts with his Lapras.

use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_BEAT_SILPH_CO_GIOVANNI, EVENT_BEAT_SILPH_CO_RIVAL,
    EVENT_SILPH_CO_7_UNLOCKED_DOOR1, EVENT_SILPH_CO_7_UNLOCKED_DOOR2, EVENT_SILPH_CO_7_UNLOCKED_DOOR3};
use poke_core::symbols::pokered_local_labels::{SilphCo7FRivalText as rival,
    SilphCo7FSilphWorkerM1Text as worker_m1, SilphCo7FSilphWorkerM2Text as worker_m2,
    SilphCo7FSilphWorkerM3Text as worker_m3, SilphCo7FSilphWorkerM4Text as worker_m4};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SILPHCO7F_DEFAULT, SCRIPT_SILPHCO7F_RIVAL_AFTER_BATTLE,
    SCRIPT_SILPHCO7F_RIVAL_EXIT, SCRIPT_SILPHCO7F_RIVAL_START_BATTLE, TEXT_SILPHCO7F_RIVAL,
    TEXT_SILPHCO7F_RIVAL_GOOD_LUCK_TO_YOU, TEXT_SILPHCO7F_RIVAL_WAITED_HERE, TEXT_SILPHCO7F_ROCKET1,
    TEXT_SILPHCO7F_ROCKET2, TEXT_SILPHCO7F_ROCKET3, TEXT_SILPHCO7F_SCIENTIST, TEXT_SILPHCO7F_SILPH_WORKER_M1,
    TEXT_SILPHCO7F_SILPH_WORKER_M2, TEXT_SILPHCO7F_SILPH_WORKER_M3, TEXT_SILPHCO7F_SILPH_WORKER_M4};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::SILPHCO7F_RIVAL;
use poke_core::symbols::pokered_toggles::TOGGLE_SILPH_CO_7F_RIVAL;
use poke_core::trainer_headers::OPP_ID_OFFSET;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP};
use crate::systems::overworld::sprites::SPRITE_FACING_UP;
use super::silph_co::beat_giovanni_print_de_or_print_hl as print_held_or_freed;
use super::{text_at, Code, Flow, Script};

/// `PLAYER_DIR_DOWN`.
const PLAYER_DIR_DOWN: u8 = 4;
const END: u8 = 0xFF;
/// `OPP_RIVAL2`.
const OPP_RIVAL2: u8 = OPP_ID_OFFSET + 42;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// The Lapras the worker hands over.
const LAPRAS_LEVEL: u8 = 15;

/// The block a card key door is drawn as while it is still shut.
const CLOSED_DOOR: u8 = 0x54;
/// `SilphCo7F_GateCallbackScript.GateCoordinates`, in blocks, and the event each gate has.
const GATES: [(u8, u8); 3] = [(5, 3), (10, 2), (10, 6)];
const DOORS: [u16; 3] = [EVENT_SILPH_CO_7_UNLOCKED_DOOR1, EVENT_SILPH_CO_7_UNLOCKED_DOOR2,
    EVENT_SILPH_CO_7_UNLOCKED_DOOR3];

/// `SilphCo7FDefaultScript.RivalEncounterCoordinates`, as (x, y). The upper of the two is index 1,
/// and every step the rival takes afterwards is measured from which of them the player came in on.
const RIVAL_COORDS: [(u8, u8); 2] = [(3, 2), (3, 3)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo7FCurScript`.
    pub cur_script: u8,
    /// `wSavedCoordIndex`.
    pub saved_coord_index: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    RivalStartBattle,
    RivalAfterBattle,
    RivalExit,
    /// `ld [wSilphCo7FCurScript], a` after the table's routine.
    StoreCurScript,
    RivalChallenged,
    RivalWaited,
    RivalWaitedDelay,
    RivalTurned,
    GoodLuckSaid,
    MusicBack,
    LaprasOffered,
    LaprasGiven,
    BoxTextRead,
    LaprasDescribed,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo7F_GateCallbackScript`.
    super::silph_co::gate_callback(rt, &GATES, &DOORS, CLOSED_DOOR);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_7f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo7TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_SILPHCO7F_DEFAULT => Label::DefaultScript.into(),
        SCRIPT_SILPHCO7F_RIVAL_START_BATTLE => Label::RivalStartBattle.into(),
        SCRIPT_SILPHCO7F_RIVAL_AFTER_BATTLE => Label::RivalAfterBattle.into(),
        SCRIPT_SILPHCO7F_RIVAL_EXIT => Label::RivalExit.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `SilphCo7FSetCurScript`.
fn set_script(rt: &mut Script, index: u8) {
    rt.maps().silph_co_7f.cur_script = index;
    rt.set_cur_map_script(index);
}

/// `SilphCo7FSetDefaultScript`.
fn set_default_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_SILPHCO7F_DEFAULT);
    Flow::Return
}

/// `SilphCo7FDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_BEAT_SILPH_CO_RIVAL) {
        return rt.trainer_script(SCRIPT_SILPHCO7F_DEFAULT).ret();
    }
    let Some(index) = rt.are_player_coords_in_array(&RIVAL_COORDS) else {
        return rt.trainer_script(SCRIPT_SILPHCO7F_DEFAULT).ret();
    };
    rt.clear_joy_held();
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_player_moving_direction(PLAYER_DIR_DOWN);
    rt.play_sound(SoundId::STOP_ALL_MUSIC);
    rt.play_music(sounds::MUSIC_MEET_RIVAL);
    // The cartridge reads `wCoordIndex` after the words, where nothing has moved the player.
    rt.maps().silph_co_7f.saved_coord_index = index;
    rt.display_text_id(TEXT_SILPHCO7F_RIVAL).then(Label::RivalChallenged)
}

/// `SilphCo7FDefaultScript.RivalMovementUp`: the player on the upper square is met a square sooner,
/// so the list's first step is skipped.
fn rival_walk_up_path(index: u8) -> Vec<u8> {
    let steps = if index == 1 { 4 } else { 3 };
    let mut path = vec![NPC_MOVEMENT_UP; steps];
    path.push(END);
    path
}

/// `SilphCo7FRivalAfterBattleScript.RivalExitRightMovement` and `.RivalWalkAroundPlayerMovement`:
/// both end on the stairwell west of him, but from the lower square the player is in the way.
fn rival_exit_path(index: u8) -> Vec<u8> {
    match index == 1 {
        true => vec![NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, END],
        false => vec![NPC_MOVEMENT_LEFT, NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, NPC_MOVEMENT_RIGHT,
            NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_DOWN, END],
    }
}

/// `SilphCo7FRivalStartBattleScript`'s starter branch: which of the rival's three parties answers
/// the starter he took.
fn rival_trainer_no(rt: &mut Script) -> u8 {
    match PokemonSpecies::from_repr(rt.globals().rival_starter) {
        Some(PokemonSpecies::Squirtle) => 7,
        Some(PokemonSpecies::Bulbasaur) => 8,
        _ => 9,
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let (held, freed) = match text_id {
        TEXT_SILPHCO7F_SILPH_WORKER_M1 => return Some(silph_worker_m1(rt)),
        TEXT_SILPHCO7F_SILPH_WORKER_M2 => (worker_m2::AfterTheMasterBallText, worker_m2::CancelledTheMasterBallText),
        TEXT_SILPHCO7F_SILPH_WORKER_M3 => (worker_m3::ItWouldBeBadText, worker_m3::YouChasedOffTeamRocketText),
        TEXT_SILPHCO7F_SILPH_WORKER_M4 => (worker_m4::ItsReallyDangerousHereText, worker_m4::SafeAtLastText),
        TEXT_SILPHCO7F_ROCKET1 => return Some(rt.talk_to_trainer(sym::SilphCo7TrainerHeader0).ret()),
        TEXT_SILPHCO7F_SCIENTIST => return Some(rt.talk_to_trainer(sym::SilphCo7TrainerHeader1).ret()),
        TEXT_SILPHCO7F_ROCKET2 => return Some(rt.talk_to_trainer(sym::SilphCo7TrainerHeader2).ret()),
        TEXT_SILPHCO7F_ROCKET3 => return Some(rt.talk_to_trainer(sym::SilphCo7TrainerHeader3).ret()),
        TEXT_SILPHCO7F_RIVAL => return Some(rt.print_text(text_at(rival::Text)).ret()),
        _ => return None,
    };
    Some(print_held_or_freed(rt, held, freed))
}

/// `SilphCo7FSilphWorkerM1Text`: the Lapras comes first, so this is the one worker on the floor who
/// has nothing to say about Giovanni until he has parted with it.
fn silph_worker_m1(rt: &mut Script) -> Flow {
    if !rt.globals().got_lapras {
        return rt.print_text(text_at(worker_m1::HaveThisPokemonText)).then(Label::LaprasOffered);
    }
    let said = match rt.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
        true => worker_m1::SavedText,
        false => worker_m1::IsOurPresidentOkText,
    };
    rt.print_text(text_at(said)).ret()
}

/// The description he gives once the Lapras is the player's, whichever of the party or a box it
/// went into.
fn describe_lapras(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    rt.print_text(text_at(worker_m1::LaprasDescriptionText)).then(Label::LaprasDescribed)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::StoreCurScript => {
            rt.maps().silph_co_7f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        // He walks up to stand in front of whoever has come this far.
        Label::RivalChallenged => {
            rt.set_sprite_movement_bytes_to_ff(SILPHCO7F_RIVAL);
            let path = rival_walk_up_path(rt.maps().silph_co_7f.saved_coord_index);
            rt.move_sprite(SILPHCO7F_RIVAL, &path);
            set_script(rt, SCRIPT_SILPHCO7F_RIVAL_START_BATTLE);
            Flow::Return
        }
        // `SilphCo7FRivalStartBattleScript`.
        Label::RivalStartBattle => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.joy_ignore(Joypad::empty());
            rt.display_text_id(TEXT_SILPHCO7F_RIVAL_WAITED_HERE).then(Label::RivalWaited)
        }
        Label::RivalWaited => rt.delay3().then(Label::RivalWaitedDelay),
        Label::RivalWaitedDelay => {
            let set = rival_trainer_no(rt);
            rt.start_trainer_battle(OPP_RIVAL2, set, sym::SilphCo7FRivalDefeatedText);
            set_script(rt, SCRIPT_SILPHCO7F_RIVAL_AFTER_BATTLE);
            Flow::Return
        }
        // `SilphCo7FRivalAfterBattleScript`.
        Label::RivalAfterBattle => {
            if rt.lost_battle() {
                return set_default_script(rt);
            }
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.set_event(EVENT_BEAT_SILPH_CO_RIVAL);
            rt.set_player_moving_direction(PLAYER_DIR_DOWN);
            rt.set_sprite_facing_direction_and_delay(SILPHCO7F_RIVAL, SPRITE_FACING_UP).then(Label::RivalTurned)
        }
        Label::RivalTurned => {
            rt.display_text_id(TEXT_SILPHCO7F_RIVAL_GOOD_LUCK_TO_YOU).then(Label::GoodLuckSaid)
        }
        Label::GoodLuckSaid => {
            rt.play_sound(SoundId::STOP_ALL_MUSIC);
            rt.music_rival_alternate_start();
            let path = rival_exit_path(rt.maps().silph_co_7f.saved_coord_index);
            rt.move_sprite(SILPHCO7F_RIVAL, &path);
            set_script(rt, SCRIPT_SILPHCO7F_RIVAL_EXIT);
            Flow::Return
        }
        // `SilphCo7FRivalExitScript`: he is gone for the rest of the building.
        Label::RivalExit => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.hide_object(TOGGLE_SILPH_CO_7F_RIVAL);
            rt.play_default_music().then(Label::MusicBack)
        }
        Label::MusicBack => set_default_script(rt),
        Label::LaprasOffered => rt.give_pokemon(PokemonSpecies::Lapras, LAPRAS_LEVEL).then(Label::LaprasGiven),
        Label::LaprasGiven => {
            if !rt.gave_pokemon() {
                return Flow::Return;
            }
            match rt.added_to_party() {
                true => describe_lapras(rt),
                false => rt.wait_for_text_scroll_button_press().then(Label::BoxTextRead),
            }
        }
        Label::BoxTextRead => describe_lapras(rt),
        Label::LaprasDescribed => {
            rt.globals().got_lapras = true;
            Flow::Return
        }
    }
}
