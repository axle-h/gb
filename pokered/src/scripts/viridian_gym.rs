//! `ViridianGym_Script`: the arrow tiles that slide the player around the gym, its eight trainers,
//! and Giovanni, whose Earth Badge opens Route 22 for the second rival battle.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_2ND_ROUTE22_RIVAL_BATTLE, EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI,
    EVENT_BEAT_VIRIDIAN_GYM_TRAINER_0, EVENT_BEAT_VIRIDIAN_GYM_TRAINER_7, EVENT_GOT_TM27,
    EVENT_ROUTE22_RIVAL_WANTS_BATTLE};
use poke_core::symbols::pokered_local_labels::ViridianGymGiovanniText;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_VIRIDIANGYM_DEFAULT, SCRIPT_VIRIDIANGYM_GIOVANNI_POST_BATTLE,
    SCRIPT_VIRIDIANGYM_PLAYER_SPINNING, TEXT_VIRIDIANGYM_COOLTRAINER_M1, TEXT_VIRIDIANGYM_COOLTRAINER_M2,
    TEXT_VIRIDIANGYM_COOLTRAINER_M3, TEXT_VIRIDIANGYM_GIOVANNI, TEXT_VIRIDIANGYM_GIOVANNI_EARTH_BADGE_INFO,
    TEXT_VIRIDIANGYM_GIOVANNI_RECEIVED_TM27, TEXT_VIRIDIANGYM_GIOVANNI_TM27_NO_ROOM, TEXT_VIRIDIANGYM_GYM_GUIDE,
    TEXT_VIRIDIANGYM_HIKER1,
    TEXT_VIRIDIANGYM_HIKER2, TEXT_VIRIDIANGYM_HIKER3, TEXT_VIRIDIANGYM_ROCKER1, TEXT_VIRIDIANGYM_ROCKER2};
use poke_core::symbols::pokered_symbols::{ViridianGymArrowTilePlayerMovement, ViridianGymGuidePostBattleText,
    ViridianGymGuidePreBattleText, ViridianGymTrainerHeader0,
    ViridianGymTrainerHeader1, ViridianGymTrainerHeader2, ViridianGymTrainerHeader3, ViridianGymTrainerHeader4,
    ViridianGymTrainerHeader5, ViridianGymTrainerHeader6, ViridianGymTrainerHeader7, ViridianGymTrainerHeaders};
use poke_core::symbols::pokered_toggles::{TOGGLE_ROUTE_22_RIVAL_2, TOGGLE_VIRIDIAN_GYM_GIOVANNI};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::spinners::{arrow_tile_default, player_spinning};
use super::{text_at, Flow, Script};

