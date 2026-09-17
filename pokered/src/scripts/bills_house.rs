//! `BillsHouse_Script`: Bill stuck as a Pokémon, the cell separator that gets him out, and the
//! S.S. Ticket he pays with.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BILL_SAID_USE_CELL_SEPARATOR, EVENT_GOT_SS_TICKET, EVENT_MET_BILL,
    EVENT_MET_BILL_2, EVENT_USED_CELL_SEPARATOR_ON_BILL};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::*;
use poke_core::symbols::pokered_symbols::{BILLSHOUSE_BILL1, BILLSHOUSE_BILL_POKEMON};
use poke_core::symbols::pokered_toggles::{TOGGLE_BILL_1, TOGGLE_BILL_POKEMON, TOGGLE_CERULEAN_GUARD_1,
    TOGGLE_CERULEAN_GUARD_2};
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::modes::overworld::movement::{NPC_MOVEMENT_DOWN, NPC_MOVEMENT_LEFT, NPC_MOVEMENT_RIGHT,
    NPC_MOVEMENT_UP};
use crate::modes::overworld::script::SpritePosition;
use crate::systems::overworld::sprites::SPRITE_FACING_DOWN;
use super::{text_at, Flow, Script};

const END: u8 = 0xFF;
const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);

/// `.PokemonWalkToMachineMovement` and `.PokemonWalkAroundPlayerMovement`: a player facing down is
/// standing in the way, so Bill goes round rather than through.
const WALK_TO_MACHINE: [u8; 4] = [NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, END];
const WALK_AROUND_PLAYER: [u8; 6] = [NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, NPC_MOVEMENT_LEFT,
    NPC_MOVEMENT_UP, END];
/// `.BillExitMachineMovement`.
const EXIT_MACHINE: [u8; 6] = [NPC_MOVEMENT_DOWN, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT, NPC_MOVEMENT_RIGHT,
    NPC_MOVEMENT_DOWN, END];
/// Where Bill steps out of the machine, put there by hand because his sprite starts the map hidden.
const BILL_AT_MACHINE: SpritePosition = SpritePosition { screen_y: 0x0C, screen_x: 0x40, map_y: 6, map_x: 5 };

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wBillsHouseCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    BillPlaced,
    PcText,
    NotAPokemonYesNo,
    NotAPokemonAnswered,
    RefusedOnce,
    UseSeparationSystem,
    ThankYou,
    TicketReceived,
    GoInsteadOfMe,
    CheckOutMyRarePokemon,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    match rt.maps().bills_house.cur_script {
        SCRIPT_BILLSHOUSE_POKEMON_WALK_TO_MACHINE => walk_to_machine(rt),
        SCRIPT_BILLSHOUSE_POKEMON_ENTERS_MACHINE => enters_machine(rt),
        SCRIPT_BILLSHOUSE_BILL_EXITS_MACHINE => exits_machine(rt),
        SCRIPT_BILLSHOUSE_CLEANUP => cleanup(rt),
        SCRIPT_BILLSHOUSE_PC => rt.display_text_id(TEXT_BILLSHOUSE_ACTIVATE_PC).then(Label::PcText),
        _ => Flow::Return,
    }
}

/// `BillsHousePokemonWalkToMachineScript`.
fn walk_to_machine(rt: &mut Script) -> Flow {
    let path: &[u8] = match rt.player_facing() == SPRITE_FACING_DOWN {
        true => &WALK_AROUND_PLAYER,
        false => &WALK_TO_MACHINE,
    };
    rt.move_sprite(BILLSHOUSE_BILL_POKEMON, path);
    rt.maps().bills_house.cur_script = SCRIPT_BILLSHOUSE_POKEMON_ENTERS_MACHINE;
    Flow::Return
}

/// `BillsHousePokemonEntersMachineScript`.
fn enters_machine(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.hide_object(TOGGLE_BILL_POKEMON);
    rt.set_event(EVENT_BILL_SAID_USE_CELL_SEPARATOR);
    rt.joy_ignore(Joypad::empty());
    rt.maps().bills_house.cur_script = SCRIPT_BILLSHOUSE_BILL_EXITS_MACHINE;
    Flow::Return
}

