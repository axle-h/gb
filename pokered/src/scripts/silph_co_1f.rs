//! `SilphCo1F_Script`: the receptionist, who is only back behind her desk once Giovanni has been
//! beaten.

use poke_core::symbols::pokered_events::{EVENT_BEAT_SILPH_CO_GIOVANNI, EVENT_SILPH_CO_RECEPTIONIST_AT_DESK};
use poke_core::symbols::pokered_toggles::TOGGLE_SILPH_CO_1F_RECEPTIONIST;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    if !rt.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
        return Flow::Return;
    }
    // The event remembers that she has been shown, so every later pass leaves her alone.
    if !rt.check_and_set_event(EVENT_SILPH_CO_RECEPTIONIST_AT_DESK) {
        rt.show_object(TOGGLE_SILPH_CO_1F_RECEPTIONIST);
    }
    Flow::Return
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
