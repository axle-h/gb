//! The overworld's pure routines: collision, ledges, the two warp tests, the view and the tile in
//! front of the player, sprites in each other's way, an NPC's move, and which VRAM slot each
//! map's sprites draw from.

use gb::ram::RAM;
use poke_core::map::Map;
use poke_core::map_header::{MapHeader, TileSetId};
use poke_core::map_objects::MapObjects;
use poke_core::sprite::SpriteFacing;
use pokered::rng::GameRng;
use pokered::systems::map_data::tile_block_map;
use pokered::systems::overworld::collision::{self, ExtraWarp};
use pokered::systems::overworld::map_view::MapView;
use pokered::systems::overworld::sprites::{self, SpriteEnv, SpriteSet, SpriteState, Sprites};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgBank, DmgPointer};
use super::Oracle;

const FACINGS: [SpriteFacing; 4] = [SpriteFacing::Down, SpriteFacing::Up, SpriteFacing::Left, SpriteFacing::Right];
const TILE_MAP_BYTES: usize = 360;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/pallet-town-state.bin"))
}

fn byte(oracle: &Oracle, at: DmgPointer) -> u8 {
    oracle.read(at, 1)[0]
}

fn wram(address: u16) -> DmgPointer {
    DmgPointer { bank: DmgBank::WRAM, address }
}

/// The cells of `wTileMap` `_GetTileAndCoordsInFrontOfPlayer` reads for each facing.
fn front_cell(facing: SpriteFacing) -> u16 {
    match facing {
        SpriteFacing::Down => 11 * 20 + 8,
        SpriteFacing::Up => 7 * 20 + 8,
        SpriteFacing::Left => 9 * 20 + 6,
        SpriteFacing::Right => 9 * 20 + 10,
    }
}

/// `CheckForTilePairCollisions` for `(tileset, standing, front, water)`.
fn tile_pair_collision(oracle: &mut Oracle, (tileset, standing, front, water): (u8, u8, u8, bool)) -> bool {
    oracle.write(sym::wCurMapTileset, &[tileset]);
    oracle.write(sym::wTilePlayerStandingOn, &[standing]);
    oracle.write(sym::wTileInFrontOfPlayer, &[front]);
    let table = if water { sym::TilePairCollisionsWater } else { sym::TilePairCollisionsLand };
    oracle.registers_mut().set_hl(table.address);
    oracle.call(sym::CheckForTilePairCollisions);
    oracle.registers().flags.c
}

/// `HandleLedges` for `(tileset, facing, standing, front, held)`: whether it armed a jump, stopped
/// where it would load the shadow, which waits for frames.
fn handle_ledges(oracle: &mut Oracle, (tileset, facing, standing, front, held): (u8, u8, u8, u8, u8)) -> bool {
    oracle.write(sym::wMovementFlags, &[0]);
    oracle.write(sym::wCurMapTileset, &[tileset]);
    oracle.write(wram(sym::wSpriteStateData1.address + 9), &[facing]);
    oracle.write(sym::wTileMap, &[0; TILE_MAP_BYTES]);
    oracle.write(wram(sym::wTileMap.address + 9 * 20 + 8), &[standing]);
    let facing = SpriteFacing::from_repr(facing).unwrap();
    oracle.write(wram(sym::wTileMap.address + front_cell(facing)), &[front]);
    oracle.write(sym::hJoyHeld, &[held]);
    let (_, stop) = oracle.call_until(sym::HandleLedges, &[sym::LoadHoppingShadowOAM]);
    stop.is_some()
}

