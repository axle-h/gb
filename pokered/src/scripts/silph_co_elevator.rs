//! `SilphCoElevator_Script`: the eleven buttons, and the doors they aim at the floor chosen.

use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::symbols::pokered_map_scripts::TEXT_SILPHCOELEVATOR_ELEVATOR;
use serde::{Deserialize, Serialize};
use super::{Flow, Script};

/// `SilphCoElevatorFloors`, the buttons on the panel.
const FLOORS: [ItemId; 11] = [ItemId::Floor1F, ItemId::Floor2F, ItemId::Floor3F, ItemId::Floor4F,
    ItemId::Floor5F, ItemId::Floor6F, ItemId::Floor7F, ItemId::Floor8F, ItemId::Floor9F,
    ItemId::Floor10F, ItemId::Floor11F];
/// `SilphCoElevatorWarpMaps`: for each button, the warp of the floor the doors then open onto.
const WARP_MAPS: [(u8, Map); 11] = [(3, Map::SilphCo1F), (2, Map::SilphCo2F), (2, Map::SilphCo3F),
    (2, Map::SilphCo4F), (2, Map::SilphCo5F), (2, Map::SilphCo6F), (2, Map::SilphCo7F),
    (2, Map::SilphCo8F), (2, Map::SilphCo9F), (2, Map::SilphCo10F), (1, Map::SilphCo11F)];

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
        // `SilphCoElevatorShakeScript`, whose `Delay3` is the list menu's teardown reaching the
        // screen rather than anything to be seen.
        return rt.shake_elevator().then(Label::Shaken);
    }
    auto_text_box(rt)
}

/// `SilphCoElevatorStoreWarpEntriesScript`: both doors are pointed back at the warp the player came
/// in through, so stepping out without choosing a floor leads back where it came from. The map's own
/// two warps lead nowhere, and nothing else ever writes them.
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
    if text_id != TEXT_SILPHCOELEVATOR_ELEVATOR {
        return None;
    }
    // `SilphCoElevatorCopyWarpMapsScript` then `DisplayElevatorFloorMenu`.
    Some(rt.display_elevator_floor_menu(FLOORS.to_vec(), WARP_MAPS.to_vec()).ret())
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::Shaken => auto_text_box(rt),
    }
}