/// `BillsHouseBillExitsMachineScript`: the PC, not the script, is what puts Bill back together, so
/// this waits on the event the cell separator sets.
fn exits_machine(rt: &mut Script) -> Flow {
    if !rt.check_event(EVENT_USED_CELL_SEPARATOR_ON_BILL) {
        return Flow::Return;
    }
    rt.joy_ignore(PAD_CTRL_PAD);
    rt.set_sprite_index(BILLSHOUSE_BILL1);
    rt.set_sprite_position(BILLSHOUSE_BILL1, BILL_AT_MACHINE);
    rt.show_object(TOGGLE_BILL_1);
    rt.delay_frames(8).then(Label::BillPlaced)
}

/// `BillsHouseCleanupScript`.
fn cleanup(rt: &mut Script) -> Flow {
    if rt.npc_moving() {
        return Flow::Return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.set_event(EVENT_MET_BILL_2);
    rt.set_event(EVENT_MET_BILL);
    rt.maps().bills_house.cur_script = SCRIPT_BILLSHOUSE_DEFAULT;
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_BILLSHOUSE_BILL_POKEMON => {
            rt.print_text(text_at(local::BillsHouseBillPokemonText::ImNotAPokemonText)).then(Label::NotAPokemonYesNo)
        }
        TEXT_BILLSHOUSE_BILL_SS_TICKET => ss_ticket_text(rt),
        TEXT_BILLSHOUSE_BILL_CHECK_OUT_MY_RARE_POKEMON => {
            rt.print_text(text_at(local::BillsHouseBillCheckOutMyRarePokemonText::Text)).ret()
        }
        _ => return None,
    })
}

/// `BillsHouseBillSSTicketText`: the ticket is handed over once, and the guard blocking Vermilion's
/// dock is swapped for the one who lets the player past.
fn ss_ticket_text(rt: &mut Script) -> Flow {
    use local::BillsHouseBillSSTicketText as words;
    if rt.check_event(EVENT_GOT_SS_TICKET) {
        return rt.print_text(text_at(words::WhyDontYouGoInsteadOfMeText)).ret();
    }
    rt.print_text(text_at(words::ThankYouText)).then(Label::ThankYou)
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    use local::BillsHouseBillPokemonText as pokemon;
    use local::BillsHouseBillSSTicketText as ticket;
    match label {
        Label::BillPlaced => {
            rt.move_sprite(BILLSHOUSE_BILL1, &EXIT_MACHINE);
            rt.maps().bills_house.cur_script = SCRIPT_BILLSHOUSE_CLEANUP;
            Flow::Return
        }
        Label::PcText => {
            rt.maps().bills_house.cur_script = SCRIPT_BILLSHOUSE_DEFAULT;
            Flow::Return
        }
        Label::NotAPokemonYesNo => rt.yes_no_choice().then(Label::NotAPokemonAnswered),
        // No is not an answer: he asks again and takes the second silence for a yes.
        Label::NotAPokemonAnswered => match rt.chose_yes() {
            true => rt.print_text(text_at(pokemon::UseSeparationSystemText)).then(Label::UseSeparationSystem),
            false => rt.print_text(text_at(pokemon::NoYouGottaHelpText)).then(Label::RefusedOnce),
        },
        Label::RefusedOnce => rt.print_text(text_at(pokemon::UseSeparationSystemText)).then(Label::UseSeparationSystem),
        Label::UseSeparationSystem => {
            rt.maps().bills_house.cur_script = SCRIPT_BILLSHOUSE_POKEMON_WALK_TO_MACHINE;
            Flow::Return
        }
        Label::ThankYou => {
            if !rt.give_item(ItemId::SSTicket, 1) {
                return rt.print_text(text_at(ticket::SSTicketNoRoomText)).ret();
            }
            rt.print_text(text_at(ticket::SSTicketReceivedText)).then(Label::TicketReceived)
        }
        Label::TicketReceived => {
            rt.set_event(EVENT_GOT_SS_TICKET);
            rt.show_object(TOGGLE_CERULEAN_GUARD_1);
            rt.hide_object(TOGGLE_CERULEAN_GUARD_2);
            rt.print_text(text_at(ticket::WhyDontYouGoInsteadOfMeText)).then(Label::GoInsteadOfMe)
        }
        Label::GoInsteadOfMe | Label::CheckOutMyRarePokemon => Flow::Return,
    }
}
