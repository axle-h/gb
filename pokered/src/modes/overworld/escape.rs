//! `ItemUseEscapeRope`, which DIG and TELEPORT reach as well, and the spinning halves of
//! `_LeaveMapAnim` and `EnterMapAnim` that carry an escape warp, a warp pad and a hole.
//!
//! The spin is the player's own sprite cycled through `PlayerSpinningFacingOrder` from whichever way
//! they already face. `PlayerSpinInPlace` counts a delay towards an end value rather than a number
//! of frames, so the escape's 16 down to 1 is 120 frames and the arrival's 1 up to 7 is 28, and
//! neither delays on the iteration that reaches the end.

use poke_core::map::Map;
use poke_core::map_objects::WarpTo;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_symbols;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::mode::Ctx;
use super::script::{Block, Flow, Routine, Then};
use super::Overworld;

/// `wStandingOnWarpPadOrHole`.
pub const NEITHER: u8 = 0;
pub const WARP_PAD: u8 = 1;
pub const HOLE: u8 = 2;

/// `PlayerSpinningFacingOrder`, which is the sprite facings in the order a spin runs them.
const SPIN_ORDER: [u8; 4] = [0x0, 0x8, 0x4, 0xC];
/// `GetPlayerTeleportAnimFrameDelay` off an SGB.
const SPIN_STEP_FRAMES: u8 = 3;
/// `PlayerSpinWhileMovingUpOrDown`'s two runs: up off the top of the screen, and down to where the
/// player stands.
const UP_DELTA: u8 = 0xF0;
const UP_MAX_Y: u8 = 0xEC;
const DOWN_DELTA: u8 = 0x10;
const DOWN_MAX_Y: u8 = 0x3C;
/// `_LeaveMapAnim`'s extra delay where the player is not leaving from a warp pad.
const EXTRA_LEAVE_FRAMES: u8 = 10;
/// `.dungeonWarpAnimation`'s `DelayFrames 50` before the player drops in.
const DUNGEON_ENTER_FRAMES: u8 = 50;

/// `wFacingDirectionList` and the bytes `PlayerSpinInPlace` and `PlayerSpinWhileMovingUpOrDown`
/// count on. One struct because only one spin ever runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SpinAnim {
    /// Where in `SPIN_ORDER` the next frame's facing comes from.
    at: u8,
    /// `wPlayerSpinInPlaceAnimFrameDelay`, its delta and its end value.
    delay: u8,
    delta: u8,
    end: u8,
    /// `wPlayerSpinInPlaceAnimSoundID`, or `$ff` for a silent spin.
    sound: u8,
    /// `wPlayerSpinWhileMovingUpOrDownAnimDeltaY` and its max.
    delta_y: u8,
    max_y: u8,
}

/// `EscapeRopeTilesets` with `ItemUseEscapeRope`'s refusal of Agatha's room ahead of it.
pub fn escape_rope_allowed(map: Map, tileset: TileSetId) -> bool {
    map != Map::AgathasRoom
        && rom_slice(pokered_symbols::EscapeRopeTilesets).iter().take_while(|&&id| id != 0xFF)
            .any(|&id| id == tileset as u8)
}

/// `ItemUseEscapeRope` where the map allows it: the escape warp armed, and the Safari Zone left
/// behind. What the cartridge does with `wEscapedFromBattle` matters only inside a battle.
pub fn arm_escape_warp(ctx: &mut Ctx) {
    ctx.world.location.fly_warp = Some(ctx.world.location.last_blackout_map);
    ctx.world.location.escape_warp = true;
    ctx.world.events.clear(poke_core::symbols::pokered_events::EVENT_IN_SAFARI_ZONE);
    ctx.world.safari_balls = 0;
}

/// `DungeonWarpList` and `DungeonWarpData`: where a hole on `map`'s floor above drops the player.
/// Each row of the list names a destination map and which of its holes, and the data beside it has
/// the same shape as a fly warp's.
pub fn dungeon_warp(map: Map, which: u8) -> Option<WarpTo> {
    const ENTRY: u16 = 6;
    let list = rom_slice(pokered_symbols::DungeonWarpList);
    let at = list.chunks(2).take_while(|row| row[0] != 0xFF)
        .position(|row| row[0] == map as u8 && row[1] == which)? as u16;
    let data = rom_slice(pokered_symbols::DungeonWarpData + at * ENTRY);
    Some(WarpTo { view: u16::from_le_bytes([data[0], data[1]]), y: data[2], x: data[3] })
}

