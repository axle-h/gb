//! `IndigoPlateauLobby_Script`: the lobby forgets a half-fought Elite Four challenge.

use poke_core::symbols::pokered_events::{EVENT_LANCES_ROOM_LOCK_DOOR, EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH};
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `INDIGO_PLATEAU_EVENTS_START`, which is a bare address rather than an event of its own.
const INDIGO_PLATEAU_EVENTS_START: u16 = 0x8E0;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

/// Coming back down here puts Victory Road's first switch back up, and undoes every room of a
/// challenge that was started and not finished, so the four are fought from the beginning again.
/// The cable club receptionist is a non-goal, so `Serial_TryEstablishingExternallyClockedConnection`
/// is not recreated.
pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    if !rt.check_and_reset_cur_map_loaded(2) {
        return Flow::Return;
    }
    rt.reset_event(EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH);
    if !std::mem::take(&mut rt.globals().started_elite_4) {
        return Flow::Return;
    }
    for event in INDIGO_PLATEAU_EVENTS_START..=EVENT_LANCES_ROOM_LOCK_DOOR {
        rt.reset_event(event);
    }
    Flow::Return
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
