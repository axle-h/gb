//! `VermilionCity_Script`: the sailor who will not let anyone onto the dock without a ticket, the
//! Machop being made to stomp a yard flat, and the walk back up the pier once the ship has gone.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_SS_ANNE_LEFT, EVENT_WALKED_PAST_GUARD_AFTER_SS_ANNE_LEFT};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_VERMILIONCITY_DEFAULT,
    SCRIPT_VERMILIONCITY_PLAYER_ALLOWED_TO_PASS, SCRIPT_VERMILIONCITY_PLAYER_EXIT_SHIP,
    SCRIPT_VERMILIONCITY_PLAYER_MOVING_UP1, SCRIPT_VERMILIONCITY_PLAYER_MOVING_UP2,
    TEXT_VERMILIONCITY_GAMBLER1, TEXT_VERMILIONCITY_MACHOP, TEXT_VERMILIONCITY_SAILOR1};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_RIGHT};
use super::{text_at, Flow, Script};

const PAD_BUTTONS_AND_CTRL_PAD: Joypad = Joypad::A.union(Joypad::B).union(Joypad::SELECT).union(Joypad::START)
    .union(Joypad::UP).union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `SSAnneTicketCheckCoords`: the square in front of the sailor, which is the only way down the pier.
const TICKET_CHECK_COORDS: [(u8, u8); 1] = [(18, 30)];
/// `.inFrontOfOrBehindGuardCoords`: talked to from either of these he asks for the ticket, and from
/// anywhere else he only says hello.
const GUARD_COORDS: [(u8, u8); 2] = [(19, 29), (19, 31)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wVermilionCityCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// After the sailor's own words, where the ticket decides whether the player is walked back.
    TicketCheck,
    MovingUp1Delay,
    SailorTicket,
    MachopCry,
    MachopStomp,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    if rt.check_and_reset_cur_map_loaded(2) {
        left_ss_anne_callback(rt);
    }
    if rt.check_and_reset_cur_map_loaded(1) {
        // `.setFirstLockTrashCanIndex`: the gym's first switch is chosen out here, on `hRandomSub`.
        rt.random();
        let can = rt.random() & 0x0E;
        rt.globals().trash_cans[0] = can;
    }
    match rt.maps().vermilion_city.cur_script {
        SCRIPT_VERMILIONCITY_PLAYER_MOVING_UP1 => moving_up1(rt),
        SCRIPT_VERMILIONCITY_PLAYER_EXIT_SHIP => exit_ship(rt),
        SCRIPT_VERMILIONCITY_PLAYER_MOVING_UP2 => moving_up2(rt),
        SCRIPT_VERMILIONCITY_PLAYER_ALLOWED_TO_PASS => allowed_to_pass(rt),
        _ => default_script(rt),
    }
}

/// `VermilionCityLeftSSAnneCallbackScript`: the first load of the town after the ship has sailed is
/// the player stepping off the pier, and they are walked up off it.
fn left_ss_anne_callback(rt: &mut Script) {
    if !rt.check_event(EVENT_SS_ANNE_LEFT) || rt.check_and_set_event(EVENT_WALKED_PAST_GUARD_AFTER_SS_ANNE_LEFT) {
        return;
    }
    rt.maps().vermilion_city.cur_script = SCRIPT_VERMILIONCITY_PLAYER_EXIT_SHIP;
}

/// `VermilionCityDefaultScript`: walking down onto the sailor's square is the check, not talking to him.
fn default_script(rt: &mut Script) -> Flow {
    if rt.player_facing() != SPRITE_FACING_DOWN || rt.are_player_coords_in_array(&TICKET_CHECK_COORDS).is_none() {
        return Flow::Return;
    }
    rt.clear_joy_held();
    rt.display_text_id(TEXT_VERMILIONCITY_SAILOR1).then(Label::TicketCheck)
}

/// `VermilionCityPlayerMovingUp1Script`.
fn moving_up1(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.delay_frames(10).then(Label::MovingUp1Delay)
}

