//! `CheckForCollisionWhenPushingBoulder`: whether the square two ahead of the player will take the
//! boulder standing one ahead.

use poke_core::map_header::TileSetId;
use poke_core::sprite::SpriteFacing;
use super::collision::{tile_pair_collision, tile_passable};
use super::map_view::TileMap;
use super::sprites::Sprites;

/// `BOULDER_MOVEMENT_BYTE_2`: movement byte 2 of a sprite that Strength can shove.
pub const BOULDER_MOVEMENT_BYTE_2: u8 = 0x10;
/// The stairs tile, which no boulder may be pushed onto even though it is passable.
const STAIRS: u8 = 0x15;

/// `GetTileTwoStepsInFrontOfPlayer`, which also leaves its answer in `wTileInFrontOfPlayer`.
pub fn tile_two_steps_in_front(tiles: &TileMap, facing: SpriteFacing) -> u8 {
    let (column, row) = match facing {
        SpriteFacing::Down => (8, 13),
        SpriteFacing::Up => (8, 5),
        SpriteFacing::Left => (4, 9),
        SpriteFacing::Right => (12, 9),
    };
    tiles[row * 20 + column]
}

/// `CheckForBoulderCollisionWithSprites`: another sprite already standing where the boulder would
/// land. The boulder itself is in the list and cannot match, since it is not where it is going.
fn boulder_collision_with_sprites(sprites: &Sprites, num_sprites: u8, boulder: u8, facing: SpriteFacing) -> bool {
    let (y, x) = (sprites[boulder as usize].map_y, sprites[boulder as usize].map_x);
    let (to_y, to_x) = match facing {
        SpriteFacing::Down => (y.wrapping_add(1), x),
        SpriteFacing::Up => (y.wrapping_sub(1), x),
        SpriteFacing::Right => (y, x.wrapping_add(1)),
        SpriteFacing::Left => (y, x.wrapping_sub(1)),
    };
    (1..=num_sprites as usize).any(|slot| (sprites[slot].map_y, sprites[slot].map_x) == (to_y, to_x))
}

/// `CheckForCollisionWhenPushingBoulder`: `true` where the boulder cannot move. The tile pair check
/// is against the tile the *player* stands on, as the routine's `CheckForTilePairCollisions2` reads
/// it, rather than the boulder's own.
pub fn boulder_blocked(tileset: TileSetId, tiles: &TileMap, facing: SpriteFacing, sprites: &Sprites,
    num_sprites: u8, boulder: u8) -> bool
{
    let two_ahead = tile_two_steps_in_front(tiles, facing);
    if !tile_passable(tileset, two_ahead) {
        return true;
    }
    if tile_pair_collision(tileset as u8, tiles[9 * 20 + 8], two_ahead, false) {
        return true;
    }
    if two_ahead == STAIRS {
        return true;
    }
    boulder_collision_with_sprites(sprites, num_sprites, boulder, facing)
}
