//! `SilphCo11F_Script`: Giovanni, who walks down to meet the player and takes every Rocket in the
//! building and in Saffron with him when he loses, and the president, who hands over the Master Ball.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_SILPH_CO_GIOVANNI, EVENT_GOT_MASTER_BALL,
    EVENT_SILPH_CO_11_UNLOCKED_DOOR};
use poke_core::symbols::pokered_local_labels::SilphCo11FSilphPresidentText as president;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SILPHCO11F_DEFAULT, SCRIPT_SILPHCO11F_GIOVANNI_AFTER_BATTLE,
    SCRIPT_SILPHCO11F_GIOVANNI_FACING, SCRIPT_SILPHCO11F_GIOVANNI_START_BATTLE, TEXT_SILPHCO11F_GIOVANNI,
    TEXT_SILPHCO11F_GIOVANNI_YOU_RUINED_OUR_PLANS, TEXT_SILPHCO11F_ROCKET1, TEXT_SILPHCO11F_ROCKET2,
    TEXT_SILPHCO11F_SILPH_PRESIDENT};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_symbols::SILPHCO11F_GIOVANNI;
use poke_core::symbols::pokered_toggles::*;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::movement::NPC_MOVEMENT_DOWN;
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_RIGHT};
use super::{text_at, Code, Flow, Script};

/// `PLAYER_DIR_UP` and `PLAYER_DIR_LEFT`.
const PLAYER_DIR_UP: u8 = 8;
const PLAYER_DIR_LEFT: u8 = 2;
const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// The block this floor's one card key door is drawn as while it is shut, and where it stands.
const CLOSED_DOOR: u8 = 0x20;
const GATES: [(u8, u8); 1] = [(3, 6)];

/// `SilphCo11FDefaultScript.PlayerCoordsArray`: the square below Giovanni's three steps and the
/// square east of it. `wCoordIndex` counts from one, so the first of these is the one the cartridge
/// compares against `1` — its comment there calls that the second entry, and is wrong.
const GIOVANNI_COORDS: [(u8, u8); 2] = [(6, 13), (7, 12)];
const GIOVANNI_WALKS_DOWN: [u8; 4] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, NPC_MOVEMENT_DOWN, END];

/// `SilphCo11FTeamRocketLeavesScript.HideToggleableObjectIDs`: every Rocket still standing in the
/// building, and the nine in Saffron who bar its streets.
const ROCKETS_LEAVE: [u16; 40] = [TOGGLE_SAFFRON_CITY_1, TOGGLE_SAFFRON_CITY_2, TOGGLE_SAFFRON_CITY_3,
    TOGGLE_SAFFRON_CITY_4, TOGGLE_SAFFRON_CITY_5, TOGGLE_SAFFRON_CITY_6, TOGGLE_SAFFRON_CITY_7,
    TOGGLE_SAFFRON_CITY_E, TOGGLE_SAFFRON_CITY_F, TOGGLE_SILPH_CO_2F_2, TOGGLE_SILPH_CO_2F_3,
    TOGGLE_SILPH_CO_2F_4, TOGGLE_SILPH_CO_2F_5, TOGGLE_SILPH_CO_3F_1, TOGGLE_SILPH_CO_3F_2,
    TOGGLE_SILPH_CO_4F_1, TOGGLE_SILPH_CO_4F_2, TOGGLE_SILPH_CO_4F_3, TOGGLE_SILPH_CO_5F_1,
    TOGGLE_SILPH_CO_5F_2, TOGGLE_SILPH_CO_5F_3, TOGGLE_SILPH_CO_5F_4, TOGGLE_SILPH_CO_6F_1,
    TOGGLE_SILPH_CO_6F_2, TOGGLE_SILPH_CO_6F_3, TOGGLE_SILPH_CO_7F_1, TOGGLE_SILPH_CO_7F_2,
    TOGGLE_SILPH_CO_7F_3, TOGGLE_SILPH_CO_7F_4, TOGGLE_SILPH_CO_8F_1, TOGGLE_SILPH_CO_8F_2,
    TOGGLE_SILPH_CO_8F_3, TOGGLE_SILPH_CO_9F_1, TOGGLE_SILPH_CO_9F_2, TOGGLE_SILPH_CO_9F_3,
    TOGGLE_SILPH_CO_10F_1, TOGGLE_SILPH_CO_10F_2, TOGGLE_SILPH_CO_11F_1, TOGGLE_SILPH_CO_11F_2,
    TOGGLE_SILPH_CO_11F_3];
