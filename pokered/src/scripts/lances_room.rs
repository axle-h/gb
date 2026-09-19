//! `LancesRoom_Script`: the last of the Elite Four, the hallway that walks itself and the doorway
//! that bricks itself up behind the player.

use poke_core::symbols::pokered_events::{EVENT_BEAT_LANCE, EVENT_LANCES_ROOM_LOCK_DOOR};
use crate::audio::data::sounds;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_LANCESROOM_DEFAULT, SCRIPT_LANCESROOM_LANCE_END_BATTLE,
    SCRIPT_LANCESROOM_NOOP, SCRIPT_LANCESROOM_PLAYER_IS_MOVING, TEXT_LANCESROOM_LANCE};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Code, Flow, Script};

/// `LanceTriggerMovementCoords`, as (x, y). The first two stand beside Lance, the next two are the
/// doorway the room bricks up, and the last is the staircase the walk down the hallway starts on.
const TRIGGERS: [(u8, u8); 5] = [(5, 1), (6, 2), (5, 11), (6, 11), (24, 16)];
/// The `wCoordIndex` values that are not beside Lance, and the one that is the staircase, the
/// cartridge counting from one.
const NOT_BESIDE_LANCE: u8 = 3;
const STAIRCASE: u8 = 5;
/// The doorway, as the two blocks it is made of: stone once the door has locked.
const ENTRANCE_AT: [(u8, u8); 2] = [(2, 6), (3, 6)];
const ENTRANCE_OPEN: [u8; 2] = [0x31, 0x32];
const ENTRANCE_SHUT: [u8; 2] = [0x72, 0x73];
/// `WalkToLance_RLEList`.
const WALK_TO_LANCE: [(Joypad, usize); 4] =
    [(Joypad::UP, 12), (Joypad::LEFT, 12), (Joypad::DOWN, 7), (Joypad::LEFT, 6)];
const PAD_BUTTONS: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START);
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wLancesRoomCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    EndBattle,
    PlayerIsMoving,
    /// `ld [wLancesRoomCurScript], a` after the table's routine.
    StoreCurScript,
    MovedIn,
    AfterEndTrainerBattle,
    /// `LancesRoomLanceAfterBattleText`, whose `text_asm` is what marks the gauntlet won.
    LanceAfterBattle,
    LanceSpoke,
}

pub fn script(rt: &mut Script) -> Flow {
    show_or_hide_entrance_blocks(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().lances_room.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::LancesRoomTrainerHeaders);
    let entry: Code = match index {
        SCRIPT_LANCESROOM_DEFAULT => Label::DefaultScript.into(),
        SCRIPT_LANCESROOM_LANCE_END_BATTLE => Label::EndBattle.into(),
        SCRIPT_LANCESROOM_PLAYER_IS_MOVING => Label::PlayerIsMoving.into(),
        // `LancesRoomNoopScript`, which the room sits in while the rest of the floor plays.
        SCRIPT_LANCESROOM_NOOP => return Flow::Return,
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `LanceShowOrHideEntranceBlocks`. The default script re-arms `BIT_CUR_MAP_LOADED_1` itself when
/// it locks the door, so this redraws the doorway without the map being reloaded.
fn show_or_hide_entrance_blocks(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    let blocks = match rt.check_event(EVENT_LANCES_ROOM_LOCK_DOOR) {
        true => ENTRANCE_SHUT,
        false => ENTRANCE_OPEN,
    };
    for (at, block) in ENTRANCE_AT.iter().zip(blocks) {
        rt.replace_tile_block(at.0, at.1, block);
    }
}

/// `ld [wLancesRoomCurScript], a` and `ld [wCurMapScript], a`, which the room's scripts do together.
fn set_script(rt: &mut Script, index: u8) {
    rt.maps().lances_room.cur_script = index;
    rt.set_cur_map_script(index);
}

/// `LancesRoomDefaultScript`: the staircase starts the walk down the hallway, the doorway locks
/// itself the first time it is stood on, and the two squares beside Lance make him speak.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_BEAT_LANCE) {
        return Flow::Return;
    }
    let Some(index) = rt.are_player_coords_in_array(&TRIGGERS) else {
        return rt.trainer_script(SCRIPT_LANCESROOM_DEFAULT).ret();
    };
    rt.clear_joy_held();
    if index < NOT_BESIDE_LANCE {
        return rt.display_text_id(TEXT_LANCESROOM_LANCE).ret();
    }
    if index == STAIRCASE {
        return walk_to_lance(rt);
    }
    if rt.check_and_set_event(EVENT_LANCES_ROOM_LOCK_DOOR) {
        return Flow::Return;
    }
    rt.set_cur_map_loaded(1);
    rt.play_sound(sounds::SFX_GO_INSIDE);
    show_or_hide_entrance_blocks(rt);
    Flow::Return
}

/// `WalkToLance`: the hallway is walked for the player, who has no say until it runs out.
fn walk_to_lance(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
    let presses = WALK_TO_LANCE.iter().flat_map(|&(pad, count)| vec![pad; count]).collect();
    rt.simulate_joypad_presses(presses);
    set_script(rt, SCRIPT_LANCESROOM_PLAYER_IS_MOVING);
    Flow::Return
}

/// `LancesRoomPlayerIsMovingScript`: the pad is the player's again once the walk has run out.
fn player_is_moving(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::MovedIn)
}

/// `LancesRoomLanceEndBattleScript`.
fn end_battle(rt: &mut Script) -> Flow {
    rt.trainer_script(SCRIPT_LANCESROOM_LANCE_END_BATTLE).then(Label::AfterEndTrainerBattle)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    match text_id {
        TEXT_LANCESROOM_LANCE => {
            let after = Some(Label::LanceAfterBattle.into());
            Some(rt.talk_to_trainer_asm(sym::LancesRoomTrainerHeader0, None, after).ret())
        }
        _ => None,
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::EndBattle => end_battle(rt),
        Label::PlayerIsMoving => player_is_moving(rt),
        Label::StoreCurScript => {
            rt.maps().lances_room.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::MovedIn => {
            rt.joy_ignore(Joypad::empty());
            set_script(rt, SCRIPT_LANCESROOM_DEFAULT);
            Flow::Return
        }
        // `ResetLanceScript` where the player lost, and Lance's own words where they won.
        Label::AfterEndTrainerBattle => {
            if rt.lost_battle() {
                set_script(rt, SCRIPT_LANCESROOM_DEFAULT);
                return Flow::Return;
            }
            rt.display_text_id(TEXT_LANCESROOM_LANCE).ret()
        }
        Label::LanceAfterBattle => rt.print_text(text_at(sym::LancesRoomLanceAfterBattleText)).then(Label::LanceSpoke),
        Label::LanceSpoke => {
            rt.set_event(EVENT_BEAT_LANCE);
            Flow::Return
        }
    }
}
