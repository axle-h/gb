//! `SeafoamIslandsB2F_Script`: the last pair of holes above the water, which a boulder has to go
//! down before the current on B3F slows.

use poke_core::map::Map;
use poke_core::symbols::pokered_events::{EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE};
use poke_core::symbols::pokered_toggles::{TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_2,
    TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_4};
use serde::{Deserialize, Serialize};
use super::seafoam_islands::{boulders_and_holes, Floor};
use super::{Flow, Script};

/// `Seafoam3HolesCoords`. The boulders land in B3F's water as its objects 3 and 4, its own pair
/// being the ones that go on down to B4F.
const FLOOR: Floor = Floor {
    holes: [(19, 6), (22, 6)],
    down_hole: [EVENT_SEAFOAM3_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM3_BOULDER2_DOWN_HOLE],
    hide: [TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_1, TOGGLE_SEAFOAM_ISLANDS_B2F_BOULDER_2],
    show: [TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3, TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_4],
    below: Map::SeafoamIslandsB3F,
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