/// `IsPlayerStandingOnDoorTileOrWarpTile` for `(tileset, standing)`: its carry, and whether it left
/// `BIT_STANDING_ON_WARP` set.
fn door_or_warp_tile(oracle: &mut Oracle, (tileset, standing): (u8, u8)) -> (bool, bool) {
    oracle.write(sym::wMovementFlags, &[1 << 2]);
    oracle.write(sym::wCurMapTileset, &[tileset]);
    oracle.write(wram(sym::wTileMap.address + 9 * 20 + 8), &[standing]);
    oracle.call(sym::IsPlayerStandingOnDoorTileOrWarpTile);
    (oracle.registers().flags.c, byte(oracle, sym::wMovementFlags) & 1 << 2 != 0)
}

/// `ExtraWarpCheck`, with the tile in front written where any facing reads it.
fn extra_warp_check(oracle: &mut Oracle, check: ExtraWarp) -> bool {
    oracle.write(sym::wCurMap, &[check.map]);
    oracle.write(sym::wCurMapTileset, &[check.tileset as u8]);
    oracle.write(wram(sym::wSpriteStateData1.address + 9), &[check.facing as u8]);
    oracle.write(sym::wYCoord, &[check.y]);
    oracle.write(sym::wXCoord, &[check.x]);
    oracle.write(sym::wCurMapHeight, &[check.height]);
    oracle.write(sym::wCurMapWidth, &[check.width]);
    for facing in FACINGS {
        oracle.write(wram(sym::wTileMap.address + front_cell(facing)), &[check.front]);
    }
    oracle.call(sym::ExtraWarpCheck);
    oracle.registers().flags.c
}

/// The map loaded and the view on `(x, y)` built as a warp onto that square builds it, then
/// `_GetTileAndCoordsInFrontOfPlayer`: `[tile, x, y, standing]`.
fn tile_in_front(oracle: &mut Oracle, (map, x, y, facing): (u8, u8, u8, u8)) -> [u8; 4] {
    let header = MapHeader::read(Map::from_repr(map).unwrap()).unwrap();
    oracle.write(sym::wCurMap, &[map]);
    oracle.write(sym::wCurMapTileset, &[0]);
    oracle.write(sym::wDestinationWarpID, &[0xFF]);
    oracle.write(sym::wStatusFlags4, &[0]);
    oracle.call(sym::LoadMapHeader);
    let width = header.width as u16;
    let view = sym::wOverworldMap.address + 7 + width + (width + 6) * (y >> 1) as u16 + (x >> 1) as u16;
    oracle.write(sym::wCurrentTileBlockMapViewPointer, &view.to_le_bytes());
    oracle.write(sym::wYCoord, &[y]);
    oracle.write(sym::wXCoord, &[x]);
    oracle.write(sym::wYBlockCoord, &[y & 1, x & 1]);
    oracle.call(sym::LoadTileBlockMap);
    oracle.call(sym::LoadCurrentMapView);
    oracle.write(wram(sym::wSpriteStateData1.address + 9), &[facing]);
    oracle.call(sym::_GetTileAndCoordsInFrontOfPlayer);
    let registers = oracle.registers();
    [registers.c, registers.e, registers.d, byte(oracle, wram(sym::wTileMap.address + 9 * 20 + 8))]
}

/// Sixteen slots of `wSpriteStateData1`, `wSpriteStateData2` and `wMapSpriteData`, written.
fn write_sprites(oracle: &mut Oracle, sprites: &Sprites) {
    for (slot, sprite) in sprites.iter().enumerate() {
        let at = slot as u16 * 16;
        oracle.write(wram(sym::wSpriteStateData1.address + at), &sprite.data1());
        oracle.write(wram(sym::wSpriteStateData2.address + at), &sprite.data2());
        if slot > 0 {
            oracle.write(wram(sym::wMapSpriteData.address + (slot as u16 - 1) * 2), &[sprite.movement2, sprite.text_id]);
        }
    }
}

fn read_sprite(oracle: &Oracle, slot: usize) -> SpriteState {
    let at = slot as u16 * 16;
    let map_data = if slot == 0 { [0, 0] } else {
        let data = oracle.read(wram(sym::wMapSpriteData.address + (slot as u16 - 1) * 2), 2);
        [data[0], data[1]]
    };
    SpriteState::from_bytes(&oracle.read(wram(sym::wSpriteStateData1.address + at), 16),
        &oracle.read(wram(sym::wSpriteStateData2.address + at), 16), map_data)
}

