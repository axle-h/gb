//! `data/tilesets/*` beyond the graphics: which tiles can be walked on, jumped from, warped through
//! or not crossed between.

use crate::map_header::TileSetId;
use crate::sprite::SpriteFacing;
use crate::tables;

/// `<Tileset>_Coll`: the tiles `CheckTilePassable` lets the player onto.
pub fn collision_tiles(tileset: TileSetId) -> Vec<u8> {
    tables::TILESETS[tileset as usize].collision.to_vec()
}

/// A row of `LedgeTiles`: facing this way, from this tile onto that one, with this button held,
/// the player jumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ledge {
    pub facing: SpriteFacing,
    pub standing_on: u8,
    pub ledge: u8,
    pub input: u8,
}

pub fn ledge_tiles() -> Vec<Ledge> {
    tables::LEDGE_TILES.iter()
        .map(|&(facing, standing_on, ledge, input)| Ledge {
            facing: SpriteFacing::from_repr(facing).expect("a ledge row starts with a facing"),
            standing_on,
            ledge,
            input,
        })
        .collect()
}

/// `TilePairCollisionsLand` or `TilePairCollisionsWater`: `(tileset, one, other)`, a step between
/// the two in either order being refused.
pub fn tile_pair_collisions(water: bool) -> &'static [(u8, u8, u8)] {
    if water { tables::TILE_PAIR_COLLISIONS_WATER } else { tables::TILE_PAIR_COLLISIONS_LAND }
}

/// `DoorTileIDPointers`: the tiles a warp is a door on, for `IsPlayerStandingOnDoorTile`.
pub fn door_tile_ids(tileset: TileSetId) -> Vec<u8> {
    tables::DOOR_TILE_IDS.iter().find(|(id, _)| *id == tileset as u8).map(|(_, tiles)| tiles.to_vec()).unwrap_or_default()
}

/// `WarpTileIDPointers`: the tiles that warp the moment they are stepped on. A list with no
/// terminator of its own runs on into the next, which is the cartridge's own sharing.
pub fn warp_tile_ids(tileset: TileSetId) -> Vec<u8> {
    tables::WARP_TILE_IDS[tileset as usize].to_vec()
}

/// `WarpTileListPointers`, for `IsWarpTileInFrontOfPlayer`: the tile ahead that lets a warp the
/// player is standing on be walked into.
pub fn warp_carpet_tile_ids(facing: SpriteFacing) -> Vec<u8> {
    tables::WARP_CARPET_TILE_IDS[facing as usize / 4].to_vec()
}

/// `DungeonTilesets`: arriving on one always reads the destination warp's position.
pub fn is_dungeon_tileset(tileset: TileSetId) -> bool {
    tables::DUNGEON_TILESETS.contains(&(tileset as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overworld_s_tables_read_as_their_files_do() {
        assert_eq!(collision_tiles(TileSetId::Overworld)[..5], [0x00, 0x10, 0x1B, 0x20, 0x21]);
        assert_eq!(ledge_tiles().len(), 8);
        assert_eq!(ledge_tiles()[0], Ledge { facing: SpriteFacing::Down, standing_on: 0x2C, ledge: 0x37, input: 0x80 });
        assert_eq!(tile_pair_collisions(false).len(), 11);
        assert_eq!(door_tile_ids(TileSetId::Overworld), [0x1B, 0x58]);
        assert_eq!(door_tile_ids(TileSetId::RedsHouse1), Vec::<u8>::new());
        assert!(is_dungeon_tileset(TileSetId::Gym) && !is_dungeon_tileset(TileSetId::Overworld));
    }
}
