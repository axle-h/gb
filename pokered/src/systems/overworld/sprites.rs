//! The sixteen sprite slots and what `UpdateSprites` and VBlank's `PrepareOAMData` do with them.
//! Slot 0 is the player. Positions are screen pixels, as the cartridge keeps them, so a step of
//! the player shifts every other sprite the other way.

use poke_core::map::Map;
use poke_core::map_objects::{sprite_set, sprite_set_id, sprite_sheet, FIRST_STILL_SPRITE, SPRITE_SET_LENGTH, STAY, WALK};
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use serde::{Deserialize, Serialize};
use crate::gfx::layers::Object;
use crate::gfx::tiles::{TileData, V_CHARS0, V_CHARS1};
use crate::rng::{GameRng, Rng};
use poke_core::map_header::TileSetId;
use super::bike_surf::is_bike_riding_allowed;
use super::location::{Location, BIKING, SURFING, WALKING};
use super::map_view::TileMap;

pub const NUM_SPRITES: usize = 16;
/// `MAP_TILESET_SIZE`: a tile id at or past it is text, not map.
pub const MAP_TILESET_SIZE: u8 = 0x60;
const SPRITE_RED: u8 = 0x01;
pub const SPRITE_FACING_DOWN: u8 = 0x0;
pub const SPRITE_FACING_UP: u8 = 0x4;
pub const SPRITE_FACING_LEFT: u8 = 0x8;
pub const SPRITE_FACING_RIGHT: u8 = 0xC;
/// `BIT_FACE_PLAYER` of the movement status.
pub const FACE_PLAYER: u8 = 0x80;
const OAM_PRIO: u8 = 0x80;
const UNDER_GRASS: u8 = 1 << 1;
const FACING_END: u8 = 1 << 0;
/// `SCREEN_HEIGHT_PX + OAM_Y_OFS`: where an unused object is parked.
const OAM_HIDDEN_Y: u8 = 160;
const DOWN: u8 = 0xD0;
const UP: u8 = 0xD1;
const LEFT: u8 = 0xD2;
const RIGHT: u8 = 0xD3;
const UP_DOWN: u8 = 0x01;
const LEFT_RIGHT: u8 = 0x02;
const NPC_MOVEMENT_DOWN: u8 = 0x00;
const NPC_MOVEMENT_UP: u8 = 0x40;
const NPC_MOVEMENT_LEFT: u8 = 0x80;
const NPC_MOVEMENT_RIGHT: u8 = 0xC0;
/// `NPC_CHANGE_FACING`.
pub const NPC_CHANGE_FACING: u8 = 0xE0;

/// The paths scripts lay for NPCs: `MoveSprite`'s, walked a square at a time through
/// `UpdateNPCSprite`, and the in-step one `DoScriptedNPCMovement` walks beside the player.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcPaths {
    /// `wNPCMovementDirections`, up to and including its `$ff`.
    pub directions: Vec<u8>,
    /// `wNPCNumScriptedSteps`.
    pub num_scripted_steps: u8,
    /// `BIT_SCRIPTED_NPC_MOVEMENT`: a `MoveSprite` path is being walked.
    pub scripted_npc_movement: bool,
    /// `wNPCMovementDirections2`, `wNPCMovementDirections2Index` and `wScriptedNPCWalkCounter`.
    pub directions2: Vec<u8>,
    pub directions2_index: u8,
    pub scripted_walk_counter: u8,
    /// `wNPCMovementScriptSpriteOffset`, as a slot: the sprite `DoScriptedNPCMovement` moves instead
    /// of `UpdateNPCSprite`.
    pub script_sprite: u8,
    /// `BIT_INIT_SCRIPTED_MOVEMENT`.
    pub init_scripted_movement: bool,
    /// A `MoveSprite` path reached its end this pass, which zeroes `wSimulatedJoypadStatesIndex`.
    pub path_ended: bool,
}

/// One slot of `wSpriteStateData1` and `wSpriteStateData2`, with its entry of `wMapSpriteData`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteState {
    pub picture_id: u8,
    /// 0 uninitialised, 1 ready, 2 delayed, 3 walking; bit 7 is `BIT_FACE_PLAYER`.
    pub movement_status: u8,
    /// `$ff` off screen, else the VRAM slot in the high nybble and the frame in the low.
    pub image_index: u8,
    pub y_step: u8,
    pub y_pixels: u8,
    pub x_step: u8,
    pub x_pixels: u8,
    pub intra_anim_frame_counter: u8,
    pub anim_frame_counter: u8,
    pub facing: u8,
    pub y_adjusted: u8,
    pub x_adjusted: u8,
    /// Directions another sprite is in the way, as `PLAYER_DIR_*` bits.
    pub collision_data: u8,
    pub data1_0d: u8,
    /// `SPRITESTATEDATA1_0E` and `_0F`: which slots were in the way, big-endian bits.
    pub collided_with: [u8; 2],
    pub walk_animation_counter: u8,
    pub data2_01: u8,
    /// Start at 8 and count a wandering sprite's steps away from home.
    pub y_displacement: u8,
    pub x_displacement: u8,
    /// Squares, plus the 4 `object_event` adds.
    pub map_y: u8,
    pub map_x: u8,
    pub movement1: u8,
    pub grass_priority: u8,
    pub movement_delay: u8,
    pub orig_facing: u8,
    pub data2_0a: [u8; 3],
    pub picture_id2: u8,
    pub image_base_offset: u8,
    pub data2_0f: u8,
    /// `wMapSpriteData`: movement byte 2 and the text id.
    pub movement2: u8,
    pub text_id: u8,
}