/// `VermilionCityPlayerExitShipScript`.
fn exit_ship(rt: &mut Script) -> Flow {
    rt.joy_ignore(PAD_BUTTONS_AND_CTRL_PAD);
    rt.simulate_joypad_presses(vec![Joypad::UP, Joypad::UP]);
    rt.maps().vermilion_city.cur_script = SCRIPT_VERMILIONCITY_PLAYER_MOVING_UP2;
    Flow::Return
}

/// `VermilionCityPlayerMovingUp2Script`.
fn moving_up2(rt: &mut Script) -> Flow {
    if rt.simulated_joypad_states_index() != 0 {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.clear_joy_held();
    rt.maps().vermilion_city.cur_script = SCRIPT_VERMILIONCITY_DEFAULT;
    Flow::Return
}

/// `VermilionCityPlayerAllowedToPassScript`: the ticket is good for as long as the player is on the
/// sailor's square, and the check is armed again the moment they step off it.
fn allowed_to_pass(rt: &mut Script) -> Flow {
    if rt.are_player_coords_in_array(&TICKET_CHECK_COORDS).is_none() {
        rt.maps().vermilion_city.cur_script = SCRIPT_VERMILIONCITY_DEFAULT;
    }
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_VERMILIONCITY_GAMBLER1 => {
            use local::VermilionCityGambler1Text as words;
            let said = match rt.check_event(EVENT_SS_ANNE_LEFT) {
                true => words::SSAnneDepartedText,
                false => words::DidYouSeeText,
            };
            rt.print_text(text_at(said)).ret()
        }
        TEXT_VERMILIONCITY_SAILOR1 => sailor_text(rt),
        TEXT_VERMILIONCITY_MACHOP => rt.print_text(text_at(sym::VermilionCityMachopText)).then(Label::MachopCry),
        _ => return None,
    })
}

/// `VermilionCitySailor1Text`.
fn sailor_text(rt: &mut Script) -> Flow {
    use local::VermilionCitySailor1Text as words;
    if rt.check_event(EVENT_SS_ANNE_LEFT) {
        return rt.print_text(text_at(words::ShipSetSailText)).ret();
    }
    if rt.player_facing() == SPRITE_FACING_RIGHT || rt.are_player_coords_in_array(&GUARD_COORDS).is_none() {
        return rt.print_text(text_at(words::WelcomeToSSAnneText)).ret();
    }
    rt.print_text(text_at(words::DoYouHaveATicketText)).then(Label::SailorTicket)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    use local::VermilionCitySailor1Text as sailor;
    match label {
        // A ticket lets the player walk on; anyone else is pushed back up the square they came from.
        Label::TicketCheck => {
            if !rt.check_event(EVENT_SS_ANNE_LEFT) && rt.is_item_in_bag(ItemId::SSTicket) {
                return Flow::Return;
            }
            rt.simulate_joypad_presses(vec![Joypad::UP]);
            rt.maps().vermilion_city.cur_script = SCRIPT_VERMILIONCITY_PLAYER_MOVING_UP1;
            Flow::Return
        }
        Label::MovingUp1Delay => {
            rt.maps().vermilion_city.cur_script = SCRIPT_VERMILIONCITY_DEFAULT;
            Flow::Return
        }
        Label::SailorTicket => {
            if !rt.is_item_in_bag(ItemId::SSTicket) {
                return rt.print_text(text_at(sailor::YouNeedATicketText)).ret();
            }
            rt.maps().vermilion_city.cur_script = SCRIPT_VERMILIONCITY_PLAYER_ALLOWED_TO_PASS;
            rt.print_text(text_at(sailor::FlashedTicketText)).ret()
        }
        Label::MachopCry => {
            rt.play_cry(PokemonSpecies::Machop);
            rt.wait_for_sound_to_finish().then(Label::MachopStomp)
        }
        Label::MachopStomp => {
            rt.print_text(text_at(local::VermilionCityMachopText::StompingTheLandFlatText)).ret()
        }
    }
}