/// A slot as fixture text: both data blocks and the map data, in hex.
pub fn sprite_hex(sprite: &SpriteState) -> String {
    sprite.data1().iter().chain(&sprite.data2()).chain(&[sprite.movement2, sprite.text_id]).map(|b| format!("{b:02x}")).collect()
}

/// `DetectCollisionBetweenSprites` for slot `i`: slot `i` as it leaves it.
fn detect_collision(oracle: &mut Oracle, (i, sprites): (u8, Vec<SpriteState>)) -> SpriteState {
    let sprites: Sprites = sprites.try_into().unwrap();
    write_sprites(oracle, &sprites);
    oracle.write(sym::hCurrentSpriteOffset, &[i << 4]);
    oracle.call(sym::DetectCollisionBetweenSprites);
    read_sprite(oracle, i as usize)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What `UpdateNPCSprite` is given: the screen's tiles, the slots and the player.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NpcInput {
    pub tiles: Vec<u8>,
    pub sprites: Vec<SpriteState>,
    pub x: u8,
    pub y: u8,
    pub walk_counter: u8,
    pub player_direction: u8,
}

/// `UpdateNPCSprite` for slot 1 on the overworld tileset: the slots as it leaves them, and the
/// `Random` bytes it took.
fn update_npc_sprite(oracle: &mut Oracle, input: &NpcInput) -> (Vec<SpriteState>, Vec<u8>) {
    let sprites: Sprites = input.sprites.clone().try_into().unwrap();
    write_sprites(oracle, &sprites);
    oracle.write(sym::wTileMap, &input.tiles);
    oracle.write(sym::wXCoord, &[input.x]);
    oracle.write(sym::wYCoord, &[input.y]);
    oracle.write(sym::wWalkCounter, &[input.walk_counter]);
    oracle.write(sym::wPlayerDirection, &[input.player_direction]);
    oracle.write(sym::wFontLoaded, &[0]);
    oracle.write(sym::wStatusFlags3, &[0]);
    oracle.write(sym::wToggleableObjectList, &[0xFF]);
    oracle.write(sym::wNPCMovementScriptSpriteOffset, &[0]);
    oracle.write(sym::wTilesetCollisionPtr, &sym::Overworld_Coll.address.to_le_bytes());
    oracle.write(sym::wGrassTile, &[0x52]);
    oracle.write(sym::hCurrentSpriteOffset, &[0x10]);
    oracle.write(sym::hTilePlayerStandingOn, &[sprites[1].image_base_offset.wrapping_sub(1).rotate_left(4)]);
    let called = oracle.call(sym::UpdateNPCSprite);
    ((0..3).map(|slot| read_sprite(oracle, slot)).collect(), called.rng)
}

/// `LoadMapHeader` then `InitMapSprites` for `(map, x, y)`: each slot's VRAM slot and the sprite
/// set chosen.
fn init_map_sprites(oracle: &mut Oracle, (map, x, y): (u8, u8, u8)) -> (Vec<u8>, u8) {
    oracle.write(sym::wCurMap, &[map]);
    oracle.write(sym::wCurMapTileset, &[0]);
    oracle.write(sym::wDestinationWarpID, &[0xFF]);
    oracle.write(sym::wStatusFlags4, &[0]);
    oracle.call(sym::LoadMapHeader);
    oracle.write(sym::wYCoord, &[y]);
    oracle.write(sym::wXCoord, &[x]);
    oracle.write(sym::wFontLoaded, &[0]);
    oracle.write(sym::wSpriteSetID, &[0]);
    oracle.write(wram(sym::wSpriteStateData1.address), &[1]);
    oracle.call(sym::InitMapSprites);
    let bases = (0..16u16).map(|slot| byte(oracle, wram(sym::wSpriteStateData2.address + slot * 16 + 14))).collect();
    (bases, byte(oracle, sym::wSpriteSetID))
}

