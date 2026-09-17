//! `LoreleisRoom_Script`: the first of the Elite Four, the door that seals behind the player and
//! the walk up to her.

use poke_core::symbols::pokered_events::{EVENT_AUTOWALKED_INTO_LORELEIS_ROOM, EVENT_BEAT_LORELEIS_ROOM_TRAINER_0};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_LORELEISROOM_DEFAULT, SCRIPT_LORELEISROOM_LORELEI_END_BATTLE,
    SCRIPT_LORELEISROOM_NOOP, SCRIPT_LORELEISROOM_PLAYER_IS_MOVING, TEXT_LORELEISROOM_DONT_RUN_AWAY,
    TEXT_LORELEISROOM_LORELEI};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{Code, Flow, Script};

/// `LoreleiEntranceCoords`, as (x, y): the two squares of the doorway and the two above them. The
/// first two are where the player is turned back, the last two where the walk in begins.
const ENTRANCE: [(u8, u8); 4] = [(4, 10), (5, 10), (4, 11), (5, 11)];
/// The first `wCoordIndex` that is a doorway square, the cartridge counting from one.
const DOORWAY: u8 = 3;
/// The exit north, as blocks: shut until Lorelei is beaten, and open after.
const EXIT_AT: (u8, u8) = (2, 0);
const EXIT_SHUT: u8 = 0x24;
const EXIT_OPEN: u8 = 0x05;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wLoreleisRoomCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    EndBattle,
    PlayerIsMoving,
    /// `ld [wLoreleisRoomCurScript], a` after the table's routine.
    StoreCurScript,
    DontRunAway,
    MovedIn,
    AfterEndTrainerBattle,
}

pub fn script(rt: &mut Script) -> Flow {
    show_or_hide_exit_block(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().loreleis_room.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::LoreleisRoomTrainerHeaders);
    let entry: Code = match index {
        SCRIPT_LORELEISROOM_DEFAULT => Label::DefaultScript.into(),
        SCRIPT_LORELEISROOM_LORELEI_END_BATTLE => Label::EndBattle.into(),
        SCRIPT_LORELEISROOM_PLAYER_IS_MOVING => Label::PlayerIsMoving.into(),
        // `LoreleisRoomNoopScript`, which the room sits in while the rest of the floor plays.
        SCRIPT_LORELEISROOM_NOOP => return Flow::Return,
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `LoreleiShowOrHideExitBlock`. Walking in is what arms `BIT_STARTED_ELITE_4`, which is how the
/// lobby knows to forget a challenge that was left half fought.
fn show_or_hide_exit_block(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    rt.globals().started_elite_4 = true;
    let block = match rt.check_event(EVENT_BEAT_LORELEIS_ROOM_TRAINER_0) {
        true => EXIT_OPEN,
        false => EXIT_SHUT,
    };
    rt.replace_tile_block(EXIT_AT.0, EXIT_AT.1, block);
}

/// `ld [wLoreleisRoomCurScript], a` and `ld [wCurMapScript], a`, which the room's scripts do
/// together.
fn set_script(rt: &mut Script, index: u8) {
    rt.maps().loreleis_room.cur_script = index;
    rt.set_cur_map_script(index);
}

/// `LoreleisRoomDefaultScript`: stepping into the doorway walks the player six squares in, and
/// stepping back down onto it once they are in is answered with one square back up.
fn default_script(rt: &mut Script) -> Flow {
    let Some(index) = rt.are_player_coords_in_array(&ENTRANCE) else {
        return rt.trainer_script(SCRIPT_LORELEISROOM_DEFAULT).ret();
    };
    rt.clear_joy_held();
    rt.stop_simulating_joypad_states();
    if index >= DOORWAY && !rt.check_and_set_event(EVENT_AUTOWALKED_INTO_LORELEIS_ROOM) {
        return walk_into_room(rt);
    }
    rt.display_text_id(TEXT_LORELEISROOM_DONT_RUN_AWAY).then(Label::DontRunAway)
}

/// `LoreleiScriptWalkIntoRoom`.
fn walk_into_room(rt: &mut Script) -> Flow {
    rt.simulate_joypad_presses(vec![Joypad::UP; 6]);
    set_script(rt, SCRIPT_LORELEISROOM_PLAYER_IS_MOVING);
    Flow::Return
}

/// `LoreleisRoomPlayerIsMovingScript`: the pad is the player's again once the walk has run out.
fn player_is_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::MovedIn)
}

/// `LoreleisRoomLoreleiEndBattleScript`.
fn end_battle(rt: &mut Script) -> Flow {
    rt.trainer_script(SCRIPT_LORELEISROOM_LORELEI_END_BATTLE).then(Label::AfterEndTrainerBattle)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_LORELEISROOM_LORELEI => Some(rt.talk_to_trainer(sym::LoreleisRoomTrainerHeader0).ret()),
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::EndBattle => end_battle(rt),
        Label::PlayerIsMoving => player_is_moving(rt),
        Label::StoreCurScript => {
            rt.maps().loreleis_room.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::DontRunAway => {
            rt.simulate_joypad_presses(vec![Joypad::UP]);
            set_script(rt, SCRIPT_LORELEISROOM_PLAYER_IS_MOVING);
            Flow::Return
        }
        Label::MovedIn => {
            rt.joy_ignore(Joypad::empty());
            set_script(rt, SCRIPT_LORELEISROOM_DEFAULT);
            Flow::Return
        }
        // `ResetLoreleiScript` where the player lost, and Lorelei's own words where they won.
        Label::AfterEndTrainerBattle => {
            if rt.lost_battle() {
                set_script(rt, SCRIPT_LORELEISROOM_DEFAULT);
                return Flow::Return;
            }
            rt.display_text_id(TEXT_LORELEISROOM_LORELEI).ret()
        }
    }
}
