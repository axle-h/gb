//! `SafariZoneGate_Script`: the turnstile into the Safari Zone. It walks the player to the counter,
//! takes the ¥500, hands out the balls and the steps, and walks them back out again at the end.

use poke_core::symbols::pokered_events::{EVENT_IN_SAFARI_ZONE, EVENT_SAFARI_GAME_OVER};
use poke_core::symbols::pokered_local_labels::{SafariZoneGateSafariZoneWorker1LeavingEarlyText as early,
    SafariZoneGateSafariZoneWorker1WouldYouLikeToJoinText as join,
    SafariZoneGateSafariZoneWorker2Text as worker2};
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SAFARIZONEGATE_DEFAULT, SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI,
    SCRIPT_SAFARIZONEGATE_PLAYER_MOVING, SCRIPT_SAFARIZONEGATE_PLAYER_MOVING_DOWN,
    SCRIPT_SAFARIZONEGATE_PLAYER_MOVING_RIGHT, SCRIPT_SAFARIZONEGATE_SET_SCRIPT_AFTER_MOVE,
    SCRIPT_SAFARIZONEGATE_WOULD_YOU_LIKE_TO_JOIN, TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_1,
    TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_GOOD_HAUL_COME_AGAIN,
    TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_LEAVING_EARLY,
    TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_WOULD_YOU_LIKE_TO_JOIN, TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER2};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_RIGHT, SPRITE_FACING_UP};
use super::{text_at, Flow, Script};

/// `PLAYER_DIR_DOWN`.
const PLAYER_DIR_DOWN: u8 = 4;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
const PAD_BUTTONS: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START);

/// `.PlayerNextToSafariZoneWorker1CoordsArray`: the square in front of the counter, and the one to
/// its left. `wCoordIndex` counts from one, so the cartridge's `cp 1` is the first of these, the one
/// a step short of the counter; its comment there calls that the second entry, and is wrong.
const NEXT_TO_WORKER: [(u8, u8); 2] = [(3, 2), (4, 2)];

/// The fee, as BCD, and what it buys.
const FEE: [u8; 3] = [0x00, 0x05, 0x00];
const SAFARI_BALLS: u8 = 30;
/// `wSafariSteps` starts at 502, not the 500 the sign outside promises: the walk in off the counter
/// spends the first two.
const SAFARI_STEPS: u16 = 502;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSafariZoneGateCurScript`.
    pub cur_script: u8,
    /// `wNextSafariZoneGateScript`: what runs once the walk the worker's text started is over.
    pub next_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `SafariZoneGateDefaultScript` after its greeting, with `wCoordIndex` as it was.
    WorkerGreeted(u8),
    JoinAsked,
    JoinMoneyBox,
    JoinAnswered,
    Paid,
    CantPayWalkDown,
    GoodHaul,
    LeavingEarlyAsked,
    LeavingEarlyAnswered,
    LeaveNow,
    StayOn,
    AfterMoveDelayed,
    Worker2Asked,
    Worker2Answered,
}

/// `SafariZoneEntranceAutoWalk`: `count` presses of one direction, which the loop makes for the
/// player before the map's script runs again.
fn auto_walk(rt: &mut Script, pad: Joypad, count: usize) {
    rt.simulate_joypad_presses(vec![pad; count]);
}

fn set_script(rt: &mut Script, index: u8) {
    rt.maps().safari_zone_gate.cur_script = index;
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().safari_zone_gate.cur_script {
        SCRIPT_SAFARIZONEGATE_PLAYER_MOVING_RIGHT => player_moving_right(rt),
        SCRIPT_SAFARIZONEGATE_WOULD_YOU_LIKE_TO_JOIN => would_you_like_to_join(rt),
        SCRIPT_SAFARIZONEGATE_PLAYER_MOVING => player_moving_up(rt),
        SCRIPT_SAFARIZONEGATE_PLAYER_MOVING_DOWN => player_moving_down(rt),
        SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI => leaving_safari(rt),
        SCRIPT_SAFARIZONEGATE_SET_SCRIPT_AFTER_MOVE => set_script_after_move(rt),
        _ => default_script(rt),
    }
}

/// `SafariZoneGateDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    let Some(index) = rt.are_player_coords_in_array(&NEXT_TO_WORKER) else {
        return Flow::Return;
    };
    rt.display_text_id(TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_1).then(Label::WorkerGreeted(index))
}

/// `SafariZoneGatePlayerMovingRightScript`, which falls through into the offer.
fn player_moving_right(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    would_you_like_to_join(rt)
}

/// `SafariZoneGateWouldYouLikeToJoinScript`.
fn would_you_like_to_join(rt: &mut Script) -> Flow {
    rt.clear_joy_held();
    rt.joy_ignore(Joypad::empty());
    rt.update_sprites();
    rt.display_text_id(TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_WOULD_YOU_LIKE_TO_JOIN).then(Label::JoinAsked)
}

/// `SafariZoneGatePlayerMovingUpScript`: the three steps in are over, so the game is on.
fn player_moving_up(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI);
    Flow::Return
}

/// `SafariZoneGateLeavingSafariScript`, which the game-over warp arms as well as the north door.
fn leaving_safari(rt: &mut Script) -> Flow {
    rt.set_player_moving_direction(PLAYER_DIR_DOWN);
    if !rt.check_and_reset_event(EVENT_SAFARI_GAME_OVER) {
        return rt.display_text_id(TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_LEAVING_EARLY).ret();
    }
    rt.reset_event(EVENT_IN_SAFARI_ZONE);
    rt.update_sprites();
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.display_text_id(TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_GOOD_HAUL_COME_AGAIN).then(Label::GoodHaul)
}