fn maps() -> Vec<Map> {
    Map::all().filter(|map| map.header_pointer().is_some()).collect()
}

#[cfg(feature = "slow-tests")]
mod inputs {
    use rand::{RngExt, SeedableRng};
    use rand::rngs::StdRng;
    use super::*;

    pub fn rng(seed: u64) -> StdRng {
        StdRng::seed_from_u64(seed)
    }

    pub fn tile_pairs() -> Vec<(u8, u8, u8, bool)> {
        let mut rng = rng(0x7A1);
        let mut tiles: Vec<u8> = collision_tiles_in_pairs();
        tiles.extend([0x00, 0x01, 0x13, 0x33]);
        (0..600).map(|_| {
            let tileset = [3, 17, 0, 3, 17][rng.random_range(0..5)];
            (tileset, tiles[rng.random_range(0..tiles.len())], tiles[rng.random_range(0..tiles.len())], rng.random_bool(0.3))
        }).collect()
    }

    fn collision_tiles_in_pairs() -> Vec<u8> {
        let mut tiles: Vec<u8> = [false, true].iter()
            .flat_map(|&water| poke_core::tilesets::tile_pair_collisions(water))
            .flat_map(|(_, a, b)| [a, b])
            .collect();
        tiles.sort();
        tiles.dedup();
        tiles
    }

    pub fn ledges() -> Vec<(u8, u8, u8, u8, u8)> {
        let mut rng = rng(0x1ED6E);
        let mut tiles = vec![0x2C, 0x39, 0x37, 0x36, 0x27, 0x0D, 0x1D, 0x00];
        tiles.extend(poke_core::tilesets::ledge_tiles().iter().flat_map(|l| [l.standing_on, l.ledge]));
        (0..400).map(|_| {
            let tileset = if rng.random_bool(0.85) { 0 } else { rng.random_range(1..24) };
            let held = [0x80, 0x40, 0x20, 0x10, 0x01, 0x00, 0xF0][rng.random_range(0..7)];
            let rows = poke_core::tilesets::ledge_tiles();
            if rng.random_bool(0.5) {
                // A row of the table, one part of it sometimes wrong.
                let row = rows[rng.random_range(0..rows.len())];
                let facing = if rng.random_bool(0.8) { row.facing as u8 } else { FACINGS[rng.random_range(0..4)] as u8 };
                let standing = if rng.random_bool(0.9) { row.standing_on } else { tiles[rng.random_range(0..tiles.len())] };
                let held = if rng.random_bool(0.7) { row.input } else { held };
                return (tileset, facing, standing, row.ledge, held);
            }
            let facing = FACINGS[rng.random_range(0..4)] as u8;
            (tileset, facing, tiles[rng.random_range(0..tiles.len())], tiles[rng.random_range(0..tiles.len())], held)
        }).collect()
    }

    pub fn door_or_warp_tiles() -> Vec<(u8, u8)> {
        let mut rng = rng(0xD00);
        (0..24u8).flat_map(|tileset| {
            let id = TileSetId::from_repr(tileset).unwrap();
            let mut tiles = poke_core::tilesets::door_tile_ids(id);
            tiles.extend(poke_core::tilesets::warp_tile_ids(id));
            tiles.extend((0..8).map(|_| rng.random_range(0..0x60u8)));
            tiles.into_iter().map(move |tile| (tileset, tile))
        }).collect()
    }

