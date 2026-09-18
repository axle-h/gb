//! `SeafoamIslands1F_Script`: the two boulders at the top of the puzzle and the holes they go down,
//! and the event that tells Route 20 the player has been inside.

use poke_core::map::Map;
use poke_core::symbols::pokered_events::{EVENT_IN_SEAFOAM_ISLANDS, EVENT_SEAFOAM1_BOULDER1_DOWN_HOLE,
    EVENT_SEAFOAM1_BOULDER2_DOWN_HOLE};
use poke_core::symbols::pokered_toggles::{TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_2};
use serde::{Deserialize, Serialize};
use super::seafoam_islands::{boulders_and_holes, Floor};
use super::{Flow, Script};

/// `Seafoam1HolesCoords`.
const FLOOR: Floor = Floor {
    holes: [(17, 6), (24, 6)],
    down_hole: [EVENT_SEAFOAM1_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM1_BOULDER2_DOWN_HOLE],
    hide: [TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_1F_BOULDER_2],
    show: [TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_2],
    below: Map::SeafoamIslandsB1F,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    rt.set_event(EVENT_IN_SEAFOAM_ISLANDS);
    boulders_and_holes(rt, &FLOOR);
    Flow::Return
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
