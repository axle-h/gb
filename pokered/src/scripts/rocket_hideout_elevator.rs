//! `RocketHideoutElevator_Script`: the three floors the hideout's lift stops at, behind the Lift Key.

use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::symbols::pokered_local_labels::RocketHideoutElevatorText as elevator;
use poke_core::symbols::pokered_map_scripts::TEXT_ROCKETHIDEOUTELEVATOR;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

/// `RocketHideoutElevatorFloors`, the buttons on the panel. B3F is not one of them, so the floor
/// Giovanni's lift key is guarded on has to be walked down to.
const FLOORS: [ItemId; 3] = [ItemId::FloorB1F, ItemId::FloorB2F, ItemId::FloorB4F];
/// `RocketHideoutElevatorWarpMaps`: for each button, the warp of the floor the doors then open onto.
const WARP_MAPS: [(u8, Map); 3] = [(4, Map::RocketHideoutB1F), (4, Map::RocketHideoutB2F),
    (2, Map::RocketHideoutB4F)];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Shaken,
}

pub fn script(rt: &mut Script) -> Flow {
    if rt.check_and_reset_cur_map_loaded(1) {
        store_warp_entries(rt);
    }
    if rt.check_and_reset_used_elevator() {
        return rt.shake_elevator().then(Label::Shaken);
    }
    auto_text_box(rt)
}

/// `RocketHideoutElevatorStoreWarpEntriesScript`: both doors are pointed back at the warp the player
/// came in through, so stepping out without choosing a floor leads back where it came from.
fn store_warp_entries(rt: &mut Script) {
    let (warp, map) = rt.warped_from();
    for door in 0..2 {
        rt.set_warp_destination(door, warp, map);
    }
}

/// The panel's box is drawn for it, and it opens straight onto the floor menu rather than waiting
/// for a press of its own. `EnableAutoTextBoxDrawing` clears the second flag, so it is set after.
fn auto_text_box(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    rt.set_do_not_wait_for_button_press(true);
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    if text_id != TEXT_ROCKETHIDEOUTELEVATOR {
        return None;
    }
    // Without the key the panel says so and stays shut; the key is never taken.
    if !rt.is_item_in_bag(ItemId::LiftKey) {
        return Some(rt.print_text(text_at(elevator::AppearsToNeedKeyText)).ret());
    }
    Some(rt.display_elevator_floor_menu(FLOORS.to_vec(), WARP_MAPS.to_vec()).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Shaken => auto_text_box(rt),
    }
}