    pub fn extra_warps() -> Vec<ExtraWarp> {
        let mut rng = rng(0xE87A);
        let maps = maps();
        let special = [Map::SSAnne3F, Map::SSAnneBow, Map::RocketHideoutB1F, Map::RockTunnel1F];
        (0..500).map(|_| {
            let map = if rng.random_bool(0.2) { special[rng.random_range(0..special.len())] } else { maps[rng.random_range(0..maps.len())] };
            let header = MapHeader::read(map).unwrap();
            let facing = FACINGS[rng.random_range(0..4)];
            let pick = |rng: &mut StdRng, size: u8| match rng.random_range(0..3) {
                0 => 0,
                1 => size.wrapping_mul(2).wrapping_sub(1),
                _ => rng.random_range(0..size.wrapping_mul(2).max(1)),
            };
            let carpets = poke_core::tilesets::warp_carpet_tile_ids(facing);
            let front = if rng.random_bool(0.5) { carpets[rng.random_range(0..carpets.len())] } else { rng.random_range(0..0x60) };
            ExtraWarp {
                map: map as u8,
                tileset: header.tileset,
                facing,
                x: pick(&mut rng, header.width),
                y: pick(&mut rng, header.height),
                width: header.width,
                height: header.height,
                front,
            }
        }).collect()
    }

    pub fn squares() -> Vec<(u8, u8, u8, u8)> {
        let mut rng = rng(0x5C0A);
        let maps = maps();
        (0..400).map(|_| {
            let map = maps[rng.random_range(0..maps.len())];
            let header = MapHeader::read(map).unwrap();
            (map as u8, rng.random_range(0..header.width * 2), rng.random_range(0..header.height * 2), FACINGS[rng.random_range(0..4)] as u8)
        }).collect()
    }

    fn walker(rng: &mut StdRng) -> SpriteState {
        let aligned = |rng: &mut StdRng, top: u8| rng.random_range(0..top) * 16;
        let mid_step = rng.random_bool(0.3);
        let (y_step, x_step) = if mid_step {
            [(1, 0), (0xFF, 0), (0, 1), (0, 0xFF)][rng.random_range(0..4)]
        } else {
            (0, 0)
        };
        let offset = if mid_step { rng.random_range(0..16u8) } else { 0 };
        SpriteState {
            picture_id: 4,
            image_index: if rng.random_bool(0.1) { 0xFF } else { 0x10 },
            y_step,
            x_step,
            y_pixels: aligned(rng, 9).wrapping_sub(4).wrapping_add(offset * y_step),
            x_pixels: aligned(rng, 10).wrapping_add(offset * x_step),
            ..SpriteState::default()
        }
    }

    pub fn collisions() -> Vec<(u8, Vec<SpriteState>)> {
        let mut rng = rng(0xC011);
        (0..250).map(|_| {
            let mut sprites = [SpriteState::default(); 16];
            sprites[0] = SpriteState { picture_id: 1, image_index: 0, y_pixels: 0x3C, x_pixels: 0x40, ..walker(&mut rng) };
            for slot in 1..rng.random_range(2..16) {
                if rng.random_bool(0.8) {
                    sprites[slot] = walker(&mut rng);
                }
            }
            // Crowd the player so there is something to find.
            if rng.random_bool(0.7) {
                let near = rng.random_range(1..4);
                let (dy, dx) = [(16, 0), (0u8.wrapping_sub(16), 0), (0, 16), (0, 0u8.wrapping_sub(16)), (8, 0), (0, 9)][rng.random_range(0..6)];
                sprites[near] = SpriteState { y_pixels: 0x3Cu8.wrapping_add(dy), x_pixels: 0x40u8.wrapping_add(dx), ..walker(&mut rng) };
            }
            (rng.random_range(0..4), sprites.to_vec())
        }).collect()
    }

