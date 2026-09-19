//! `RocketHideoutB4F_Script`: the door its two guards unlock, and Giovanni, who leaves the Silph
//! Scope behind him.

use poke_core::symbols::pokered_events::{EVENT_BEAT_ROCKET_HIDEOUT_4_TRAINER_0, EVENT_BEAT_ROCKET_HIDEOUT_4_TRAINER_1,
    EVENT_BEAT_ROCKET_HIDEOUT_GIOVANNI, EVENT_ROCKET_DROPPED_LIFT_KEY, EVENT_ROCKET_HIDEOUT_4_DOOR_UNLOCKED};
use poke_core::symbols::pokered_local_labels::RocketHideoutB4FGiovanniText as giovanni;
use poke_core::symbols::pokered_local_labels::RocketHideoutB4FRocket3AfterBattleText as rocket3;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI, SCRIPT_ROCKETHIDEOUTB4F_DEFAULT,
    TEXT_ROCKETHIDEOUTB4F_GIOVANNI, TEXT_ROCKETHIDEOUTB4F_GIOVANNI_HOPE_WE_MEET_AGAIN, TEXT_ROCKETHIDEOUTB4F_ROCKET1,
    TEXT_ROCKETHIDEOUTB4F_ROCKET2, TEXT_ROCKETHIDEOUTB4F_ROCKET3};
use poke_core::symbols::pokered_symbols as sym;
use poke_core::symbols::pokered_toggles::{TOGGLE_ROCKET_HIDEOUT_B4F_GIOVANNI, TOGGLE_ROCKET_HIDEOUT_B4F_ITEM_4,
    TOGGLE_ROCKET_HIDEOUT_B4F_ITEM_5};
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::input::Joypad;
use super::{text_at, Flow, Script};

const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// The door the two Rockets in front of Giovanni hold shut, and the floor it becomes.
const DOOR_BLOCK: u8 = 0x2D;
const FLOOR_BLOCK: u8 = 0x0E;
const DOOR_AT: (u8, u8) = (12, 5);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRocketHideoutB4FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wRocketHideoutB4FCurScript], a` after the table's routine.
    StoreCurScript,
    /// `RocketHideoutB4FBeatGiovanniScript`, which takes him away behind a fade.
    GiovanniBeaten,
    GiovanniFadedOut,
    GiovanniGone,
    /// `RocketHideoutB4FGiovanniText`'s battle.
    PreBattle,
    /// `RocketHideoutB4FRocket3AfterBattleText`, a `text_asm` that drops the Lift Key the first time.
    Rocket3AfterBattle,
    Rocket3DroppedLiftKey,
}

pub fn script(rt: &mut Script) -> Flow {
    door_callback(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().rocket_hideout_b4f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::RocketHideout4TrainerHeaders);
    if index == SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI {
        return beat_giovanni(rt);
    }
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `RocketHideoutB4FDoorCallbackScript`: the door opens once both of its guards are beaten.
fn door_callback(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    let block = if rt.check_event(EVENT_ROCKET_HIDEOUT_4_DOOR_UNLOCKED) {
        FLOOR_BLOCK
    } else if rt.check_event(EVENT_BEAT_ROCKET_HIDEOUT_4_TRAINER_0)
        && rt.check_event(EVENT_BEAT_ROCKET_HIDEOUT_4_TRAINER_1)
    {
        rt.play_sound(sounds::SFX_GO_INSIDE);
        rt.set_event(EVENT_ROCKET_HIDEOUT_4_DOOR_UNLOCKED);
        FLOOR_BLOCK
    } else {
        DOOR_BLOCK
    };
    rt.replace_tile_block(DOOR_AT.0, DOOR_AT.1, block);
}

/// `RocketHideoutB4FSetDefaultScript`.
fn set_default_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().rocket_hideout_b4f.cur_script = SCRIPT_ROCKETHIDEOUTB4F_DEFAULT;
    rt.set_cur_map_script(SCRIPT_ROCKETHIDEOUTB4F_DEFAULT);
    Flow::Return
}

/// `RocketHideoutB4FBeatGiovanniScript`.
fn beat_giovanni(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return set_default_script(rt);
    }
    rt.update_sprites();
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_event(EVENT_BEAT_ROCKET_HIDEOUT_GIOVANNI);
    rt.display_text_id(TEXT_ROCKETHIDEOUTB4F_GIOVANNI_HOPE_WE_MEET_AGAIN).then(Label::GiovanniBeaten)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id == TEXT_ROCKETHIDEOUTB4F_GIOVANNI {
        return Some(match rt.check_event(EVENT_BEAT_ROCKET_HIDEOUT_GIOVANNI) {
            true => rt.print_text(text_at(sym::RocketHideoutB4FGiovanniHopeWeMeetAgainText)).ret(),
            false => rt.print_text(text_at(giovanni::ImpressedYouGotHereText)).then(Label::PreBattle),
        });
    }
    let header = match text_id {
        TEXT_ROCKETHIDEOUTB4F_ROCKET1 => sym::RocketHideout4TrainerHeader0,
        TEXT_ROCKETHIDEOUTB4F_ROCKET2 => sym::RocketHideout4TrainerHeader1,
        TEXT_ROCKETHIDEOUTB4F_ROCKET3 => {
            let after = Some(Label::Rocket3AfterBattle.into());
            return Some(rt.talk_to_trainer_asm(sym::RocketHideout4TrainerHeader2, None, after).ret());
        }
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().rocket_hideout_b4f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::GiovanniBeaten => rt.gb_fade_out_to_black().then(Label::GiovanniFadedOut),
        // The Silph Scope is on the square he was standing on, so it appears as he disappears.
        Label::GiovanniFadedOut => {
            rt.hide_object(TOGGLE_ROCKET_HIDEOUT_B4F_GIOVANNI);
            rt.show_object(TOGGLE_ROCKET_HIDEOUT_B4F_ITEM_4);
            rt.update_sprites();
            rt.gb_fade_in_from_black().then(Label::GiovanniGone)
        }
        Label::GiovanniGone => {
            // The door callback runs again, so the floor he was guarding is drawn without him.
            rt.set_cur_map_loaded(1);
            set_default_script(rt)
        }
        Label::Rocket3AfterBattle => rt.print_text(text_at(rocket3::Text)).then(Label::Rocket3DroppedLiftKey),
        Label::Rocket3DroppedLiftKey => {
            if !rt.check_and_set_event(EVENT_ROCKET_DROPPED_LIFT_KEY) {
                rt.show_object(TOGGLE_ROCKET_HIDEOUT_B4F_ITEM_5);
            }
            Flow::Return
        }
        Label::PreBattle => {
            rt.save_end_battle_text(giovanni::WhatCannotBeText);
            rt.engage_map_trainer(rt.sprite_index(), 0);
            rt.clear_joy_held();
            rt.maps().rocket_hideout_b4f.cur_script = SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI;
            rt.set_cur_map_script(SCRIPT_ROCKETHIDEOUTB4F_BEAT_GIOVANNI);
            Flow::Return
        }
    }
}