impl SpriteState {
    pub fn from_bytes(data1: &[u8], data2: &[u8], map_data: [u8; 2]) -> Self {
        Self {
            picture_id: data1[0],
            movement_status: data1[1],
            image_index: data1[2],
            y_step: data1[3],
            y_pixels: data1[4],
            x_step: data1[5],
            x_pixels: data1[6],
            intra_anim_frame_counter: data1[7],
            anim_frame_counter: data1[8],
            facing: data1[9],
            y_adjusted: data1[10],
            x_adjusted: data1[11],
            collision_data: data1[12],
            data1_0d: data1[13],
            collided_with: [data1[14], data1[15]],
            walk_animation_counter: data2[0],
            data2_01: data2[1],
            y_displacement: data2[2],
            x_displacement: data2[3],
            map_y: data2[4],
            map_x: data2[5],
            movement1: data2[6],
            grass_priority: data2[7],
            movement_delay: data2[8],
            orig_facing: data2[9],
            data2_0a: [data2[10], data2[11], data2[12]],
            picture_id2: data2[13],
            image_base_offset: data2[14],
            data2_0f: data2[15],
            movement2: map_data[0],
            text_id: map_data[1],
        }
    }

    pub fn data1(&self) -> [u8; 16] {
        [self.picture_id, self.movement_status, self.image_index, self.y_step, self.y_pixels, self.x_step,
         self.x_pixels, self.intra_anim_frame_counter, self.anim_frame_counter, self.facing, self.y_adjusted,
         self.x_adjusted, self.collision_data, self.data1_0d, self.collided_with[0], self.collided_with[1]]
    }

    pub fn data2(&self) -> [u8; 16] {
        [self.walk_animation_counter, self.data2_01, self.y_displacement, self.x_displacement, self.map_y,
         self.map_x, self.movement1, self.grass_priority, self.movement_delay, self.orig_facing,
         self.data2_0a[0], self.data2_0a[1], self.data2_0a[2], self.picture_id2, self.image_base_offset,
         self.data2_0f]
    }
}

pub type Sprites = [SpriteState; NUM_SPRITES];

/// `SetSpriteCollisionValues`: `(b, c)` for a step of -1, 0 or 1.
fn collision_values(step: u8) -> (u8, u8) {
    match step {
        0 => (0, 0),
        0xFF => (0xFF, 9),
        _ => (0, 7),
    }
}

/// `add b; and $f0; or c`: a coordinate snapped to its square and pushed seven pixels the way the
/// sprite is moving.
fn adjusted(pixels: u8, step: u8) -> u8 {
    let (b, c) = collision_values(step);
    (pixels.wrapping_add(b) & 0xF0) | c
}

/// `sub [hl]` and the absolute value, with the carry the subtraction left.
fn distance(a: u8, b: u8) -> (u8, bool) {
    let (diff, carry) = a.overflowing_sub(b);
    (if carry { diff.wrapping_neg() } else { diff }, carry)
}

/// `DetectCollisionBetweenSprites` for slot `i`: `collision_data` gets the directions some other
/// visible sprite is within a square of it, and `collided_with` which ones.
pub fn detect_collision_between_sprites(sprites: &mut Sprites, i: usize) {
    if sprites[i].picture_id == 0 {
        return;
    }
    let me = &mut sprites[i];
    me.y_adjusted = adjusted(me.y_pixels.wrapping_add(4), me.y_step);
    me.x_adjusted = adjusted(me.x_pixels, me.x_step);
    me.data1_0d = 0;
    me.collision_data = 0;
    let (my_y, my_x) = (me.y_adjusted, me.x_adjusted);
    for j in 0..NUM_SPRITES {
        let other = sprites[j];
        if j == i || other.picture_id == 0 || other.image_index == 0xFF {
            continue;
        }
        let (y_distance, y_carry) = distance(adjusted(other.y_pixels.wrapping_add(4), other.y_step), my_y);
        // Two `rl c`s over the register `SetSpriteCollisionValues` left: only the low bits survive.
        let mut c = collision_values(other.y_step).1 << 2 | (y_carry as u8) << 1 | !y_carry as u8;
        let temp_y = if my_y & 0x0F != 0 { 9 } else { 7 };
        if y_distance >= temp_y {
            let other_b = if other.y_step != 0 { 9 } else { 7 };
            let (rest, carry) = (y_distance - temp_y).overflowing_sub(other_b);
            if rest != 0 && !carry {
                continue;
            }
        }
        let (x_distance, x_carry) = distance(adjusted(other.x_pixels, other.x_step), my_x);
        c = c << 2 | (x_carry as u8) << 1 | !x_carry as u8;
        let temp_x = if my_x & 0x0F != 0 { 9 } else { 7 };
        if x_distance >= temp_x {
            let other_b = if other.x_step != 0 { 9 } else { 7 };
            let (rest, carry) = (x_distance - temp_x).overflowing_sub(other_b);
            if rest != 0 && !carry {
                continue;
            }
        }
        let mask = if temp_y < temp_x { 0b0011 } else { 0b1100 };
        let me = &mut sprites[i];
        me.collision_data |= c & mask;
        let bit = (1u16 << j).to_be_bytes();
        me.collided_with[0] |= bit[0];
        me.collided_with[1] |= bit[1];
    }
}