    pub fn npcs() -> Vec<NpcInput> {
        let mut rng = rng(0x9C);
        let passable = poke_core::tilesets::collision_tiles(TileSetId::Overworld);
        (0..300).map(|_| {
            let tiles = (0..TILE_MAP_BYTES).map(|_| match rng.random_range(0..20) {
                0..=13 => passable[rng.random_range(0..passable.len())],
                14..=18 => [0x01, 0x0B, 0x37, 0x3D][rng.random_range(0..4)],
                _ => rng.random_range(0x60..=0xFF),
            }).collect();
            let (x, y) = (rng.random_range(8..30u8), rng.random_range(8..30u8));
            let mut sprites = [SpriteState::default(); 16];
            sprites[0] = SpriteState { picture_id: 1, image_base_offset: 1, y_pixels: 0x3C, x_pixels: 0x40, ..SpriteState::default() };
            let status = [0, 1, 1, 1, 2, 2, 3, 3, 0x81][rng.random_range(0..9)];
            let map_y = y.wrapping_add(4).wrapping_add(rng.random_range(0..12)).wrapping_sub(5);
            let map_x = x.wrapping_add(4).wrapping_add(rng.random_range(0..14)).wrapping_sub(6);
            let walking = status == 3;
            let step = [(0u8, 0u8), (1, 0), (0xFF, 0), (0, 1), (0, 0xFF)][if walking { rng.random_range(1..5) } else { 0 }];
            sprites[1] = SpriteState {
                picture_id: 4,
                movement_status: status,
                image_index: 0x10,
                y_step: step.0,
                x_step: step.1,
                y_pixels: map_y.wrapping_sub(y).rotate_left(4).wrapping_sub(4),
                x_pixels: map_x.wrapping_sub(x).rotate_left(4),
                intra_anim_frame_counter: rng.random_range(0..4),
                anim_frame_counter: rng.random_range(0..4),
                facing: [0, 4, 8, 12][rng.random_range(0..4)],
                walk_animation_counter: if walking { rng.random_range(1..=16) } else { 0 },
                y_displacement: rng.random_range(0..12),
                x_displacement: rng.random_range(0..12),
                map_y,
                map_x,
                movement1: [0xFE, 0xFE, 0xFE, 0xFF][rng.random_range(0..4)],
                movement_delay: rng.random_range(0..4),
                image_base_offset: rng.random_range(2..11),
                movement2: [0x00, 0x01, 0x02, 0xD0, 0xD1, 0xD2, 0xD3, 0xFF][rng.random_range(0..8)],
                text_id: 1,
                ..SpriteState::default()
            };
            if rng.random_bool(0.5) {
                sprites[2] = SpriteState {
                    picture_id: 5,
                    movement_status: 1,
                    image_index: 0x20,
                    y_pixels: sprites[1].y_pixels.wrapping_add([16, 0, 0u8.wrapping_sub(16), 0][rng.random_range(0..4)]),
                    x_pixels: sprites[1].x_pixels.wrapping_add([0, 16, 0, 0u8.wrapping_sub(16)][rng.random_range(0..4)]),
                    image_base_offset: 3,
                    movement1: 0xFF,
                    movement2: 0xFF,
                    text_id: 2,
                    ..SpriteState::default()
                };
            }
            NpcInput { tiles, sprites: sprites.to_vec(), x, y, walk_counter: if rng.random_bool(0.1) { 3 } else { 0 }, player_direction: [1, 2, 4, 8][rng.random_range(0..4)] }
        }).collect()
    }

    pub fn map_sprites() -> Vec<(u8, u8, u8)> {
        let mut rng = rng(0x5E75);
        maps().into_iter().map(|map| {
            let header = MapHeader::read(map).unwrap();
            (map as u8, rng.random_range(0..header.width * 2), rng.random_range(0..header.height * 2))
        }).collect()
    }
}

