//! `Route24_Script`: the Nugget Bridge's six trainers, and the seventh at the top who pays out and
//! then turns out to be a Rocket.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BEAT_ROUTE24_ROCKET, EVENT_GOT_NUGGET,
    EVENT_NUGGET_REWARD_AVAILABLE};
use poke_core::symbols::pokered_local_labels::Route24CooltrainerM1Text as words;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{Route24TrainerHeader0, Route24TrainerHeader1, Route24TrainerHeader2,
    Route24TrainerHeader3, Route24TrainerHeader4, Route24TrainerHeader5, Route24TrainerHeaders};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use super::{text_at, Code, Flow, Script};

const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `Route24DefaultScript.PlayerCoordsArray`: the square at the top of the bridge, where the prize
/// is handed over without being asked for.
const PRIZE_COORDS: [(u8, u8); 1] = [(10, 15)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wRoute24CurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    DefaultScript,
    AfterRocketBattle,
    PlayerMoving,
    /// `ld [wRoute24CurScript], a` after the table's routine.
    StoreCurScript,
    PrizeText,
    MovingDelay,
    AfterRocketText,
    ContestPrize,
    NuggetReceived,
    JoinRocket,
    NoRoom,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().route24.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, Route24TrainerHeaders);
    let entry: Code = match index {
        SCRIPT_ROUTE24_AFTER_ROCKET_BATTLE => Label::AfterRocketBattle.into(),
        SCRIPT_ROUTE24_PLAYER_MOVING => Label::PlayerMoving.into(),
        SCRIPT_ROUTE24_DEFAULT => Label::DefaultScript.into(),
        _ => return rt.trainer_script(index).then(Label::StoreCurScript),
    };
    Flow::Call(entry, Label::StoreCurScript.into())
}

/// `Route24DefaultScript`: whoever steps onto the top of the bridge is spoken to where they stand.
fn default_script(rt: &mut Script) -> Flow {
    if rt.check_event(EVENT_GOT_NUGGET) || rt.are_player_coords_in_array(&PRIZE_COORDS).is_none() {
        return rt.trainer_script(SCRIPT_ROUTE24_DEFAULT).ret();
    }
    rt.clear_joy_held();
    rt.display_text_id(TEXT_ROUTE24_COOLTRAINER_M1).then(Label::PrizeText)
}

/// `ld [wRoute24CurScript], a` and `ld [wCurMapScript], a` together.
fn set_script(rt: &mut Script, index: u8) {
    rt.maps().route24.cur_script = index;
    rt.set_cur_map_script(index);
}

/// `Route24SetDefaultScript`.
fn set_default_script(rt: &mut Script) -> Flow {
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_ROUTE24_DEFAULT);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_ROUTE24_COOLTRAINER_M1 => return Some(cooltrainer_m1_text(rt)),
        TEXT_ROUTE24_COOLTRAINER_M2 => Route24TrainerHeader0,
        TEXT_ROUTE24_COOLTRAINER_M3 => Route24TrainerHeader1,
        TEXT_ROUTE24_COOLTRAINER_F1 => Route24TrainerHeader2,
        TEXT_ROUTE24_YOUNGSTER1 => Route24TrainerHeader3,
        TEXT_ROUTE24_COOLTRAINER_F2 => Route24TrainerHeader4,
        TEXT_ROUTE24_YOUNGSTER2 => Route24TrainerHeader5,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `Route24CooltrainerM1Text`: the prize is a Nugget and the offer that follows it is a battle.
fn cooltrainer_m1_text(rt: &mut Script) -> Flow {
    rt.reset_event(EVENT_NUGGET_REWARD_AVAILABLE);
    if rt.check_event(EVENT_GOT_NUGGET) {
        return rt.print_text(text_at(words::YouCouldBecomeATopLeaderText)).ret();
    }
    rt.print_text(text_at(words::YouBeatOurContestText)).then(Label::ContestPrize)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::DefaultScript => default_script(rt),
        Label::PrizeText => {
            // A bag too full for the Nugget leaves the reward armed, and the script waits rather
            // than walking the player off the square.
            let armed = rt.check_event(EVENT_NUGGET_REWARD_AVAILABLE);
            rt.reset_event(EVENT_NUGGET_REWARD_AVAILABLE);
            if !armed {
                return Flow::Return;
            }
            rt.simulate_joypad_presses(vec![Joypad::DOWN]);
            set_script(rt, SCRIPT_ROUTE24_PLAYER_MOVING);
            Flow::Return
        }
        Label::PlayerMoving => {
            if rt.simulated_joypad_states_index() != 0 {
                return Flow::Return;
            }
            rt.delay3().then(Label::MovingDelay)
        }
        Label::MovingDelay => set_default_script(rt),
        Label::AfterRocketBattle => {
            if rt.lost_battle() {
                return set_default_script(rt);
            }
            rt.update_sprites();
            rt.joy_ignore(PAD_CTRL_PAD);
            rt.set_event(EVENT_BEAT_ROUTE24_ROCKET);
            rt.display_text_id(TEXT_ROUTE24_COOLTRAINER_M1).then(Label::AfterRocketText)
        }
        Label::AfterRocketText => set_default_script(rt),
        Label::StoreCurScript => {
            rt.maps().route24.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ContestPrize => {
            if !rt.give_item(ItemId::Nugget, 1) {
                return rt.print_text(text_at(words::NoRoomText)).then(Label::NoRoom);
            }
            rt.set_event(EVENT_GOT_NUGGET);
            rt.print_text(text_at(words::ReceivedNuggetText)).then(Label::NuggetReceived)
        }
        Label::NuggetReceived => rt.print_text(text_at(words::JoinTeamRocketText)).then(Label::JoinRocket),
        Label::JoinRocket => {
            rt.save_end_battle_text(words::DefeatedText);
            rt.engage_map_trainer(rt.sprite_index(), 0);
            rt.clear_joy_held();
            set_script(rt, SCRIPT_ROUTE24_AFTER_ROCKET_BATTLE);
            Flow::Return
        }
        Label::NoRoom => {
            rt.set_event(EVENT_NUGGET_REWARD_AVAILABLE);
            Flow::Return
        }
    }
}