/// What the sprite routines read besides the slots.
pub struct SpriteEnv<'a> {
    /// `wTileMap`.
    pub tiles: &'a TileMap,
    /// `wXCoord`, `wYCoord`.
    pub x: u8,
    pub y: u8,
    pub walk_counter: u8,
    /// `BIT_FONT_LOADED`.
    pub font_loaded: bool,
    /// `wTilesetCollisionPtr`'s list.
    pub collision: &'a [u8],
    pub grass_tile: u8,
    /// `IsObjectHidden`, per slot.
    pub hidden: [bool; NUM_SPRITES],
    /// `BIT_NO_NPC_FACE_PLAYER`.
    pub no_face_player: bool,
    /// `wPlayerDirection`.
    pub player_direction: u8,
    /// `wPlayerMovingDirection`.
    pub moving_direction: u8,
    /// `BIT_SPINNING`.
    pub spinning: bool,
    /// `BIT_SCRIPTED_MOVEMENT_STATE`, which `DoScriptedNPCMovement` moves only under.
    pub simulating: bool,
    /// What lies past the end of `wTileMap`, `wSurroundingTiles`, which a sprite below the screen
    /// reads its tiles from; without it such a read is `$ff`.
    pub beyond: Option<&'a [u8]>,
}

impl SpriteEnv<'_> {
    fn tile(&self, at: isize) -> u8 {
        let Ok(at) = usize::try_from(at) else { return 0xFF };
        match (self.tiles.get(at), self.beyond) {
            (Some(&tile), _) => tile,
            (None, Some(beyond)) => beyond.get(at - self.tiles.len()).copied().unwrap_or(0xFF),
            (None, None) => 0xFF,
        }
    }
}

/// `_UpdateSprites`: every slot with a picture, the player first.
pub fn update_sprites(sprites: &mut Sprites, env: &SpriteEnv, paths: &mut NpcPaths, rng: &mut GameRng) {
    for slot in 0..NUM_SPRITES {
        match sprites[slot].image_base_offset {
            0 => {}
            1 => update_player_sprite(sprites, env),
            _ if slot as u8 == paths.script_sprite => do_scripted_npc_movement(&mut sprites[slot], env, paths),
            _ => update_npc_sprite(sprites, slot, env, paths, rng),
        }
    }
}

/// `UpdatePlayerSprite`.
pub fn update_player_sprite(sprites: &mut Sprites, env: &SpriteEnv) {
    let player = &mut sprites[0];
    if player.walk_animation_counter != 0 {
        if player.walk_animation_counter != 0xFF {
            player.walk_animation_counter -= 1;
        }
        player.image_index = 0xFF;
        return;
    }
    // `hTilePlayerStandingOn`, and a text box drawn over the player hides it.
    let standing = env.tiles[9 * 20 + 8];
    if standing >= MAP_TILESET_SIZE {
        player.image_index = 0xFF;
        return;
    }
    detect_collision_between_sprites(sprites, 0);
    let player = &mut sprites[0];
    let mut animate = env.walk_counter != 0;
    if !animate {
        let facing = match env.moving_direction {
            d if d & 4 != 0 => Some(SPRITE_FACING_DOWN),
            d if d & 8 != 0 => Some(SPRITE_FACING_UP),
            d if d & 2 != 0 => Some(SPRITE_FACING_LEFT),
            d if d & 1 != 0 => Some(SPRITE_FACING_RIGHT),
            _ => None,
        };
        match facing {
            Some(facing) if !env.font_loaded => {
                player.facing = facing;
                animate = true;
            }
            Some(facing) => {
                player.facing = facing;
                player.intra_anim_frame_counter = 0;
                player.anim_frame_counter = 0;
            }
            None => {
                player.intra_anim_frame_counter = 0;
                player.anim_frame_counter = 0;
            }
        }
    }
    if animate {
        if env.spinning {
            player.grass_priority = if standing == env.grass_tile { OAM_PRIO } else { 0 };
            return;
        }
        player.intra_anim_frame_counter += 1;
        if player.intra_anim_frame_counter == 4 {
            player.intra_anim_frame_counter = 0;
            player.anim_frame_counter = (player.anim_frame_counter + 1) & 3;
        }
    }
    player.image_index = player.anim_frame_counter.wrapping_add(player.facing);
    player.grass_priority = if standing == env.grass_tile { OAM_PRIO } else { 0 };
}

/// `GetTileSpriteStandsOn`: the lower left tile of the sprite's square, as an index into
/// `wTileMap` that can run off either end for a sprite at the screen's edge.
fn tile_sprite_stands_on(sprite: &SpriteState) -> isize {
    let row = (sprite.y_pixels.wrapping_add(4) & 0xF0) >> 1;
    let column = (sprite.x_pixels >> 3) + 20;
    row as isize * 5 + column as isize
}

/// `UpdateSpriteImage`.
fn update_sprite_image(sprite: &mut SpriteState) {
    let base = sprite.image_base_offset.wrapping_sub(1).rotate_left(4);
    sprite.image_index = sprite.anim_frame_counter.wrapping_add(sprite.facing).wrapping_add(base);
}