/// The recreation's side of each routine, beside the cartridge's.
fn recreated_npc(input: &NpcInput, rng: &[u8]) -> Vec<SpriteState> {
    let mut sprites: Sprites = input.sprites.clone().try_into().unwrap();
    let tiles: [u8; TILE_MAP_BYTES] = input.tiles.clone().try_into().unwrap();
    let collision = poke_core::tilesets::collision_tiles(TileSetId::Overworld);
    let env = SpriteEnv {
        tiles: &tiles,
        x: input.x,
        y: input.y,
        walk_counter: input.walk_counter,
        font_loaded: false,
        collision: &collision,
        grass_tile: 0x52,
        hidden: [false; 16],
        no_face_player: false,
        player_direction: input.player_direction,
        moving_direction: 0,
        spinning: false,
        simulating: false,
        beyond: None,
    };
    sprites::update_npc_sprite(&mut sprites, 1, &env, &mut sprites::NpcPaths::default(), &mut GameRng::tape(rng.to_vec()));
    sprites[..3].to_vec()
}

fn recreated_view(map: u8, x: u8, y: u8) -> MapView {
    let map = Map::from_repr(map).unwrap();
    let header = MapHeader::read(map).unwrap();
    let width = header.width as u16;
    MapView {
        tileset: header.tileset,
        width: header.width,
        height: header.height,
        blocks: tile_block_map(map).unwrap(),
        view: 7 + width + (width + 6) * (y >> 1) as u16 + (x >> 1) as u16,
        x_block: x & 1,
        y_block: y & 1,
    }
}

fn recreated_map_sprites(map: u8, x: u8, y: u8) -> (Vec<u8>, u8) {
    let map = Map::from_repr(map).unwrap();
    let objects = MapObjects::read(map).unwrap();
    let mut sprites = [SpriteState::default(); 16];
    sprites[0].picture_id = 1;
    sprites[0].image_base_offset = 1;
    for (slot, object) in objects.objects.iter().enumerate() {
        sprites[slot + 1].picture_id = object.picture;
    }
    let mut set = SpriteSet::default();
    let mut tiles = pokered::gfx::tiles::TileData::default();
    sprites::init_map_sprites(&mut sprites, &mut set, map, x, y, objects.objects.len() as u8, false, &mut tiles);
    (sprites.iter().map(|s| s.image_base_offset).collect(), set.id)
}

/// The oracle's own checks, on cases with a known answer.
#[test]
fn a_ledge_is_jumped_only_facing_it_with_the_button_held() {
    let mut oracle = oracle();
    assert!(handle_ledges(&mut oracle, (0, SpriteFacing::Down as u8, 0x2C, 0x37, 0x80)));
    assert!(!handle_ledges(&mut oracle, (0, SpriteFacing::Down as u8, 0x2C, 0x37, 0x00)));
    assert!(!handle_ledges(&mut oracle, (0, SpriteFacing::Up as u8, 0x2C, 0x37, 0x80)));
}

#[test]
fn red_s_door_in_pallet_town_is_below_the_player_s_view() {
    let mut oracle = oracle();
    let [tile, x, y, _] = tile_in_front(&mut oracle, (Map::PalletTown as u8, 5, 6, SpriteFacing::Up as u8));
    assert_eq!((x, y), (5, 5));
    assert!(poke_core::tilesets::door_tile_ids(TileSetId::Overworld).contains(&tile), "${tile:02X} is not a door");
}