/// `WarpPadAndHoleData`: what the player is standing on, by the tile at the middle of the screen.
pub fn warp_pad_or_hole(tileset: TileSetId, tile: u8) -> u8 {
    let table = rom_slice(pokered_symbols::WarpPadAndHoleData);
    table.chunks(3).take_while(|row| row[0] != 0xFF)
        .find(|row| row[0] == tileset as u8 && row[1] == tile)
        .map_or(NEITHER, |row| row[2])
}

impl Overworld {
    /// `HandleFlyWarpOrDungeonWarp` from the loop's test of `BIT_DUNGEON_WARP`: the player falls
    /// through the hole they are standing on and spins down onto the floor below.
    pub(super) fn handle_dungeon_warp(&mut self, ctx: &mut crate::mode::Ctx) -> crate::mode::Transition {
        self.update_sprites(ctx);
        let location = &mut ctx.world.location;
        location.walk_bike_surf = 0;
        location.always_on_bike = false;
        self.map_pal_offset = 0;
        self.rt.entered_by = Some(super::script::SpecialEnter::Dungeon);
        self.run_script_from(ctx, vec![Routine::SpecialWarpFaded.into(), Routine::LeaveMapAnim.into()])
    }

    /// `IsPlayerStandingOnWarpPadOrHole`, whose tile is `coord 8, 9`.
    pub(super) fn standing_on_warp_pad_or_hole(&mut self, ctx: &Ctx) -> u8 {
        let tile = self.tile_map(ctx)[9 * crate::gfx::ui::SCREEN_TILES_X + 8];
        self.standing_on_warp_pad_or_hole = warp_pad_or_hole(self.view.tileset, tile);
        self.standing_on_warp_pad_or_hole
    }

    /// `InitFacingDirectionList`, which finds the player's facing in the order a spin runs.
    pub(super) fn init_facing_direction_list(&mut self) {
        let facing = self.sprites[0].image_index;
        self.rt.spin.at = SPIN_ORDER.iter().position(|&one| one == facing).unwrap_or(0) as u8;
    }

    /// `SpinPlayerSprite`.
    fn spin_player_sprite(&mut self) {
        let anim = &mut self.rt.spin;
        self.sprites[0].image_index = SPIN_ORDER[anim.at as usize % 4];
        anim.at = (anim.at + 1) % 4;
    }

    /// `PlayerSpinInPlace`, armed for the escape warp's spin out.
    pub(super) fn spin_out_in_place(&mut self) -> Flow {
        self.rt.spin.delay = 16;
        self.rt.spin.delta = 0xFF;
        self.rt.spin.end = 0;
        self.rt.spin.sound = sounds::SFX_TELEPORT_EXIT_2.0;
        Flow::Jump(Routine::SpinInPlace.into())
    }

    /// One iteration of `PlayerSpinInPlace`: the sound every fourth delay, and no delay at all on
    /// the iteration whose new delay is the end value.
    pub(super) fn spin_in_place(&mut self, ctx: &mut Ctx) -> Flow {
        self.spin_player_sprite();
        let anim = &mut self.rt.spin;
        if anim.delay % 4 == 0 && anim.sound != 0xFF {
            ctx.audio.play_sound(crate::audio::data::SoundId(anim.sound));
        }
        anim.delay = anim.delay.wrapping_add(anim.delta);
        if anim.delay == anim.end {
            return Flow::Return;
        }
        Then::block(Block::Frames(anim.delay)).then(Routine::SpinInPlace)
    }

    /// One iteration of `PlayerSpinWhileMovingUpOrDown`.
    pub(super) fn spin_while_moving(&mut self) -> Flow {
        self.spin_player_sprite();
        let anim = self.rt.spin;
        let y = self.sprites[0].y_pixels.wrapping_add(anim.delta_y);
        self.sprites[0].y_pixels = y;
        if y == anim.max_y {
            return Flow::Return;
        }
        Then::block(Block::Frames(SPIN_STEP_FRAMES)).then(Routine::SpinWhileMoving)
    }
}