/// `UpdateNPCSprite`.
pub fn update_npc_sprite(sprites: &mut Sprites, slot: usize, env: &SpriteEnv, paths: &mut NpcPaths, rng: &mut GameRng) {
    let sprite = &mut sprites[slot];
    if sprite.movement_status == 0 {
        // `InitializeSpriteStatus`.
        sprite.movement_status = 1;
        sprite.image_index = 0xFF;
        sprite.y_displacement = 8;
        sprite.x_displacement = 8;
        return;
    }
    if !check_sprite_availability(sprite, slot, env) {
        return;
    }
    if sprite.movement_status & FACE_PLAYER != 0 {
        // `MakeNPCFacePlayer`.
        if !env.no_face_player {
            sprite.movement_status &= !FACE_PLAYER;
            sprite.facing = match env.player_direction {
                d if d & 8 != 0 => SPRITE_FACING_DOWN,
                d if d & 4 != 0 => SPRITE_FACING_UP,
                d if d & 2 != 0 => SPRITE_FACING_RIGHT,
                _ => SPRITE_FACING_LEFT,
            };
        }
        return not_yet_moving(sprite);
    }
    if env.font_loaded {
        return not_yet_moving(sprite);
    }
    match sprite.movement_status {
        2 => return update_sprite_movement_delay(sprite),
        3 => return update_sprite_in_walking_animation(sprite, rng),
        _ => {}
    }
    if env.walk_counter != 0 {
        return;
    }
    // `InitializeSpriteScreenPosition`.
    sprite.y_pixels = sprite.map_y.wrapping_sub(env.y).rotate_left(4).wrapping_sub(4);
    sprite.x_pixels = sprite.map_x.wrapping_sub(env.x).rotate_left(4);
    let mut at = tile_sprite_stands_on(sprite);
    let random = if sprite.movement1 < WALK {
        // `MoveSprite`'s path: movement byte 1 is the index of the next direction.
        let index = sprite.movement1;
        sprite.movement1 = index.wrapping_add(1);
        paths.num_scripted_steps = paths.num_scripted_steps.wrapping_sub(1);
        let direction = paths.directions.get(index as usize).copied().unwrap_or(STAY);
        match direction {
            STAY => {
                sprite.movement1 = STAY;
                paths.scripted_npc_movement = false;
                paths.path_ended = true;
                return;
            }
            // `ChangeFacingDirection` walks with the registers the lookup left, which no path in
            // the cartridge's first maps reaches; the sprite stands where it is.
            NPC_CHANGE_FACING => return,
            // A `WALK` in a path reads the byte 254 places on and sets the index back to 1.
            WALK => {
                sprite.movement1 = 1;
                paths.directions.get(WALK as usize).copied().unwrap_or(STAY)
            }
            direction => direction,
        }
    } else {
        rng.random()
    };
    let movement2 = sprite.movement2;
    let down = |at: &mut isize| { *at += 40; (1u8, 0u8, 4u8, SPRITE_FACING_DOWN) };
    let up = |at: &mut isize| { *at -= 40; (0xFF, 0, 8, SPRITE_FACING_UP) };
    let left = |at: &mut isize| { *at -= 2; (0, 0xFF, 2, SPRITE_FACING_LEFT) };
    let right = |at: &mut isize| { *at += 2; (0, 1, 1, SPRITE_FACING_RIGHT) };
    let (dy, dx, direction, facing) = match movement2 {
        DOWN => down(&mut at),
        UP => up(&mut at),
        LEFT => left(&mut at),
        RIGHT => right(&mut at),
        _ if random < NPC_MOVEMENT_UP => if movement2 == LEFT_RIGHT { left(&mut at) } else { down(&mut at) },
        _ if random < NPC_MOVEMENT_LEFT => if movement2 == LEFT_RIGHT { right(&mut at) } else { up(&mut at) },
        _ if random < NPC_MOVEMENT_RIGHT => if movement2 == UP_DOWN { up(&mut at) } else { left(&mut at) },
        _ => if movement2 == UP_DOWN { down(&mut at) } else { right(&mut at) },
    };
    try_walking(sprites, slot, env, rng, env.tile(at), direction, facing, dy, dx);
}

/// `TryWalking`.
#[allow(clippy::too_many_arguments)]
fn try_walking(sprites: &mut Sprites, slot: usize, env: &SpriteEnv, rng: &mut GameRng, tile: u8, direction: u8, facing: u8, dy: u8, dx: u8) {
    let sprite = &mut sprites[slot];
    sprite.facing = facing;
    sprite.y_step = dy;
    sprite.x_step = dx;
    if !can_walk_onto_tile(sprites, slot, env, rng, tile, direction, dy, dx) {
        return;
    }
    let sprite = &mut sprites[slot];
    sprite.map_y = sprite.map_y.wrapping_add(dy);
    sprite.map_x = sprite.map_x.wrapping_add(dx);
    sprite.walk_animation_counter = 0x10;
    sprite.movement_status = 3;
    update_sprite_image(sprite);
}

