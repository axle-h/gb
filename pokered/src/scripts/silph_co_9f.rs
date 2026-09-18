//! `SilphCo9F_Script`: two Rockets and a scientist, four card key doors in two different tilings, and
//! the nurse who heals the party for as long as Team Rocket hold the building.

use poke_core::symbols::pokered_events::{EVENT_BEAT_SILPH_CO_GIOVANNI, EVENT_SILPH_CO_9_UNLOCKED_DOOR1,
    EVENT_SILPH_CO_9_UNLOCKED_DOOR2, EVENT_SILPH_CO_9_UNLOCKED_DOOR3, EVENT_SILPH_CO_9_UNLOCKED_DOOR4};
use poke_core::symbols::pokered_local_labels::SilphCo9FNurseText as nurse;
use poke_core::symbols::pokered_map_scripts::{TEXT_SILPHCO9F_NURSE, TEXT_SILPHCO9F_ROCKET1,
    TEXT_SILPHCO9F_ROCKET2, TEXT_SILPHCO9F_SCIENTIST};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// `SilphCo9FGateCallbackScript.GateCoordinates`, in blocks, the event each gate has, and the block
/// it is drawn as while it is shut: the two gates in the middle of the floor are tiled the other way
/// round from the two on its edges.
const GATES: [(u8, u8); 4] = [(1, 4), (9, 2), (9, 5), (5, 6)];
const DOORS: [u16; 4] = [EVENT_SILPH_CO_9_UNLOCKED_DOOR1, EVENT_SILPH_CO_9_UNLOCKED_DOOR2,
    EVENT_SILPH_CO_9_UNLOCKED_DOOR3, EVENT_SILPH_CO_9_UNLOCKED_DOOR4];
const CLOSED_DOORS: [u8; 4] = [0x5F, 0x54, 0x54, 0x5F];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wSilphCo9FCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wSilphCo9FCurScript], a` after the table's routine.
    StoreCurScript,
    YouLookTired,
    FadedOut,
    Delayed,
    FadedIn,
}

pub fn script(rt: &mut Script) -> Flow {
    // `SilphCo9FGateCallbackScript`.
    super::silph_co::gate_callback_blocks(rt, &GATES, &DOORS, &CLOSED_DOORS);
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().silph_co_9f.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::SilphCo9TrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    let header = match text_id {
        TEXT_SILPHCO9F_NURSE => return Some(nurse_text(rt)),
        TEXT_SILPHCO9F_ROCKET1 => sym::SilphCo9TrainerHeader0,
        TEXT_SILPHCO9F_SCIENTIST => sym::SilphCo9TrainerHeader1,
        TEXT_SILPHCO9F_ROCKET2 => sym::SilphCo9TrainerHeader2,
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).ret())
}

/// `SilphCo9FNurseText`: once the building is Silph's again she only thanks the player, so the one
/// free heal in the game is gone with Team Rocket.
fn nurse_text(rt: &mut Script) -> Flow {
    match rt.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
        true => rt.print_text(text_at(nurse::ThankYouText)).ret(),
        false => rt.print_text(text_at(nurse::YouLookTiredText)).then(Label::YouLookTired),
    }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript => {
            rt.maps().silph_co_9f.cur_script = rt.cur_map_script();
            Flow::Return
        }
        // The party is healed behind the fade rather than by a healing machine: there is none here.
        Label::YouLookTired => {
            rt.heal_party();
            rt.gb_fade_out_to_white().then(Label::FadedOut)
        }
        Label::FadedOut => rt.delay3().then(Label::Delayed),
        Label::Delayed => rt.gb_fade_in_from_white().then(Label::FadedIn),
        Label::FadedIn => rt.print_text(text_at(nurse::DontGiveUpText)).ret(),
    }
}