/// The leaving half.
impl Overworld {
    /// `_LeaveMapAnim`'s `.spinWhileMovingUp`.
    pub(super) fn spin_while_moving_up(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.audio.play_sound(sounds::SFX_TELEPORT_EXIT_1);
        self.rt.spin.delta_y = UP_DELTA;
        self.rt.spin.max_y = UP_MAX_Y;
        Then::call(Routine::SpinWhileMoving).then(Routine::LeaveMapAnimSpun)
    }

    /// The extra ten frames a leave that did not start on a warp pad takes, then the fade.
    pub(super) fn leave_map_anim_spun(&mut self, ctx: &mut Ctx) -> Flow {
        let frames = if self.standing_on_warp_pad_or_hole(ctx) == WARP_PAD { 0 } else { EXTRA_LEAVE_FRAMES };
        Then::block(Block::Frames(frames)).then(Routine::FadeOutToWhite(super::fly::WHITE - 2))
    }

    /// `LeaveMapThroughHoleAnim`: the player's top half dropped a row, then hidden, then the fade.
    pub(super) fn leave_map_through_hole(&mut self) -> Flow {
        self.rt.sprites_frozen = true;
        self.sprites[0].y_pixels = self.sprites[0].y_pixels.wrapping_add(8);
        Then::block(Block::Frames(2)).then(Routine::LeaveMapThroughHoleHidden)
    }

    pub(super) fn leave_map_through_hole_hidden(&mut self) -> Flow {
        self.sprites[0].y_pixels = 0xA0;
        Then::call(Routine::FadeOutToWhite(super::fly::WHITE - 2)).then(Routine::LeaveMapThroughHoleDone)
    }

    pub(super) fn leave_map_through_hole_done(&mut self) -> Flow {
        self.rt.sprites_frozen = false;
        Flow::Return
    }
}

/// The arriving half.
impl Overworld {
    /// `EnterMapAnim`'s spin, which a dungeon warp delays before and skips the rest of.
    pub(super) fn enter_map_anim_spin(&mut self, ctx: &mut Ctx, dungeon: bool) -> Flow {
        ctx.audio.play_sound(sounds::SFX_TELEPORT_ENTER_1);
        self.rt.spin.delta_y = DOWN_DELTA;
        self.rt.spin.max_y = DOWN_MAX_Y;
        let then = if dungeon { Routine::EnterMapAnimDone } else { Routine::EnterMapAnimSpun };
        let down = Then::call(Routine::SpinWhileMoving).then(then);
        if dungeon { Then::block(Block::Frames(DUNGEON_ENTER_FRAMES)).then(Routine::EnterMapAnimDungeon) } else { down }
    }

    /// `.dungeonWarpAnimation` after its delay.
    pub(super) fn enter_map_anim_dungeon(&mut self) -> Flow {
        Then::call(Routine::SpinWhileMoving).then(Routine::EnterMapAnimDone)
    }

    /// Landing after the spin down: a warp pad or a hole is stood on and done with, and anywhere
    /// else the player spins to a stop and the map's music comes back.
    pub(super) fn enter_map_anim_spun(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.audio.play_sound(sounds::SFX_TELEPORT_ENTER_2);
        if self.standing_on_warp_pad_or_hole(ctx) != NEITHER {
            return Flow::Jump(Routine::EnterMapAnimDone.into());
        }
        self.rt.spin.delay = 0;
        self.rt.spin.delta = 1;
        self.rt.spin.end = 8;
        self.rt.spin.sound = 0xFF;
        Then::call(Routine::SpinInPlace).then(Routine::EnterMapAnimMusic)
    }

    pub(super) fn enter_map_anim_music(&mut self) -> Flow {
        Then::call(Routine::PlayDefaultMusic).then(Routine::EnterMapAnimDone)
    }
}