/// `CanWalkOntoTile`. A refusal leaves the sprite delayed a random while.
#[allow(clippy::too_many_arguments)]
fn can_walk_onto_tile(sprites: &mut Sprites, slot: usize, env: &SpriteEnv, rng: &mut GameRng, tile: u8, direction: u8, dy: u8, dx: u8) -> bool {
    if sprites[slot].movement1 < WALK {
        return true;
    }
    let passable = (|| {
        let sprite = sprites[slot];
        if !env.collision.contains(&tile) || sprite.movement1 == STAY {
            return false;
        }
        if sprite.y_pixels.wrapping_add(4).wrapping_add(dy) >= 0x80 || sprite.x_pixels.wrapping_add(dx) >= 0x90 {
            return false;
        }
        true
    })();
    if passable {
        detect_collision_between_sprites(sprites, slot);
        let sprite = &mut sprites[slot];
        if sprite.collision_data & direction == 0 {
            // Walking up is limited to four steps from home, and a sprite that has done it can no
            // longer move sideways: the test against 5 is made for every step that is not up.
            let y = if dy & 0x80 != 0 {
                sprite.y_displacement.checked_sub(1)
            } else {
                Some(sprite.y_displacement.wrapping_add(dy)).filter(|&y| y >= 5)
            };
            let x = if dx & 0x80 != 0 {
                sprite.x_displacement.checked_sub(1)
            } else {
                Some(sprite.x_displacement.wrapping_add(dx))
            };
            if let (Some(y), Some(x)) = (y, x) {
                sprite.x_displacement = x;
                sprite.y_displacement = y;
                return true;
            }
        }
    }
    let sprite = &mut sprites[slot];
    sprite.movement_status = 2;
    sprite.y_step = 0;
    sprite.x_step = 0;
    sprite.movement_delay = rng.random() & 0x7F;
    false
}

/// `DoScriptedNPCMovement`: two pixels a pass along `wNPCMovementDirections2`, in step with a player
/// whose presses are simulated.
fn do_scripted_npc_movement(sprite: &mut SpriteState, env: &SpriteEnv, paths: &mut NpcPaths) {
    if !env.simulating {
        return;
    }
    if !std::mem::replace(&mut paths.init_scripted_movement, true) {
        paths.directions2_index = 0;
        paths.scripted_walk_counter = 8;
        return anim_scripted_npc_movement(sprite);
    }
    let direction = paths.directions2.get(paths.directions2_index as usize).copied().unwrap_or(STAY);
    let (facing, dy, dx) = match direction {
        NPC_MOVEMENT_UP => (SPRITE_FACING_UP, -2i8, 0i8),
        NPC_MOVEMENT_DOWN => (SPRITE_FACING_DOWN, 2, 0),
        NPC_MOVEMENT_LEFT => (SPRITE_FACING_LEFT, 0, -2),
        NPC_MOVEMENT_RIGHT => (SPRITE_FACING_RIGHT, 0, 2),
        _ => return,
    };
    sprite.y_pixels = sprite.y_pixels.wrapping_add(dy as u8);
    sprite.x_pixels = sprite.x_pixels.wrapping_add(dx as u8);
    sprite.facing = facing;
    anim_scripted_npc_movement(sprite);
    paths.scripted_walk_counter = paths.scripted_walk_counter.wrapping_sub(1);
    if paths.scripted_walk_counter == 0 {
        paths.scripted_walk_counter = 8;
        paths.directions2_index = paths.directions2_index.wrapping_add(1);
    }
}

/// `AnimScriptedNPCMovement`, with `AdvanceScriptedNPCAnimFrameCounter`. The frame added is
/// `hSpriteAnimFrameCounter`, written only when the counter wraps, which is the sprite's own frame
/// while one sprite at a time is moved this way.
fn anim_scripted_npc_movement(sprite: &mut SpriteState) {
    if !matches!(sprite.facing, SPRITE_FACING_DOWN | SPRITE_FACING_UP | SPRITE_FACING_LEFT | SPRITE_FACING_RIGHT) {
        return;
    }
    let base = sprite.image_base_offset.wrapping_sub(1).rotate_left(4).wrapping_add(sprite.facing);
    sprite.intra_anim_frame_counter = sprite.intra_anim_frame_counter.wrapping_add(1);
    if sprite.intra_anim_frame_counter == 4 {
        sprite.intra_anim_frame_counter = 0;
        sprite.anim_frame_counter = (sprite.anim_frame_counter + 1) & 3;
    }
    sprite.image_index = base.wrapping_add(sprite.anim_frame_counter);
}

/// `UpdateSpriteInWalkingAnimation`: a pixel a call, sixteen to a step.
fn update_sprite_in_walking_animation(sprite: &mut SpriteState, rng: &mut GameRng) {
    sprite.intra_anim_frame_counter += 1;
    if sprite.intra_anim_frame_counter == 4 {
        sprite.intra_anim_frame_counter = 0;
        sprite.anim_frame_counter = (sprite.anim_frame_counter + 1) & 3;
    }
    sprite.y_pixels = sprite.y_pixels.wrapping_add(sprite.y_step);
    sprite.x_pixels = sprite.x_pixels.wrapping_add(sprite.x_step);
    sprite.walk_animation_counter = sprite.walk_animation_counter.wrapping_sub(1);
    if sprite.walk_animation_counter != 0 {
        return;
    }
    if sprite.movement1 < WALK {
        sprite.movement_status = 1;
        return;
    }
    // A delay of 0 is 256 frames, since the count is decremented before it is tested.
    sprite.movement_delay = rng.random() & 0x7F;
    sprite.movement_status = 2;
    sprite.y_step = 0;
    sprite.x_step = 0;
}

