//! The block every Seafoam floor above the water opens with: a boulder pushed onto one of the
//! floor's two holes drops to the floor below and reappears there, and a player standing on a hole
//! falls after it. `SeafoamIslands1F_Script` and the three floors under it differ only in the
//! squares, the events and which objects are swapped.

use poke_core::map::Map;
use super::Script;

/// One floor's `.hideAndShowBoulderObjects` block. The events and the shown objects are named for
/// the floor below, which is where the boulder lands.
pub struct Floor {
    /// `Seafoam<n>HolesCoords`, as (x, y).
    pub holes: [(u8, u8); 2],
    /// `EVENT_SEAFOAM<n>_BOULDER1_DOWN_HOLE` and its pair.
    pub down_hole: [u16; 2],
    /// `wObjectToHide`: this floor's two boulders.
    pub hide: [u16; 2],
    /// `wObjectToShow`: the boulders of the floor below, where they land.
    pub show: [u16; 2],
    /// `wDungeonWarpDestinationMap`.
    pub below: Map,
}

/// Whether the floor's own script carries on: a pushed boulder that missed both holes and a fall
/// down one both return, as the cartridge does, and the rest falls through.
pub fn boulders_and_holes(rt: &mut Script, floor: &Floor) -> bool {
    if rt.check_and_reset_pushed_boulder() {
        let Some(hole) = rt.check_boulder_coords(&floor.holes) else {
            return false;
        };
        rt.set_event(floor.down_hole[hole]);
        rt.hide_object(floor.hide[hole]);
        rt.show_object(floor.show[hole]);
    } else if let Some(which) = rt.are_player_coords_in_array(&floor.holes) {
        // `IsPlayerOnDungeonWarp`: the fall is taken before the map's own script runs.
        rt.fall_down_hole(floor.below, which);
        return false;
    }
    true
}