#[cfg(test)]
mod tests {
    use poke_core::map_objects::fly_warp;
    use crate::command::Decision;
    use crate::mode::{Mode, ModeUpdate, Status};
    use crate::rng::GameRng;
    use crate::systems::overworld::location::{Location, BIKING};
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    /// Mount Moon, on the bike, with an escape warp armed for Pewter City.
    fn escaping() -> Game {
        let location = Location {
            map: Map::MtMoon1F, x: 15, y: 17, walk_bike_surf: BIKING, always_on_bike: true,
            last_blackout_map: Map::PewterCity, fly_warp: Some(Map::PewterCity), escape_warp: true,
            ..Location::default()
        };
        let world = World { location, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        game.push(Mode::Overworld(overworld));
        game
    }

    fn settle(game: &mut Game) -> u32 {
        for frames in 1..3000 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::Overworld) {
                return frames;
            }
        }
        panic!("the escape never landed: {:?}", game.modes().last().map(Mode::status));
    }

    #[test]
    fn an_escape_warp_lands_on_the_last_blackout_town_and_takes_the_bike_away() {
        let mut game = escaping();
        settle(&mut game);
        let location = &game.world().location;
        let warp = fly_warp(Map::PewterCity).unwrap();
        assert_eq!((location.map, location.x, location.y), (Map::PewterCity, warp.x, warp.y));
        assert_eq!((location.walk_bike_surf, location.always_on_bike), (0, false));
        assert_eq!((location.fly_warp, location.escape_warp), (None, false), "and both flags are spent");
    }

    /// The spin out counts 16 down to 1, the spin up takes four steps of three frames with ten more
    /// after it, and the arrival is twelve frames down and a spin of 1 up to 7.
    #[test]
    fn the_spins_take_the_frames_their_counters_give_them() {
        let out: u32 = (1..16).sum();
        let up = 4 * SPIN_STEP_FRAMES as u32 + EXTRA_LEAVE_FRAMES as u32;
        let back: u32 = 4 * SPIN_STEP_FRAMES as u32 + (1..8).sum::<u32>();
        let fades = 6 * super::super::FADE_FRAMES as u32;
        let mut game = escaping();
        let frames = settle(&mut game);
        assert!(frames > out + up + back + fades, "{frames} frames is less than the spins alone");
    }

    /// Seafoam Islands 1F's first hole, over the boulder hole at (17, 6).
    #[test]
    fn a_hole_drops_the_player_onto_the_floor_below_without_moving_the_last_map() {
        let location = Location {
            map: Map::SeafoamIslands1F, x: 17, y: 6, last_map: Map::Route20, ..Location::default()
        };
        let world = World { location, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(3), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        overworld.rt.dungeon_warp = Some((Map::SeafoamIslandsB1F, 1));
        game.push(Mode::Overworld(overworld));
        settle(&mut game);
        let landing = dungeon_warp(Map::SeafoamIslandsB1F, 1).expect("the first hole is in the list");
        let location = &game.world().location;
        assert_eq!((location.map, location.x, location.y), (Map::SeafoamIslandsB1F, landing.x, landing.y));
        assert_eq!(location.last_map, Map::Route20, "a dungeon warp is the one that leaves `wLastMap` alone");
    }

    #[test]
    fn the_cartridge_s_two_hole_tiles_and_two_warp_pads_are_the_whole_table() {
        use poke_core::map_header::TileSetId;
        assert_eq!(warp_pad_or_hole(TileSetId::Cavern, 0x22), HOLE);
        assert_eq!(warp_pad_or_hole(TileSetId::Facility, 0x11), HOLE);
        assert_eq!(warp_pad_or_hole(TileSetId::Facility, 0x20), WARP_PAD);
        assert_eq!(warp_pad_or_hole(TileSetId::Interior, 0x55), WARP_PAD);
        assert_eq!(warp_pad_or_hole(TileSetId::Overworld, 0x20), NEITHER, "the tileset has to match too");
    }

    #[test]
    fn a_save_mid_spin_resumes_identically() {
        let mut whole = escaping();
        for _ in 0..90 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..400 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events, a.status), (restored.ui(), b.events, b.status), "frame {frame}");
        }
        assert_eq!(restored.world().location.map, Map::PewterCity);
    }
}