/// `UpdateSpriteMovementDelay`.
fn update_sprite_movement_delay(sprite: &mut SpriteState) {
    if sprite.movement1 < WALK {
        sprite.movement_delay = 0;
        sprite.movement_status = 1;
    } else {
        sprite.movement_delay = sprite.movement_delay.wrapping_sub(1);
        if sprite.movement_delay == 0 {
            sprite.movement_status = 1;
        }
    }
    not_yet_moving(sprite);
}

/// `NotYetMoving`.
fn not_yet_moving(sprite: &mut SpriteState) {
    sprite.anim_frame_counter = 0;
    update_sprite_image(sprite);
}

/// `CheckSpriteAvailability`: `false`, with the sprite hidden, for one that is toggled off, outside
/// the squares around the player, or standing on a text box.
fn check_sprite_availability(sprite: &mut SpriteState, slot: usize, env: &SpriteEnv) -> bool {
    let visible = (|| {
        if env.hidden[slot] {
            return None;
        }
        if sprite.movement1 >= WALK {
            if env.y != sprite.map_y && (env.y > sprite.map_y || env.y.wrapping_add(8) < sprite.map_y) {
                return None;
            }
            if env.x != sprite.map_x && (env.x > sprite.map_x || env.x.wrapping_add(9) < sprite.map_x) {
                return None;
            }
        }
        let at = tile_sprite_stands_on(sprite);
        let corners = [env.tile(at), env.tile(at + 1), env.tile(at - 20)];
        let top_right = env.tile(at - 19);
        if corners.iter().any(|&tile| tile >= MAP_TILESET_SIZE) || top_right >= MAP_TILESET_SIZE {
            return None;
        }
        Some(top_right)
    })();
    let Some(top_right) = visible else {
        sprite.image_index = 0xFF;
        return false;
    };
    if env.walk_counter == 0 {
        update_sprite_image(sprite);
        // The grass test reads the sprite's top right tile, where the player's reads its bottom left.
        sprite.grass_priority = if top_right == env.grass_tile { OAM_PRIO } else { 0 };
    }
    true
}

/// `IsSpriteInFrontOfPlayer2`: the slot within `range` pixels the way the player faces, which is
/// marked to turn and face the player. Also leaves `wPlayerDirection` pointing that way.
pub fn sprite_in_front_of_player(sprites: &mut Sprites, num_sprites: u8, range: u8, player_direction: &mut u8) -> u8 {
    let (mut y, mut x) = (0x3Cu8, 0x40u8);
    match sprites[0].facing {
        SPRITE_FACING_UP => { y = y.wrapping_sub(range); *player_direction = 8; }
        SPRITE_FACING_DOWN => { y = y.wrapping_add(range); *player_direction = 4; }
        SPRITE_FACING_RIGHT => { x = x.wrapping_add(range); *player_direction = 1; }
        _ => { x = x.wrapping_sub(range); *player_direction = 2; }
    }
    for slot in 1..=num_sprites as usize {
        let sprite = &mut sprites[slot];
        if sprite.picture_id == 0 || sprite.image_index == 0xFF {
            continue;
        }
        if sprite.y_pixels == y && sprite.x_pixels == x {
            sprite.movement_status |= FACE_PLAYER;
            return slot as u8;
        }
    }
    0
}

/// `PrepareOAMData`, which VBlank runs from the slots every frame. With a ledge being jumped the
/// last four objects are the shadow and are left alone.
pub fn prepare_oam(sprites: &mut Sprites, objects: &mut Vec<Object>, ledge: bool) {
    objects.resize(40, Object { y: OAM_HIDDEN_Y, ..Object::default() });
    let table = pokered_symbols::SpriteFacingAndAnimationTable;
    let at = |address: u16| rom_slice(DmgPointer { bank: table.bank, address });
    let mut next = 0;
    for sprite in sprites.iter_mut() {
        if sprite.picture_id == 0 {
            continue;
        }
        let image = sprite.image_index;
        // `GetSpriteScreenXY`.
        sprite.y_adjusted = sprite.y_pixels.wrapping_add(4) & 0xF0;
        sprite.x_adjusted = sprite.x_pixels & 0xF0;
        if image == 0xFF {
            continue;
        }
        let entry = if image >= 0xA0 { (image & 0x0F) + 0x10 } else { image & 0x0F };
        let row = rom_slice(table + entry as u16 * 4);
        let tiles = at(u16::from_le_bytes([row[0], row[1]]));
        let layout = at(u16::from_le_bytes([row[2], row[3]]));
        let slot = image >> 4;
        let first_tile = if slot == 0x0B { 0x0A * 12 + 4 } else { slot * 12 };
        for quadrant in 0..4 {
            let (dy, dx, flags) = (layout[quadrant * 3], layout[quadrant * 3 + 1], layout[quadrant * 3 + 2]);
            let attributes = if flags & UNDER_GRASS != 0 { sprite.grass_priority & OAM_PRIO | flags } else { flags };
            objects[next] = Object {
                y: sprite.y_pixels.wrapping_add(0x10).wrapping_add(dy),
                x: sprite.x_pixels.wrapping_add(8).wrapping_add(dx),
                tile: first_tile.wrapping_add(tiles[quadrant]),
                attributes,
            };
            next += 1;
            if attributes & FACING_END != 0 {
                break;
            }
        }
    }
    let end = if ledge { 36 } else { 40 };
    for object in &mut objects[next.min(end)..end] {
        object.y = OAM_HIDDEN_Y;
    }
}