/// `...ShowToggleableObjectIDs`: the townspeople who take their place in Saffron.
const TOWNSPEOPLE_RETURN: [u16; 6] = [TOGGLE_SAFFRON_CITY_8, TOGGLE_SAFFRON_CITY_9, TOGGLE_SAFFRON_CITY_A,
    TOGGLE_SAFFRON_CITY_B, TOGGLE_SAFFRON_CITY_C, TOGGLE_SAFFRON_CITY_D];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo11FCurScript`.
    pub cur_script: u8,
    /// `wSavedCoordIndex`: which of the two squares the player met Giovanni on, which is what
    /// decides the way the two of them face for the battle and for the words after it.
    pub saved_coord_index: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    GiovanniFacing,
    GiovanniStartBattle,
    GiovanniAfterBattle,
    /// `ld [wSilphCo11FCurScript], a` after the table's routine.
    StoreCurScript,
    GiovanniChallenged,
    GiovanniTurned,
    GiovanniTurnedDelay,
    BeatenGiovanniTurned,
    RuinedOurPlans,
    FadedOut,
    RocketsGone,
    BuildingCleared,
    PresidentOffered,
}

pub fn script(rt: &mut Script) -> Flow {
    gate_callback(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_11f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo11TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_SILPHCO11F_DEFAULT => Label::DefaultScript.into(),
        SCRIPT_SILPHCO11F_GIOVANNI_FACING => Label::GiovanniFacing.into(),
        SCRIPT_SILPHCO11F_GIOVANNI_START_BATTLE => Label::GiovanniStartBattle.into(),
        SCRIPT_SILPHCO11F_GIOVANNI_AFTER_BATTLE => Label::GiovanniAfterBattle.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `SilphCo11FGateCallbackScript`: one gate, and one event that remembers it.
fn gate_callback(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    if super::silph_co_2f::unlocked_door(rt, &GATES) != 0 {
        rt.set_event(EVENT_SILPH_CO_11_UNLOCKED_DOOR);
    }
    if !rt.check_event(EVENT_SILPH_CO_11_UNLOCKED_DOOR) {
        rt.replace_tile_block(GATES[0].0, GATES[0].1, CLOSED_DOOR);
    }
}

/// `SilphCo11FSetCurScript`.
fn set_script(rt: &mut Script, index: u8) {
    rt.maps().silph_co_11f.cur_script = index;
    rt.set_cur_map_script(index);
}

/// `SilphCo11FResetCurScript`.
fn reset_cur_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_SILPHCO11F_DEFAULT);
    Flow::Return
}

/// `SilphCo11FDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
        return Flow::Return;
    }
    let Some(index) = rt.are_player_coords_in_array(&GIOVANNI_COORDS) else {
        return rt.trainer_script(SCRIPT_SILPHCO11F_DEFAULT).ret();
    };
    rt.maps().silph_co_11f.saved_coord_index = index;
    rt.clear_joy_held();
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.display_text_id(TEXT_SILPHCO11F_GIOVANNI).then(Label::GiovanniChallenged)
}

/// `SilphCo11FSetPlayerAndSpriteFacingDirectionScript`: they face each other across whichever of
/// the two squares the player is standing on.
fn face_each_other(rt: &mut Script, next: Label) -> Flow {
    let (direction, facing) = match rt.maps().silph_co_11f.saved_coord_index {
        1 => (PLAYER_DIR_UP, SPRITE_FACING_DOWN),
        _ => (PLAYER_DIR_LEFT, SPRITE_FACING_RIGHT),
    };
    rt.set_player_moving_direction(direction);
    rt.set_sprite_facing_direction_and_delay(SILPHCO11F_GIOVANNI, facing).then(next)
}