/// `SafariZoneGatePlayerMovingDownScript`.
fn player_moving_down(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    set_script(rt, SCRIPT_SAFARIZONEGATE_DEFAULT);
    Flow::Return
}

/// `SafariZoneGateSetScriptAfterMoveScript`.
fn set_script_after_move(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay3().then(Label::AfterMoveDelayed)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_WOULD_YOU_LIKE_TO_JOIN => {
            rt.print_text(text_at(sym::SafariZoneGateSafariZoneWorker1WouldYouLikeToJoinText))
                .then(Label::JoinMoneyBox)
        }
        TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER1_LEAVING_EARLY => {
            rt.print_text(text_at(sym::SafariZoneGateSafariZoneWorker1LeavingEarlyText))
                .then(Label::LeavingEarlyAsked)
        }
        TEXT_SAFARIZONEGATE_SAFARI_ZONE_WORKER2 => {
            rt.print_text(text_at(worker2::FirstTimeHereText)).then(Label::Worker2Asked)
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::WorkerGreeted(index) => {
            rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
            rt.clear_joy_held();
            rt.set_player_facing(SPRITE_FACING_RIGHT);
            if index != 1 {
                set_script(rt, SCRIPT_SAFARIZONEGATE_WOULD_YOU_LIKE_TO_JOIN);
                return Flow::Return;
            }
            auto_walk(rt, Joypad::RIGHT, 1);
            rt.joy_ignore(PAD_CTRL_PAD);
            set_script(rt, SCRIPT_SAFARIZONEGATE_PLAYER_MOVING_RIGHT);
            Flow::Return
        }
        Label::JoinAsked => {
            rt.joy_ignore(PAD_BUTTONS.union(PAD_CTRL_PAD));
            Flow::Return
        }
        Label::JoinMoneyBox => {
            rt.money_box();
            rt.yes_no_choice().then(Label::JoinAnswered)
        }
        Label::JoinAnswered => {
            if !rt.chose_yes() {
                return rt.print_text(text_at(join::PleaseComeAgainText)).then(Label::CantPayWalkDown);
            }
            if !rt.has_enough_money(FEE) {
                return rt.print_text(text_at(join::NotEnoughMoneyText)).then(Label::CantPayWalkDown);
            }
            rt.subtract_money(FEE);
            rt.money_box();
            rt.print_text(text_at(join::MakePaymentText)).then(Label::Paid)
        }
        Label::Paid => {
            rt.set_safari_balls(SAFARI_BALLS);
            rt.set_safari_steps(SAFARI_STEPS);
            auto_walk(rt, Joypad::UP, 3);
            rt.set_event(EVENT_IN_SAFARI_ZONE);
            rt.reset_event(EVENT_SAFARI_GAME_OVER);
            set_script(rt, SCRIPT_SAFARIZONEGATE_PLAYER_MOVING);
            Flow::Return
        }
        Label::CantPayWalkDown => {
            auto_walk(rt, Joypad::DOWN, 1);
            set_script(rt, SCRIPT_SAFARIZONEGATE_PLAYER_MOVING_DOWN);
            Flow::Return
        }
        // The balls go back whether or not any were thrown; the steps are not touched, since the
        // next game overwrites them.
        Label::GoodHaul => {
            rt.set_safari_balls(0);
            auto_walk(rt, Joypad::DOWN, 3);
            set_script(rt, SCRIPT_SAFARIZONEGATE_PLAYER_MOVING_DOWN);
            Flow::Return
        }
        Label::LeavingEarlyAsked => rt.yes_no_choice().then(Label::LeavingEarlyAnswered),
        Label::LeavingEarlyAnswered => match rt.chose_yes() {
            true => rt.print_text(text_at(early::ReturnSafariBallsText)).then(Label::LeaveNow),
            false => rt.print_text(text_at(early::GoodLuckText)).then(Label::StayOn),
        },
        Label::LeaveNow => {
            rt.set_player_facing(SPRITE_FACING_DOWN);
            auto_walk(rt, Joypad::DOWN, 3);
            // `ResetEvents` takes a list, not a range.
            rt.reset_event(EVENT_SAFARI_GAME_OVER);
            rt.reset_event(EVENT_IN_SAFARI_ZONE);
            after_move(rt, SCRIPT_SAFARIZONEGATE_DEFAULT)
        }
        Label::StayOn => {
            rt.set_player_facing(SPRITE_FACING_UP);
            auto_walk(rt, Joypad::UP, 1);
            after_move(rt, SCRIPT_SAFARIZONEGATE_LEAVING_SAFARI)
        }
        Label::AfterMoveDelayed => {
            let next = rt.maps().safari_zone_gate.next_script;
            set_script(rt, next);
            Flow::Return
        }
        Label::Worker2Asked => rt.yes_no_choice().then(Label::Worker2Answered),
        Label::Worker2Answered => {
            let said = match rt.chose_yes() {
                true => worker2::SafariZoneExplanationText,
                false => worker2::YoureARegularHereText,
            };
            rt.print_text(text_at(said)).ret()
        }
    }
}

fn after_move(rt: &mut Script, next: u8) -> Flow {
    rt.maps().safari_zone_gate.next_script = next;
    set_script(rt, SCRIPT_SAFARIZONEGATE_SET_SCRIPT_AFTER_MOVE);
    Flow::Return
}