/// The sprite set outside, which `InitOutsideMapSprites` keeps: `wSpriteSetID` and `wSpriteSet`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteSet {
    pub id: u8,
    pub pictures: [u8; SPRITE_SET_LENGTH],
}

/// `InitMapSprites`: which VRAM slot each sprite draws from, and the patterns loaded there. With
/// the font loaded only the walking frames, which the font covered, are loaded again.
pub fn init_map_sprites(sprites: &mut Sprites, set: &mut SpriteSet, map: Map, x: u8, y: u8, num_sprites: u8, font_loaded: bool, tiles: &mut TileData) {
    let Some(id) = sprite_set_id(map, x, y) else {
        for sprite in sprites.iter_mut() {
            sprite.picture_id2 = sprite.picture_id;
        }
        return load_map_sprite_tile_patterns(sprites, num_sprites, font_loaded, tiles);
    };
    if font_loaded || set.id != id {
        *set = SpriteSet { id, pictures: sprite_set(id) };
        sprites[0].picture_id2 = SPRITE_RED;
        for (slot, sprite) in sprites.iter_mut().enumerate().skip(1) {
            sprite.picture_id2 = set.pictures.get(slot - 1).copied().unwrap_or(0);
        }
        load_map_sprite_tile_patterns(sprites, SPRITE_SET_LENGTH as u8, font_loaded, tiles);
        for sprite in sprites.iter_mut().skip(1) {
            sprite.image_base_offset = 0;
        }
    }
    for sprite in sprites.iter_mut().skip(1) {
        sprite.image_base_offset = match sprite.picture_id {
            0 => 0,
            // Not found runs on past the set, as the cartridge's loop has no end test.
            picture => set.pictures.iter().position(|&p| p == picture).map_or(0, |i| i as u8 + 2),
        };
    }
}

/// `LoadMapSpriteTilePatterns`: slots 1 to `count` given the next free VRAM slot, or the one an
/// earlier slot with the same picture has; a still picture takes one of the two four-tile slots.
fn load_map_sprite_tile_patterns(sprites: &mut Sprites, count: u8, font_loaded: bool, tiles: &mut TileData) {
    if count == 0 {
        return;
    }
    for sprite in sprites.iter_mut() {
        sprite.image_base_offset = sprite.picture_id2;
    }
    let mut four_tile_sprites = 0;
    for slot in 1..=count as usize {
        let picture = sprites[slot].image_base_offset;
        if let Some(earlier) = (1..slot).find(|&earlier| sprites[earlier].picture_id2 == picture) {
            sprites[slot].image_base_offset = sprites[earlier].image_base_offset;
            continue;
        }
        let highest = (1..slot).map(|earlier| sprites[earlier].image_base_offset).filter(|&base| base < 11).fold(1, u8::max);
        let vram_slot = if picture >= FIRST_STILL_SPRITE { four_tile_sprites + 11 } else { highest + 1 };
        sprites[slot].image_base_offset = vram_slot;
        let sheet = sprite_sheet(picture);
        let bytes = rom_slice(sheet.pointer);
        let first = if vram_slot >= 11 {
            let first = if four_tile_sprites == 0 { 0x78 } else { 0x7C };
            four_tile_sprites = 1;
            first
        } else {
            (vram_slot as usize - 1) * 12
        };
        if !font_loaded {
            tiles.load(V_CHARS0 + first, &bytes[..sheet.bytes]);
        }
        if vram_slot < 11 {
            tiles.load(V_CHARS1 + first, &bytes[0xC0..0xC0 + sheet.bytes]);
        }
    }
    for sprite in sprites.iter_mut() {
        sprite.picture_id2 = 0;
    }
}

/// `LoadWalkingPlayerSpriteGraphics`: Red's standing frames to slot 1, the walking ones above.
pub fn load_walking_player_sprite_graphics(tiles: &mut TileData) {
    load_player_sprite_graphics_common(tiles, pokered_symbols::RedSprite);
}

/// `LoadPlayerSpriteGraphicsCommon`.
fn load_player_sprite_graphics_common(tiles: &mut TileData, sheet: DmgPointer) {
    let bytes = rom_slice(sheet);
    tiles.load(V_CHARS0, &bytes[..12 * TILE_BYTES]);
    tiles.load(V_CHARS1, &bytes[0xC0..0xC0 + 12 * TILE_BYTES]);
}

/// `LoadBirdSpriteGraphics`: the bird over the player's own sheet, for the flying animation.
pub fn load_bird_sprite_graphics(tiles: &mut TileData) {
    load_player_sprite_graphics_common(tiles, pokered_symbols::BirdSprite);
}

