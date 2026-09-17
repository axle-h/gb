//! The bicycle and surfing as rules: where the bike may be ridden, the squares that put the player
//! on the bike or the water, and what the water lets the player do.

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_events::{EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE, EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use crate::world::EventFlags;
use super::collision::{tile_pair_collision, tile_passable};
use super::location::{Ahead, Location, BIKING, SURFING};

/// The tile `CollisionCheckOnWater` finds in `c` when a sprite is in the way: `_UpdateSprites` leaves
/// the last slot's offset there, and no tileset lists it as passable.
const UPDATE_SPRITES_LAST_OFFSET: u8 = 0xF0;
const WATER_TILE: u8 = 0x14;
/// The eastern shore, and on `SHIP_PORT` the left of the S.S. Anne's boarding platform.
const SHORE_TILE: u8 = 0x32;
const SAFARI_SHORE_TILE: u8 = 0x48;

fn table(at: DmgPointer) -> impl Iterator<Item = u8> {
    rom_slice(at).iter().copied().take_while(|&byte| byte != 0xFF)
}

/// `IsBikeRidingAllowed`: Route 23, Indigo Plateau, or a tileset in `BikeRidingTilesets`.
pub fn is_bike_riding_allowed(map: Map, tileset: TileSetId) -> bool {
    matches!(map, Map::Route23 | Map::IndigoPlateau) || table(pokered_symbols::BikeRidingTilesets).any(|t| t == tileset as u8)
}

/// `CheckForceBikeOrSurf`'s search of `ForcedBikeOrSurfMaps`: the state a square puts the player in.
pub fn forced_bike_or_surf(map: Map, x: u8, y: u8) -> Option<u8> {
    let bytes = rom_slice(pokered_symbols::ForcedBikeOrSurfMaps);
    bytes.chunks(3).take_while(|row| row[0] != 0xFF).find(|row| row[0] == map as u8 && row[1] == y && row[2] == x)
        .map(|_| if matches!(map, Map::SeafoamIslandsB3F | Map::SeafoamIslandsB4F) { SURFING } else { BIKING })
}

/// `IsNextTileShoreOrWater`, on the tilesets in `WaterTilesets`.
pub fn is_next_tile_shore_or_water(tileset: TileSetId, tile: u8) -> bool {
    if !table(pokered_symbols::WaterTilesets).any(|t| t == tileset as u8) {
        return false;
    }
    tile == WATER_TILE || tileset != TileSetId::ShipPort && (tile == SAFARI_SHORE_TILE || tile == SHORE_TILE)
}

/// `IsSurfingAllowed`: the text that refuses, on Cycling Road or at the foot of Seafoam's stairs
/// before both boulders have gone down to slow the current.
pub fn surfing_refusal(location: &Location, events: &EventFlags) -> Option<DmgPointer> {
    if location.always_on_bike {
        return Some(pokered_symbols::CyclingIsFunText);
    }
    let boulders = events.is_set(EVENT_SEAFOAM4_BOULDER1_DOWN_HOLE) && events.is_set(EVENT_SEAFOAM4_BOULDER2_DOWN_HOLE);
    // `SeafoamIslandsB4FStairsCoords`.
    let at_the_stairs = (location.x, location.y) == (7, 11);
    (location.map == Map::SeafoamIslandsB4F && !boulders && at_the_stairs).then_some(pokered_symbols::CurrentTooFastText)
}

/// What a step off the square the player surfs on does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnWater {
    Blocked,
    Swim,
    /// `.stopSurfing`: onto land, walking.
    GetOff,
}

/// `CollisionCheckOnWater` for a step the game is not simulating.
pub fn collision_check_on_water(tileset: TileSetId, standing: u8, front: u8, sprite_in_the_way: bool) -> OnWater {
    let passable = |tile| if tile_passable(tileset, tile) { OnWater::GetOff } else { OnWater::Blocked };
    if sprite_in_the_way {
        return passable(UPDATE_SPRITES_LAST_OFFSET);
    }
    if tile_pair_collision(tileset as u8, standing, front, true) {
        return OnWater::Blocked;
    }
    match front {
        WATER_TILE | SAFARI_SHORE_TILE => OnWater::Swim,
        SHORE_TILE if tileset != TileSetId::ShipPort => OnWater::Swim,
        SHORE_TILE => OnWater::GetOff,
        tile => passable(tile),
    }
}

/// `ItemUseSurfboard`'s refusal to go onto the water: `SurfingAttemptFailed`.
pub fn no_surfing_here(tileset: TileSetId, ahead: &Ahead) -> bool {
    !is_next_tile_shore_or_water(tileset, ahead.tile) || tile_pair_collision(tileset as u8, ahead.standing_on, ahead.tile, true)
}

/// `.tryToStopSurfing`'s refusal: `.cannotStopSurfing`.
pub fn no_place_to_get_off(tileset: TileSetId, ahead: &Ahead) -> bool {
    ahead.sprite || tile_pair_collision(tileset as u8, ahead.standing_on, ahead.tile, true) || !tile_passable(tileset, ahead.tile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bike_is_ridden_outside_and_in_caves_but_not_indoors() {
        assert!(is_bike_riding_allowed(Map::PalletTown, TileSetId::Overworld));
        assert!(is_bike_riding_allowed(Map::MtMoon1F, TileSetId::Cavern));
        assert!(is_bike_riding_allowed(Map::IndigoPlateau, TileSetId::Plateau));
        assert!(!is_bike_riding_allowed(Map::RedsHouse1F, TileSetId::RedsHouse1));
        assert!(!is_bike_riding_allowed(Map::ViridianPokecenter, TileSetId::Pokecenter));
    }

    #[test]
    fn cycling_road_s_gates_force_the_bike_and_seafoam_s_currents_the_water() {
        assert_eq!(forced_bike_or_surf(Map::Route16, 17, 10), Some(BIKING));
        assert_eq!(forced_bike_or_surf(Map::Route18, 33, 9), Some(BIKING));
        assert_eq!(forced_bike_or_surf(Map::SeafoamIslandsB4F, 5, 14), Some(SURFING));
        assert_eq!(forced_bike_or_surf(Map::Route16, 17, 12), None);
        assert_eq!(forced_bike_or_surf(Map::PalletTown, 17, 10), None);
    }

    #[test]
    fn water_is_swum_the_shore_is_land_and_a_wall_is_a_wall() {
        assert_eq!(collision_check_on_water(TileSetId::Overworld, WATER_TILE, WATER_TILE, false), OnWater::Swim);
        assert_eq!(collision_check_on_water(TileSetId::Overworld, WATER_TILE, SHORE_TILE, false), OnWater::Swim);
        assert_eq!(collision_check_on_water(TileSetId::ShipPort, WATER_TILE, SHORE_TILE, false), OnWater::GetOff);
        let grass = poke_core::map_gfx::tileset_entry(TileSetId::Overworld).grass_tile;
        assert_eq!(collision_check_on_water(TileSetId::Overworld, WATER_TILE, grass, false), OnWater::GetOff);
        assert_eq!(collision_check_on_water(TileSetId::Overworld, WATER_TILE, grass, true), OnWater::Blocked, "a sprite");
        assert!(!is_next_tile_shore_or_water(TileSetId::Mart, WATER_TILE));
        assert!(is_next_tile_shore_or_water(TileSetId::Overworld, SHORE_TILE));
        assert!(!is_next_tile_shore_or_water(TileSetId::ShipPort, SHORE_TILE));
    }
}