#[test]
#[cfg(feature = "slow-tests")]
fn the_port_matches_the_cartridge() {
    let mut oracle = oracle();
    for input in inputs::tile_pairs() {
        assert_eq!(collision::tile_pair_collision(input.0, input.1, input.2, input.3), tile_pair_collision(&mut oracle, input), "{input:?}");
    }
    for input in inputs::ledges() {
        let (tileset, facing, standing, front, held) = input;
        let ours = collision::ledge_input(TileSetId::from_repr(tileset).unwrap(), SpriteFacing::from_repr(facing).unwrap(), standing, front)
            .is_some_and(|needed| needed & held != 0);
        assert_eq!(ours, handle_ledges(&mut oracle, input), "{input:?}");
    }
    for input in inputs::door_or_warp_tiles() {
        let (warps, clears) = collision::on_door_or_warp_tile(TileSetId::from_repr(input.0).unwrap(), input.1);
        assert_eq!((warps, !clears), door_or_warp_tile(&mut oracle, input), "{input:?}");
    }
    for input in inputs::extra_warps() {
        assert_eq!(collision::extra_warp_check(input), extra_warp_check(&mut oracle, input), "{input:?}");
    }
    for input in inputs::squares() {
        let (map, x, y, facing) = input;
        let view = recreated_view(map, x, y);
        let tiles = view.tile_map();
        let front = collision::in_front(&tiles, x, y, SpriteFacing::from_repr(facing).unwrap());
        assert_eq!([front.tile, front.x, front.y, tiles[9 * 20 + 8]], tile_in_front(&mut oracle, input), "{input:?}");
    }
    for input in inputs::collisions() {
        let mut sprites: Sprites = input.1.clone().try_into().unwrap();
        sprites::detect_collision_between_sprites(&mut sprites, input.0 as usize);
        assert_eq!(sprites[input.0 as usize], detect_collision(&mut oracle, input.clone()), "slot {}", input.0);
    }
    for input in inputs::npcs() {
        let (theirs, rng) = update_npc_sprite(&mut oracle, &input);
        assert_eq!(recreated_npc(&input, &rng), theirs, "{:?}", &input.sprites[..3]);
    }
    for (map, x, y) in inputs::map_sprites() {
        assert_eq!(recreated_map_sprites(map, x, y), init_map_sprites(&mut oracle, (map, x, y)), "{:?}", Map::from_repr(map));
    }
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/overworld/*.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_overworld() {
    use super::{write_fixture, Case};
    let mut oracle = oracle();
    fn case<I, O>(input: I, output: O) -> Case<I, O> {
        Case { input, output, rng: vec![] }
    }
    write_fixture("overworld", "tile_pair_collisions",
        &inputs::tile_pairs().into_iter().map(|i| case(i, tile_pair_collision(&mut oracle, i))).collect::<Vec<_>>());
    write_fixture("overworld", "handle_ledges",
        &inputs::ledges().into_iter().map(|i| case(i, handle_ledges(&mut oracle, i))).collect::<Vec<_>>());
    write_fixture("overworld", "door_or_warp_tile",
        &inputs::door_or_warp_tiles().into_iter().map(|i| case(i, door_or_warp_tile(&mut oracle, i))).collect::<Vec<_>>());
    write_fixture("overworld", "extra_warp_check",
        &inputs::extra_warps().into_iter().map(|i| case(i, extra_warp_check(&mut oracle, i))).collect::<Vec<_>>());
    write_fixture("overworld", "tile_in_front",
        &inputs::squares().into_iter().map(|i| case(i, tile_in_front(&mut oracle, i))).collect::<Vec<_>>());
    write_fixture("overworld", "detect_collision_between_sprites",
        &inputs::collisions().into_iter().map(|i| {
            let out = detect_collision(&mut oracle, i.clone());
            case((i.0, i.1.iter().map(sprite_hex).collect::<Vec<_>>()), sprite_hex(&out))
        }).collect::<Vec<_>>());
    write_fixture("overworld", "update_npc_sprite",
        &inputs::npcs().into_iter().map(|i| {
            let (output, rng) = update_npc_sprite(&mut oracle, &i);
            let input = (hex(&i.tiles), i.sprites[..3].iter().map(sprite_hex).collect::<Vec<_>>(), i.x, i.y, i.walk_counter, i.player_direction);
            Case { input, output: output.iter().map(sprite_hex).collect::<Vec<_>>(), rng }
        }).collect::<Vec<_>>());
    write_fixture("overworld", "init_map_sprites",
        &inputs::map_sprites().into_iter().map(|i| case(i, init_map_sprites(&mut oracle, i))).collect::<Vec<_>>());
}