/// `LoadPlayerSpriteGraphics`: Red walking, on the bike or on the Seel. The bike where it cannot be
/// ridden, and the water on a map whose tiles do not animate, put the player back on foot.
pub fn load_player_sprite_graphics(tiles: &mut TileData, location: &mut Location, tileset: TileSetId) {
    let keeps = match location.walk_bike_surf {
        BIKING => is_bike_riding_allowed(location.map, tileset),
        _ => tiles.animation.kind != 0,
    };
    if !keeps {
        location.walk_bike_surf = WALKING;
    }
    let sheet = match location.walk_bike_surf {
        BIKING => pokered_symbols::RedBikeSprite,
        SURFING => pokered_symbols::SeelSprite,
        _ => pokered_symbols::RedSprite,
    };
    load_player_sprite_graphics_common(tiles, sheet);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walker(y: u8, x: u8, y_step: u8, x_step: u8) -> SpriteState {
        SpriteState { picture_id: 4, image_index: 0x10, y_pixels: y, x_pixels: x, y_step, x_step, ..SpriteState::default() }
    }

    use crate::fixtures::cases;

    fn from_hex(text: &str) -> SpriteState {
        let bytes: Vec<u8> = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect();
        SpriteState::from_bytes(&bytes[..16], &bytes[16..32], [bytes[32], bytes[33]])
    }

    #[test]
    fn sprites_in_the_way_match_the_cartridge() {
        for ((i, slots), after, _) in cases::<(u8, Vec<String>), String>(include_str!("../../../fixtures/overworld/detect_collision_between_sprites.jsonl")) {
            let mut sprites: Sprites = std::array::from_fn(|slot| from_hex(&slots[slot]));
            detect_collision_between_sprites(&mut sprites, i as usize);
            assert_eq!(sprites[i as usize], from_hex(&after), "slot {i} of {slots:?}");
        }
    }

    #[test]
    fn an_npc_s_move_matches_the_cartridge() {
        let collision = poke_core::tilesets::collision_tiles(poke_core::map_header::TileSetId::Overworld);
        for ((tiles, slots, x, y, walk_counter, player_direction), after, rng) in
            cases::<(String, Vec<String>, u8, u8, u8, u8), Vec<String>>(include_str!("../../../fixtures/overworld/update_npc_sprite.jsonl"))
        {
            let tiles: TileMap = std::array::from_fn(|i| u8::from_str_radix(&tiles[i * 2..i * 2 + 2], 16).unwrap());
            let mut sprites = [SpriteState::default(); NUM_SPRITES];
            for (slot, text) in slots.iter().enumerate() {
                sprites[slot] = from_hex(text);
            }
            let env = SpriteEnv {
                tiles: &tiles, x, y, walk_counter, font_loaded: false, collision: &collision, grass_tile: 0x52,
                hidden: [false; NUM_SPRITES], no_face_player: false, player_direction, moving_direction: 0, spinning: false,
                simulating: false, beyond: None,
            };
            let mut tape = GameRng::tape(rng.clone());
            update_npc_sprite(&mut sprites, 1, &env, &mut NpcPaths::default(), &mut tape);
            let ours: Vec<SpriteState> = sprites[..3].to_vec();
            assert_eq!(ours, after.iter().map(|text| from_hex(text)).collect::<Vec<_>>(), "{slots:?} rng {rng:?}");
            assert!(matches!(tape, GameRng::Tape { cursor, .. } if cursor == rng.len()), "every random byte taken");
        }
    }

    #[test]
    fn every_map_s_sprites_draw_from_the_cartridge_s_vram_slots() {
        for ((map, x, y), (bases, set_id), _) in cases::<(u8, u8, u8), (Vec<u8>, u8)>(include_str!("../../../fixtures/overworld/init_map_sprites.jsonl")) {
            let map = Map::from_repr(map).unwrap();
            let objects = poke_core::map_objects::MapObjects::read(map).unwrap();
            let mut sprites = [SpriteState::default(); NUM_SPRITES];
            sprites[0].picture_id = 1;
            sprites[0].image_base_offset = 1;
            for (slot, object) in objects.objects.iter().enumerate() {
                sprites[slot + 1].picture_id = object.picture;
            }
            let mut set = SpriteSet::default();
            init_map_sprites(&mut sprites, &mut set, map, x, y, objects.objects.len() as u8, false, &mut TileData::default());
            assert_eq!((sprites.iter().map(|s| s.image_base_offset).collect::<Vec<_>>(), set.id), (bases, set_id), "{map}");
        }
    }

    #[test]
    fn a_sprite_a_square_below_the_player_blocks_the_way_down() {
        let mut sprites = [SpriteState::default(); NUM_SPRITES];
        sprites[0] = walker(0x3C, 0x40, 1, 0);
        sprites[1] = walker(0x4C, 0x40, 0, 0);
        detect_collision_between_sprites(&mut sprites, 0);
        assert_eq!(sprites[0].collision_data & 4, 4, "down");
        assert_eq!(sprites[0].collided_with, [0, 0b10]);
    }

    #[test]
    fn a_sprite_two_squares_away_is_not_in_the_way() {
        let mut sprites = [SpriteState::default(); NUM_SPRITES];
        sprites[0] = walker(0x3C, 0x40, 1, 0);
        sprites[1] = walker(0x5C, 0x40, 0, 0);
        detect_collision_between_sprites(&mut sprites, 0);
        assert_eq!(sprites[0].collision_data, 0);
    }
}
