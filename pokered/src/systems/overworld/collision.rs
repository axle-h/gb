//! The player's collision rules, each a pure function of the tiles and the map: the checks
//! `CollisionCheckOnLand` strings together, the ledge table, and the two warp tests.

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_symbols;
use poke_core::tilesets::{collision_tiles, door_tile_ids, ledge_tiles, warp_carpet_tile_ids, warp_tile_ids};
use serde::{Deserialize, Serialize};
use super::map_view::TileMap;

/// `CheckTilePassable`.
pub fn tile_passable(tileset: TileSetId, tile: u8) -> bool {
    collision_tiles(tileset).contains(&tile)
}

/// `CheckForTilePairCollisions`, walking the table's bytes as the routine does. A row whose first
/// tile matches and whose second does not leaves the pointer on that second tile, which the next
/// pass then reads as a tileset: the table is parsed out of step from there on.
pub fn tile_pair_collision(tileset: u8, standing: u8, front: u8, water: bool) -> bool {
    let table = rom_slice(if water { pokered_symbols::TilePairCollisionsWater } else { pokered_symbols::TilePairCollisionsLand });
    let mut hl = 0;
    loop {
        let a = table[hl];
        hl += 1;
        if a == 0xFF {
            return false;
        }
        if a != tileset {
            hl += 2;
            continue;
        }
        if table[hl] == standing {
            hl += 1;
            if table[hl] == front {
                return true;
            }
            continue;
        }
        hl += 1;
        if table[hl] == standing {
            let first = table[hl - 1];
            hl += 1;
            if first == front {
                return true;
            }
            continue;
        }
        hl += 1;
    }
}

/// `HandleLedges`' match: the buttons that jump from `standing` over `front` facing that way, on
/// the one tileset that has ledges.
pub fn ledge_input(tileset: TileSetId, facing: SpriteFacing, standing: u8, front: u8) -> Option<u8> {
    if tileset != TileSetId::Overworld {
        return None;
    }
    ledge_tiles().into_iter()
        .find(|ledge| ledge.facing == facing && ledge.standing_on == standing && ledge.ledge == front)
        .map(|ledge| ledge.input)
}

/// `_GetTileAndCoordsInFrontOfPlayer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFront {
    pub tile: u8,
    pub x: u8,
    pub y: u8,
}

pub fn in_front(tiles: &TileMap, x: u8, y: u8, facing: SpriteFacing) -> InFront {
    let at = |column: usize, row: usize| tiles[row * 20 + column];
    match facing {
        SpriteFacing::Down => InFront { tile: at(8, 11), x, y: y.wrapping_add(1) },
        SpriteFacing::Up => InFront { tile: at(8, 7), x, y: y.wrapping_sub(1) },
        SpriteFacing::Left => InFront { tile: at(6, 9), x: x.wrapping_sub(1), y },
        SpriteFacing::Right => InFront { tile: at(10, 9), x: x.wrapping_add(1), y },
    }
}

/// `IsPlayerStandingOnDoorTile`, from the lower left tile of the player's square.
pub fn on_door_tile(tileset: TileSetId, standing: u8) -> bool {
    door_tile_ids(tileset).contains(&standing)
}

/// `IsPlayerStandingOnDoorTileOrWarpTile`: whether the warp takes the player at once, and whether
/// it was a warp tile rather than a door, which clears `BIT_STANDING_ON_WARP`.
pub fn on_door_or_warp_tile(tileset: TileSetId, standing: u8) -> (bool, bool) {
    if on_door_tile(tileset, standing) {
        return (true, false);
    }
    let warp = warp_tile_ids(tileset).contains(&standing);
    (warp, warp)
}

/// `CheckIfInOutsideMap`.
pub fn is_outside(tileset: TileSetId) -> bool {
    matches!(tileset, TileSetId::Overworld | TileSetId::Plateau)
}

/// What `ExtraWarpCheck` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraWarp {
    pub map: u8,
    pub tileset: TileSetId,
    pub facing: SpriteFacing,
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
    pub front: u8,
}

/// `ExtraWarpCheck`: a few maps and the outdoor tilesets let a warp be walked into only onto a
/// carpet tile (`IsWarpTileInFrontOfPlayer`); everywhere else only off the edge of the map
/// (`IsPlayerFacingEdgeOfMap`).
pub fn extra_warp_check(check: ExtraWarp) -> bool {
    let map = check.map;
    let reads_the_tile = map != Map::SSAnne3F as u8
        && ([Map::RocketHideoutB1F, Map::RocketHideoutB2F, Map::RocketHideoutB4F, Map::RockTunnel1F]
                .iter().any(|&m| m as u8 == map)
            || matches!(check.tileset, TileSetId::Overworld | TileSetId::Ship | TileSetId::ShipPort | TileSetId::Plateau));
    if reads_the_tile {
        if map == Map::SSAnneBow as u8 {
            return check.front == 0x15;
        }
        return warp_carpet_tile_ids(check.facing).contains(&check.front);
    }
    match check.facing {
        SpriteFacing::Down => check.y == (check.height.wrapping_mul(2)).wrapping_sub(1),
        SpriteFacing::Up => check.y == 0,
        SpriteFacing::Left => check.x == 0,
        SpriteFacing::Right => check.x == (check.width.wrapping_mul(2)).wrapping_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::cases;

    #[test]
    fn tile_pairs_match_the_cartridge() {
        for ((tileset, standing, front, water), collides, _) in cases::<(u8, u8, u8, bool), bool>(include_str!("../../../fixtures/overworld/tile_pair_collisions.jsonl")) {
            assert_eq!(tile_pair_collision(tileset, standing, front, water), collides, "{tileset} ${standing:02X} ${front:02X} {water}");
        }
    }

    #[test]
    fn ledges_match_the_cartridge() {
        for ((tileset, facing, standing, front, held), jumps, _) in cases::<(u8, u8, u8, u8, u8), bool>(include_str!("../../../fixtures/overworld/handle_ledges.jsonl")) {
            let ours = ledge_input(TileSetId::from_repr(tileset).unwrap(), SpriteFacing::from_repr(facing).unwrap(), standing, front)
                .is_some_and(|input| input & held != 0);
            assert_eq!(ours, jumps, "{tileset} {facing} ${standing:02X} ${front:02X} {held:08b}");
        }
    }

    #[test]
    fn door_and_warp_tiles_match_the_cartridge() {
        for ((tileset, standing), (warps, kept), _) in cases::<(u8, u8), (bool, bool)>(include_str!("../../../fixtures/overworld/door_or_warp_tile.jsonl")) {
            let (ours, clears) = on_door_or_warp_tile(TileSetId::from_repr(tileset).unwrap(), standing);
            assert_eq!((ours, !clears), (warps, kept), "{tileset} ${standing:02X}");
        }
    }

    #[test]
    fn extra_warp_checks_match_the_cartridge() {
        for (check, passes, _) in cases::<ExtraWarp, bool>(include_str!("../../../fixtures/overworld/extra_warp_check.jsonl")) {
            assert_eq!(extra_warp_check(check), passes, "{check:?}");
        }
    }
}