/// `BIT_EARTHBADGE`.
const BIT_EARTHBADGE: u8 = 7;
/// `wGymLeaderNo` for Giovanni, which keeps the gym leader's own music.
const GIOVANNI: u8 = 8;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wViridianGymCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wViridianGymCurScript], a` after the table's routine.
    StoreCurScript,
    /// `ViridianGymReceiveTM27`, which both the post-battle script and Giovanni's own text run.
    ReceiveTm27,
    BadgeInfo,
    ReceivedTm27,
    GymVictory,
    /// The `DisableWaitingAfterTextDisplay` his text does once the TM27 script has returned to it.
    TextDone,
    /// `ViridianGymGiovanniText`'s two other branches.
    PreBattle,
    PostBattleAdvice,
    GiovanniHidden,
    GiovanniGone,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().viridian_gym.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, ViridianGymTrainerHeaders);
    let then = match index {
        SCRIPT_VIRIDIANGYM_DEFAULT => {
            arrow_tile_default(rt, ViridianGymArrowTilePlayerMovement, SCRIPT_VIRIDIANGYM_PLAYER_SPINNING)
        }
        SCRIPT_VIRIDIANGYM_PLAYER_SPINNING => {
            player_spinning(rt, SCRIPT_VIRIDIANGYM_DEFAULT);
            None
        }
        SCRIPT_VIRIDIANGYM_GIOVANNI_POST_BATTLE => return giovanni_post_battle(rt),
        _ => Some(rt.trainer_script(index)),
    };
    match then {
        Some(then) => then.then(Label::StoreCurScript),
        None => resume(rt, Label::StoreCurScript),
    }
}

/// `ViridianGymGiovanniPostBattle`: the badge is handed over once the battle is won.
fn giovanni_post_battle(rt: &mut Script) -> Flow {
    if rt.lost_battle() {
        return reset_scripts(rt);
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    receive_tm27(rt)
}

/// `ViridianGymReceiveTM27`.
fn receive_tm27(rt: &mut Script) -> Flow {
    rt.display_text_id(TEXT_VIRIDIANGYM_GIOVANNI_EARTH_BADGE_INFO).then(Label::BadgeInfo)
}

/// `ViridianGymResetScripts`.
fn reset_scripts(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    rt.maps().viridian_gym.cur_script = SCRIPT_VIRIDIANGYM_DEFAULT;
    rt.set_cur_map_script(SCRIPT_VIRIDIANGYM_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id == TEXT_VIRIDIANGYM_GIOVANNI {
        return Some(giovanni_text(rt));
    }
    // `ViridianGymGymGuideText`.
    if text_id == TEXT_VIRIDIANGYM_GYM_GUIDE {
        let words = match rt.check_event(EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI) {
            true => ViridianGymGuidePostBattleText,
            false => ViridianGymGuidePreBattleText,
        };
        return Some(rt.print_text(text_at(words)).ret());
    }
    let header = match text_id {
        TEXT_VIRIDIANGYM_COOLTRAINER_M1 => ViridianGymTrainerHeader0,
        TEXT_VIRIDIANGYM_HIKER1 => ViridianGymTrainerHeader1,
        TEXT_VIRIDIANGYM_ROCKER1 => ViridianGymTrainerHeader2,
        TEXT_VIRIDIANGYM_HIKER2 => ViridianGymTrainerHeader3,
        TEXT_VIRIDIANGYM_COOLTRAINER_M2 => ViridianGymTrainerHeader4,
        TEXT_VIRIDIANGYM_HIKER3 => ViridianGymTrainerHeader5,
        TEXT_VIRIDIANGYM_ROCKER2 => ViridianGymTrainerHeader6,
        TEXT_VIRIDIANGYM_COOLTRAINER_M3 => ViridianGymTrainerHeader7,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `ViridianGymGiovanniText`.
fn giovanni_text(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI) {
        return rt.print_text(text_at(ViridianGymGiovanniText::PreBattleText)).then(Label::PreBattle);
    }
    if !rt.check_event(EVENT_GOT_TM27) {
        return Flow::Call(Label::ReceiveTm27.into(), Label::TextDone.into());
    }
    rt.set_do_not_wait_for_button_press(true);
    rt.print_text(text_at(ViridianGymGiovanniText::PostBattleAdviceText)).then(Label::PostBattleAdvice)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().viridian_gym.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ReceiveTm27 => receive_tm27(rt),
        Label::TextDone => {
            rt.set_do_not_wait_for_button_press(true);
            Flow::Return
        }
        Label::BadgeInfo => {
            rt.set_event(EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI);
            match rt.give_item(ItemId::Tm27Fissure, 1) {
                true => rt.display_text_id(TEXT_VIRIDIANGYM_GIOVANNI_RECEIVED_TM27).then(Label::ReceivedTm27),
                false => rt.display_text_id(TEXT_VIRIDIANGYM_GIOVANNI_TM27_NO_ROOM).then(Label::GymVictory),
            }
        }
        Label::ReceivedTm27 => {
            rt.set_event(EVENT_GOT_TM27);
            resume(rt, Label::GymVictory)
        }
        // `.gym_victory`: the gym's trainers are all marked beaten, so nobody stops the way out.
        Label::GymVictory => {
            rt.set_badge(BIT_EARTHBADGE);
            for event in EVENT_BEAT_VIRIDIAN_GYM_TRAINER_0..=EVENT_BEAT_VIRIDIAN_GYM_TRAINER_7 {
                rt.set_event(event);
            }
            rt.show_object(TOGGLE_ROUTE_22_RIVAL_2);
            rt.set_event(EVENT_2ND_ROUTE22_RIVAL_BATTLE);
            rt.set_event(EVENT_ROUTE22_RIVAL_WANTS_BATTLE);
            reset_scripts(rt)
        }
        Label::PreBattle => {
            rt.save_end_battle_text(ViridianGymGiovanniText::ReceivedEarthBadgeText);
            rt.engage_map_trainer(rt.sprite_index(), GIOVANNI);
            rt.maps().viridian_gym.cur_script = SCRIPT_VIRIDIANGYM_GIOVANNI_POST_BATTLE;
            Flow::Return
        }
        // He goes behind a fade, so the square he was standing on is empty when it lifts.
        Label::PostBattleAdvice => rt.gb_fade_out_to_black().then(Label::GiovanniHidden),
        Label::GiovanniHidden => {
            rt.hide_object(TOGGLE_VIRIDIAN_GYM_GIOVANNI);
            rt.update_sprites();
            rt.delay3().then(Label::GiovanniGone)
        }
        Label::GiovanniGone => rt.gb_fade_in_from_black().ret(),
    }
}
