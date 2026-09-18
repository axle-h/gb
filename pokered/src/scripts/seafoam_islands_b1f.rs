//! `SeafoamIslandsB1F_Script`: the boulders that came down from the floor above, and the next pair
//! of holes for them.

use poke_core::map::Map;
use poke_core::symbols::pokered_events::{EVENT_SEAFOAM2_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM2_BOULDER2_DOWN_HOLE};
use poke_core::symbols::pokered_toggles::{TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_2};
use serde::{Deserialize, Serialize};
use super::seafoam_islands::{boulders_and_holes, Floor};
use super::{Flow, Script};

/// `Seafoam2HolesCoords`.
const FLOOR: Floor = Floor {
    holes: [(18, 6), (23, 6)],
    down_hole: [EVENT_SEAFOAM2_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM2_BOULDER2_DOWN_HOLE],
    hide: [TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B1F_BOULDER_2],
    show: [TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_2],
    below: Map::SeafoamIslandsB2F,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    boulders_and_holes(rt, &FLOOR);
    Flow::Return
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

pub fn resume(_rt: &mut Script, label: Label) -> Flow {
    match label {}
}
