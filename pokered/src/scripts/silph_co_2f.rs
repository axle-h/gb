//! `SilphCo2F_Script`: two scientists and two Rockets, the two card key doors the floor draws shut
//! until they are opened, and the Silph worker who hands over TM36.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::{EVENT_GOT_TM36, EVENT_SILPH_CO_2_UNLOCKED_DOOR1, EVENT_SILPH_CO_2_UNLOCKED_DOOR2};
use poke_core::symbols::pokered_local_labels::SilphCo2FSilphWorkerFText as worker;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO2F_ROCKET1, TEXT_SILPHCO2F_ROCKET2, TEXT_SILPHCO2F_SCIENTIST1,
    TEXT_SILPHCO2F_SCIENTIST2, TEXT_SILPHCO2F_SILPH_WORKER_F};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// The block a card key door is drawn as while it is still shut.
const CLOSED_DOOR: u8 = 0x54;
/// `SilphCo2FGateCallbackScript.GateCoordinates`, in blocks.
const GATES: [(u8, u8); 2] = [(2, 2), (2, 5)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo2FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo2FCurScript], a` after the table's routine.
    StoreCurScript,
    Tm36Offered,
}

pub fn script(rt: &mut Script) -> Flow {
    gate_callback(rt);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_2f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo2TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

/// `SilphCo2F_SetCardKeyDoorYScript`: `hUnlockedSilphCoDoors`, which is where in this floor's gate
/// list the door a card key has just opened is, counted from one, and zero for none of them. The
/// coordinate is cleared by the match, so no other floor opens a gate at the same square.
pub(super) fn unlocked_door(rt: &mut Script, gates: &[(u8, u8)]) -> u8 {
    let Some(index) = gates.iter().position(|&gate| gate == rt.card_key_door()) else {
        return 0;
    };
    rt.clear_card_key_door();
    index as u8 + 1
}

/// `SilphCo2FGateCallbackScript`: the two gates are redrawn shut on every load, so a door opened on
/// one visit is the only one that stays open.
fn gate_callback(rt: &mut Script) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    match unlocked_door(rt, &GATES) {
        0 => {}
        1 => rt.set_event(EVENT_SILPH_CO_2_UNLOCKED_DOOR1),
        _ => rt.set_event(EVENT_SILPH_CO_2_UNLOCKED_DOOR2),
    }
    for (event, (x, y)) in [(EVENT_SILPH_CO_2_UNLOCKED_DOOR1, GATES[0]), (EVENT_SILPH_CO_2_UNLOCKED_DOOR2, GATES[1])] {
        if !rt.check_event(event) {
            rt.replace_tile_block(x, y, CLOSED_DOOR);
        }
    }
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SILPHCO2F_SILPH_WORKER_F => return Some(worker_text(rt)),
        TEXT_SILPHCO2F_SCIENTIST1 => sym::SilphCo2TrainerHeader0,
        TEXT_SILPHCO2F_SCIENTIST2 => sym::SilphCo2TrainerHeader1,
        TEXT_SILPHCO2F_ROCKET1 => sym::SilphCo2TrainerHeader2,
        TEXT_SILPHCO2F_ROCKET2 => sym::SilphCo2TrainerHeader3,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `SilphCo2FSilphWorkerFText`.
fn worker_text(rt: &mut Script) -> Flow {
    match rt.check_event(EVENT_GOT_TM36) {
        true => rt.print_text(text_at(worker::TM36ExplanationText)).ret(),
        false => rt.print_text(text_at(worker::PleaseTakeThisText)).then(Label::Tm36Offered),
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_2f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::Tm36Offered => {
            let said = match rt.give_item(ItemId::Tm36Selfdestruct, 1) {
                true => {
                    rt.set_event(EVENT_GOT_TM36);
                    worker::ReceivedTM36Text
                }
                false => worker::TM36NoRoomText,
            };
            rt.print_text(text_at(said)).ret()
        }
    }
}
