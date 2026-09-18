//! `Museum1F_Script`: the ¥50 door fee, which the scientist at the counter stops the player in the
//! doorway to ask for, and the colleague at the back who gives the Old Amber away.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_BOUGHT_MUSEUM_TICKET, EVENT_GOT_OLD_AMBER};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_MUSEUM1F_NOOP, TEXT_MUSEUM1F_GAMBLER, TEXT_MUSEUM1F_OLD_AMBER,
    TEXT_MUSEUM1F_SCIENTIST1, TEXT_MUSEUM1F_SCIENTIST2, TEXT_MUSEUM1F_SCIENTIST3};
use poke_core::symbols::pokered_toggles::TOGGLE_OLD_AMBER;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::input::Joypad;
use super::{text_at, Flow, Script};

use local::Museum1FScientist1Text as counter;
use local::Museum1FScientist2Text as amber;

/// A child's ticket, as BCD.
const FEE: [u8; 3] = [0x00, 0x00, 0x50];

/// `Museum1FDefaultScript`: the two squares inside the west door, from which the counter is asked.
const DOORWAY_Y: u8 = 4;
const DOORWAY_X: [u8; 2] = [9, 10];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wMuseum1FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `Museum1FScientist1Text.no_ticket` after `.WouldYouLikeToComeInText`, and after the yes/no.
    TicketAsked,
    TicketAnswered,
    /// `.DontHaveEnoughMoneyText` printed, which falls into `.deny_entry`.
    NoMoney,
    /// `.deny_entry` after `.ComeAgainText`.
    DenyEntry,
    /// `.buy_ticket` after `.ThankYouText`, then `PlaySoundWaitForCurrent` and its own wait.
    BoughtTicket,
    PurchaseSound,
    AllowEntry,
    /// `.behind_counter` after `.DoYouKnowWhatAmberIsText`, and after the yes/no.
    AmberAsked,
    AmberAnswered,
    /// `Museum1FScientist2Text` after `.TakeThisToAPokemonLabText`.
    AmberOffered,
}

pub fn script(rt: &mut Script) -> Flow {
    // The map draws its own text boxes, since the doorway text is one the player did not ask for.
    rt.disable_auto_text_box_drawing();
    match rt.maps().museum_1f.cur_script {
        SCRIPT_MUSEUM1F_NOOP => Flow::Return,
        _ => default_script(rt),
    }
}

/// `Museum1FDefaultScript`.
fn default_script(rt: &mut Script) -> Flow {
    if rt.y() != DOORWAY_Y || !DOORWAY_X.contains(&rt.x()) {
        return Flow::Return;
    }
    rt.clear_joy_held();
    rt.display_text_id(TEXT_MUSEUM1F_SCIENTIST1).ret()
}

/// `Museum1FScientist1Text`: the counter, which answers by where it is spoken to from. Behind it is
/// the square right of the scientist or the one above him, `.not_right_of_scientist` comparing the Y
/// still in `a` with 3 rather than reloading the X its name suggests.
fn counter_text(rt: &mut Script) -> Flow {
    let (x, y) = (rt.x(), rt.y());
    if (y == 4 && x == 13) || (y == 3 && x == 12) {
        return rt.print_text(text_at(counter::DoYouKnowWhatAmberIsText)).then(Label::AmberAsked);
    }
    if rt.check_event(EVENT_BOUGHT_MUSEUM_TICKET) {
        return rt.print_text(text_at(counter::TakePlentyOfTimeText)).ret();
    }
    if y != 4 {
        return rt.print_text(text_at(counter::GoToOtherSideText)).ret();
    }
    rt.money_box();
    rt.clear_joy_held();
    rt.print_text(text_at(counter::WouldYouLikeToComeInText)).then(Label::TicketAsked)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_MUSEUM1F_SCIENTIST1 => counter_text(rt),
        TEXT_MUSEUM1F_GAMBLER => rt.print_text(text_at(local::Museum1FGamblerText::Text)).ret(),
        TEXT_MUSEUM1F_SCIENTIST2 => {
            if rt.check_event(EVENT_GOT_OLD_AMBER) {
                return Some(rt.print_text(text_at(amber::GetTheOldAmberCheckText)).ret());
            }
            rt.print_text(text_at(amber::TakeThisToAPokemonLabText)).then(Label::AmberOffered)
        }
        TEXT_MUSEUM1F_SCIENTIST3 => rt.print_text(text_at(local::Museum1FScientist3Text::Text)).ret(),
        TEXT_MUSEUM1F_OLD_AMBER => rt.print_text(text_at(local::Museum1FOldAmberText::Text)).ret(),
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::TicketAsked => rt.yes_no_choice().then(Label::TicketAnswered),
        Label::TicketAnswered => {
            if !rt.chose_yes() {
                return deny_entry(rt);
            }
            if !rt.has_enough_money(FEE) {
                return rt.print_text(text_at(counter::DontHaveEnoughMoneyText)).then(Label::NoMoney);
            }
            rt.print_text(text_at(counter::ThankYouText)).then(Label::BoughtTicket)
        }
        Label::NoMoney => deny_entry(rt),
        // Nothing arms a script to wait on the step, so the default routine asks again as soon as
        // the player walks back into the doorway.
        Label::DenyEntry => {
            rt.simulate_joypad_presses(vec![Joypad::DOWN]);
            rt.update_sprites();
            Flow::Return
        }
        Label::BoughtTicket => {
            rt.set_event(EVENT_BOUGHT_MUSEUM_TICKET);
            rt.subtract_money(FEE);
            rt.money_box();
            rt.wait_for_sound_to_finish().then(Label::PurchaseSound)
        }
        Label::PurchaseSound => {
            rt.play_sound(sounds::SFX_PURCHASE);
            rt.wait_for_sound_to_finish().then(Label::AllowEntry)
        }
        // `.allow_entry` leaves the map on its noop script, so the doorway never asks again.
        Label::AllowEntry => {
            rt.maps().museum_1f.cur_script = SCRIPT_MUSEUM1F_NOOP;
            Flow::Return
        }
        Label::AmberAsked => rt.yes_no_choice().then(Label::AmberAnswered),
        // Saying yes to knowing what amber is earns the tip about the lab, saying no the definition.
        Label::AmberAnswered => {
            let said = match rt.chose_yes() {
                true => counter::TheresALabSomewhereText,
                false => counter::AmberIsFossilizedTreeSapText,
            };
            rt.print_text(text_at(said)).ret()
        }
        Label::AmberOffered => {
            if !rt.give_item(ItemId::OldAmber, 1) {
                return rt.print_text(text_at(amber::YouDontHaveSpaceText)).ret();
            }
            rt.set_event(EVENT_GOT_OLD_AMBER);
            rt.hide_object(TOGGLE_OLD_AMBER);
            rt.print_text(text_at(amber::ReceivedOldAmberText)).ret()
        }
    }
}

fn deny_entry(rt: &mut Script) -> Flow {
    rt.print_text(text_at(counter::ComeAgainText)).then(Label::DenyEntry)
}