/// `SilphCo11FTeamRocketLeavesScript`.
fn team_rocket_leaves(rt: &mut Script) {
    for toggle in ROCKETS_LEAVE {
        rt.hide_object(toggle);
    }
    for toggle in TOWNSPEOPLE_RETURN {
        rt.show_object(toggle);
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SILPHCO11F_SILPH_PRESIDENT => {
            return Some(match rt.check_event(EVENT_GOT_MASTER_BALL) {
                true => rt.print_text(text_at(president::MasterBallDescriptionText)).ret(),
                false => rt.print_text(text_at(president::Text)).then(Label::PresidentOffered),
            });
        }
        TEXT_SILPHCO11F_ROCKET1 => sym::SilphCo11TrainerHeader0,
        TEXT_SILPHCO11F_ROCKET2 => sym::SilphCo11TrainerHeader1,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::StoreCurScript => {
            rt.maps().silph_co_11f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        // He walks three squares down to stand in front of whoever has come for him.
        Label::GiovanniChallenged => {
            rt.set_sprite_movement_bytes_to_ff(SILPHCO11F_GIOVANNI);
            rt.move_sprite(SILPHCO11F_GIOVANNI, &GIOVANNI_WALKS_DOWN);
            set_script(rt, SCRIPT_SILPHCO11F_GIOVANNI_FACING);
            Flow::Return
        }
        // `SilphCo11FGiovanniBattleFacingScript`.
        Label::GiovanniFacing => {
            if rt.npc_moving() {
                return Flow::Return;
            }
            rt.set_sprite_movement_bytes_to_ff(SILPHCO11F_GIOVANNI);
            face_each_other(rt, Label::GiovanniTurned)
        }
        Label::GiovanniTurned => rt.delay3().then(Label::GiovanniTurnedDelay),
        Label::GiovanniTurnedDelay => {
            set_script(rt, SCRIPT_SILPHCO11F_GIOVANNI_START_BATTLE);
            Flow::Return
        }
        // `SilphCo11FGiovanniStartBattleScript`.
        Label::GiovanniStartBattle => {
            rt.save_end_battle_text(sym::SilphCo11FGiovanniILostAgainText);
            rt.engage_map_trainer(SILPHCO11F_GIOVANNI, 0);
            rt.joy_ignore(Joypad::empty());
            set_script(rt, SCRIPT_SILPHCO11F_GIOVANNI_AFTER_BATTLE);
            Flow::Return
        }
        // `SilphCo11FGiovanniAfterBattleScript`.
        Label::GiovanniAfterBattle => match rt.lost_battle() {
            true => reset_cur_script(rt),
            false => face_each_other(rt, Label::BeatenGiovanniTurned),
        },
        Label::BeatenGiovanniTurned => {
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.display_text_id(TEXT_SILPHCO11F_GIOVANNI_YOU_RUINED_OUR_PLANS).then(Label::RuinedOurPlans)
        }
        Label::RuinedOurPlans => rt.gb_fade_out_to_black().then(Label::FadedOut),
        Label::FadedOut => {
            team_rocket_leaves(rt);
            rt.update_sprites();
            rt.delay3().then(Label::RocketsGone)
        }
        Label::RocketsGone => rt.gb_fade_in_from_black().then(Label::BuildingCleared),
        Label::BuildingCleared => {
            rt.set_event(EVENT_BEAT_SILPH_CO_GIOVANNI);
            reset_cur_script(rt)
        }
        Label::PresidentOffered => {
            if !rt.give_item(ItemId::MasterBall, 1) {
                return rt.print_text(text_at(president::NoRoomText)).ret();
            }
            rt.set_event(EVENT_GOT_MASTER_BALL);
            rt.print_text(text_at(president::ReceivedMasterBallText)).ret()
        }
    }
}
