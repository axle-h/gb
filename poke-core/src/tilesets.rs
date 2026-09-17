//! `data/tilesets/*` beyond the graphics: which tiles can be walked on, jumped from, warped through
//! or not crossed between, read from the cartridge's own tables.

use crate::map_gfx::tileset_entry;
use crate::map_header::TileSetId;
use crate::rom_gfx::rom_slice;
use crate::sprite::SpriteFacing;
use crate::symbols::{pokered_symbols, DmgBank, DmgPointer};

fn until(pointer: DmgPointer, terminator: u8) -> Vec<u8> {
    rom_slice(pointer).iter().copied().take_while(|&byte| byte != terminator).collect()
}

/// `<Tileset>_Coll`: the tiles `CheckTilePassable` lets the player onto.
pub fn collision_tiles(tileset: TileSetId) -> Vec<u8> {
    until(DmgPointer { bank: DmgBank::ROM { bank: 0 }, address: tileset_entry(tileset).coll }, 0xFF)
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
    rom_slice(pokered_symbols::LedgeTiles).chunks_exact(4)
        .take_while(|row| row[0] != 0xFF)
        .map(|row| Ledge {
            facing: SpriteFacing::from_repr(row[0]).expect("a ledge row starts with a facing"),
            standing_on: row[1],
            ledge: row[2],
            input: row[3],
        })
        .collect()
}

/// `TilePairCollisionsLand` or `TilePairCollisionsWater`: `(tileset, one, other)`, a step between
/// the two in either order being refused.
pub fn tile_pair_collisions(water: bool) -> Vec<(u8, u8, u8)> {
    let table = if water { pokered_symbols::TilePairCollisionsWater } else { pokered_symbols::TilePairCollisionsLand };
    rom_slice(table).chunks_exact(3).take_while(|row| row[0] != 0xFF).map(|row| (row[0], row[1], row[2])).collect()
}

/// `DoorTileIDPointers`: the tiles a warp is a door on, for `IsPlayerStandingOnDoorTile`.
pub fn door_tile_ids(tileset: TileSetId) -> Vec<u8> {
    let table = pokered_symbols::DoorTileIDPointers;
    rom_slice(table).chunks_exact(3)
        .take_while(|row| row[0] != 0xFF)
        .find(|row| row[0] == tileset as u8)
        .map(|row| until(DmgPointer { bank: table.bank, address: u16::from_le_bytes([row[1], row[2]]) }, 0))
        .unwrap_or_default()
}

/// `WarpTileIDPointers`: the tiles that warp the moment they are stepped on. A list with no
/// terminator of its own runs on into the next, which is the cartridge's own sharing.
pub fn warp_tile_ids(tileset: TileSetId) -> Vec<u8> {
    let table = pokered_symbols::WarpTileIDPointers;
    let row = rom_slice(table + tileset as u16 * 2);
    until(DmgPointer { bank: table.bank, address: u16::from_le_bytes([row[0], row[1]]) }, 0xFF)
}

/// `WarpTileListPointers`, for `IsWarpTileInFrontOfPlayer`: the tile ahead that lets a warp the
/// player is standing on be walked into.
pub fn warp_carpet_tile_ids(facing: SpriteFacing) -> Vec<u8> {
    let table = pokered_symbols::WarpTileListPointers;
    let row = rom_slice(table + facing as u16 / 2);
    until(DmgPointer { bank: table.bank, address: u16::from_le_bytes([row[0], row[1]]) }, 0xFF)
}

/// `DungeonTilesets`: arriving on one always reads the destination warp's position.
pub fn is_dungeon_tileset(tileset: TileSetId) -> bool {
    until(pokered_symbols::DungeonTilesets, 0xFF).contains(&(tileset as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TILESETS: std::ops::RangeInclusive<u8> = 0..=23;

    #[test]
    fn the_hand_written_warp_tables_are_the_cartridge_s() {
        for tileset in TILESETS.map(|id| TileSetId::from_repr(id).unwrap()) {
            assert_eq!(warp_tile_ids(tileset), tileset.warp_tile_ids(), "{tileset}");
        }
        use crate::sprite::PlayerFacingDirection as Player;
        for (facing, player) in [(SpriteFacing::Down, Player::Down), (SpriteFacing::Up, Player::Up),
                                 (SpriteFacing::Left, Player::Left), (SpriteFacing::Right, Player::Right)] {
            assert_eq!(warp_carpet_tile_ids(facing), TileSetId::warp_carpet_tile_ids(player), "{facing}");
        }
    }

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
