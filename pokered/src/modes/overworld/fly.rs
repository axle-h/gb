//! `HandleFlyWarpOrDungeonWarp` and the two halves of the flying animation, `_LeaveMapAnim`'s and
//! `EnterMapAnim`'s.
//!
//! The bird is the player's own sprite with the bird's graphics loaded over it: `DoFlyAnimation`
//! flaps it by flipping the bottom bit of its image index every three frames and, once it is in the
//! air, moves it along a table of screen coordinates a step per flap. Every frame count here is
//! exact; the `Delay3` between `UpdateSprites` and the animation is loading.
//!
//! Where the player lands is `FlyWarpDataPtr`'s entry for the town, which is the same table a
//! blackout returns on.

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::map_objects::fly_warp;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::{pokered_symbols, DmgPointer};
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::input::Joypad;
use crate::mode::{Ctx, Transition};
use crate::systems::overworld::map_view::MapView;
use crate::systems::overworld::sprites::{load_bird_sprite_graphics, load_player_sprite_graphics, SpriteState};
use super::battles::SPECIAL_ENTER_MAP_FRAMES;
use super::script::{Block, Flow, Routine, SpecialEnter, Then};
use super::Overworld;

/// `DoFlyAnimation`'s `Delay3` a flap.
const FLAP_FRAMES: u8 = 3;
/// The flaps of each run: in place before taking off, along `FlyAnimationScreenCoords1`, along
/// `FlyAnimationScreenCoords2` (one flap past the table's end, onto the `$f0` that follows it), and
/// along `FlyAnimationEnterScreenCoords`.
const FLAPS_IN_PLACE: u8 = 8;
const FLAPS_UP: u8 = 12;
const FLAPS_AWAY: u8 = 11;
const FLAPS_IN: u8 = 12;
/// `.flyAnimation`'s `DelayFrames 40` while the bird is off the top of the screen.
const OFF_SCREEN_FRAMES: u8 = 40;
/// `wFlyAnimBirdSpriteImageIndex`: facing right for the flight out and in, left for the flight away.
const FACING_RIGHT: u8 = 0x08;
const FACING_LEFT: u8 = 0x0C;
/// `_LeaveMapAnim`'s `StopMusic` argument.
const STOP_MUSIC_FADE: u8 = 4;
/// `GBFadeOutToWhite` runs `FadePal6` to `FadePal8` and `GBFadeInFromWhite` `FadePal7` to `FadePal5`,
/// eight frames each.
pub(super) const WHITE: u8 = 8;
const FADE_IN_LAST: u8 = 5;
/// `EnterMapAnim`'s first act: the player parked above the screen until the bird brings them down.
const OFF_SCREEN_Y: u8 = 0xEC;

/// `DoFlyAnimation`'s three bytes of state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FlyAnim {
    /// `wFlyAnimCounter`.
    counter: u8,
    /// `wFlyAnimBirdSpriteImageIndex`.
    image: u8,
    /// `wFlyAnimUsingCoordList`: where in a table of `y, x` pairs, or `None` to flap in place.
    coords: Option<(DmgPointer, u8)>,
}

/// `InitFacingDirectionList` and `RestoreFacingDirectionAndYScreenPos`: what the animation borrows
/// the player's sprite for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SavedPlayer {
    facing: u8,
    y: u8,
}

impl Overworld {
    /// `HandleFlyWarpOrDungeonWarp`, from the loop's test of `BIT_FLY_WARP`.
    pub(super) fn handle_fly_warp(&mut self, ctx: &mut Ctx, destination: Map) -> Transition {
        self.update_sprites(ctx);
        let location = &mut ctx.world.location;
        location.walk_bike_surf = 0;
        location.always_on_bike = false;
        self.map_pal_offset = 0;
        self.rt.fly_destination = Some(destination);
        // The escape rope, DIG and TELEPORT clear `BIT_NO_BATTLES` where they arm the warp, which
        // only the overworld holds.
        if ctx.world.location.escape_warp {
            self.rt.no_battles = false;
        }
        self.rt.entered_by = Some(if ctx.world.location.escape_warp { SpecialEnter::Spin } else { SpecialEnter::Fly });
        self.run_script_from(ctx, vec![Routine::SpecialWarpFaded.into(), Routine::LeaveMapAnim.into()])
    }

    /// `_LeaveMapAnim`: a warp pad spins the player up on the spot, a hole drops them through, and
    /// anywhere else the music is stopped and waited out first.
    pub(super) fn leave_map_anim(&mut self, ctx: &mut Ctx) -> Flow {
        self.rt.saved_player = SavedPlayer { facing: self.sprites[0].image_index, y: self.sprites[0].y_pixels };
        self.init_facing_direction_list();
        match self.standing_on_warp_pad_or_hole(ctx) {
            super::escape::HOLE => return Flow::Jump(Routine::LeaveMapThroughHole.into()),
            super::escape::WARP_PAD => return Flow::Jump(Routine::SpinWhileMovingUp.into()),
            _ => {}
        }
        ctx.audio.stop_music(STOP_MUSIC_FADE);
        Then::block(Block::MusicStopped).then(Routine::LeaveMapAnimStopped)
    }

    /// Which animation the stopped music leads into: the escape warp's spin, or the bird.
    pub(super) fn leave_map_anim_stopped(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.audio.stop_all_sounds();
        if ctx.world.location.escape_warp {
            return Then::call(Routine::SpinOutInPlace).then(Routine::SpinWhileMovingUp);
        }
        Flow::Jump(Routine::LeaveMapAnimFlap.into())
    }

    /// `.flyAnimation`: the bird flapping where the player stands.
    pub(super) fn leave_map_anim_flap(&mut self, ctx: &mut Ctx) -> Flow {
        load_bird_sprite_graphics(&mut ctx.screen.tiles);
        self.rt.fly_anim = FlyAnim { counter: FLAPS_IN_PLACE, image: FACING_LEFT, coords: None };
        Then::call(Routine::FlyAnimStep).then(Routine::LeaveMapAnimUp)
    }

    /// The flight up and off the top of the screen.
    pub(super) fn leave_map_anim_up(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.audio.play_sound(sounds::SFX_FLY);
        self.rt.fly_anim = FlyAnim {
            counter: FLAPS_UP,
            image: FACING_LEFT,
            coords: Some((pokered_symbols::FlyAnimationScreenCoords1, 0)),
        };
        Then::call(Routine::FlyAnimStep).then(Routine::LeaveMapAnimWait)
    }

    pub(super) fn leave_map_anim_wait(&mut self) -> Flow {
        Then::block(Block::Frames(OFF_SCREEN_FRAMES)).then(Routine::LeaveMapAnimAway)
    }

    /// The flight across the sky, and the fade out behind it.
    pub(super) fn leave_map_anim_away(&mut self) -> Flow {
        self.rt.fly_anim = FlyAnim {
            counter: FLAPS_AWAY,
            image: FACING_RIGHT,
            coords: Some((pokered_symbols::FlyAnimationScreenCoords2, 0)),
        };
        Then::call(Routine::FlyAnimStep).then(Routine::FadeOutToWhite(WHITE - 2))
    }

    /// One palette of `GBFadeOutToWhite`, which returns to whoever called the leave animation.
    pub(super) fn fade_out_to_white(&mut self, ctx: &mut Ctx, palette: u8) -> Flow {
        self.set_fade_palette(ctx, palette);
        if palette == WHITE {
            self.restore_facing_direction_and_y_screen_pos();
            return Then::block(Block::Frames(super::FADE_FRAMES)).ret();
        }
        Then::block(Block::Frames(super::FADE_FRAMES)).then(Routine::FadeOutToWhite(palette + 1))
    }

    /// `PrepareForSpecialWarp` and `SpecialEnterMap` up to its wait. An escape warp lands on
    /// `wLastBlackoutMap` rather than `wDestinationMap`, and either way the square is the town's own
    /// entry in `FlyWarpDataPtr`, the same table a blackout returns on.
    pub(super) fn special_warp_faded(&mut self, ctx: &mut Ctx) -> Flow {
        let dungeon = self.rt.dungeon_warp.take();
        let destination = match dungeon {
            Some((map, _)) => map,
            None => self.rt.fly_destination.take().expect("a special warp has a destination"),
        };
        let location = &mut ctx.world.location;
        location.escape_warp = false;
        location.map = destination;
        // A dungeon warp is the one special warp that leaves `wLastMap` alone.
        if dungeon.is_none() {
            location.last_map = destination;
        }
        let warp = match dungeon {
            Some((map, which)) => super::escape::dungeon_warp(map, which).expect("a hole drops onto a dungeon warp"),
            None => fly_warp(destination).expect("a fly lands on a town with a fly warp"),
        };
        self.view.view = MapView::view_from_address(warp.view);
        location.y = warp.y;
        location.x = warp.x;
        self.view.y_block = warp.y & 1;
        self.view.x_block = warp.x & 1;
        self.view.tileset = TileSetId::Overworld;
        self.standing.destination_warp = 0xFF;
        ctx.pad.pressed = Joypad::empty();
        ctx.pad.held = Joypad::empty();
        ctx.world.play_time.counting = true;
        self.sprites[0] = SpriteState { picture_id: 1, image_base_offset: 1, y_pixels: 0x3C, x_pixels: 0x40, ..SpriteState::default() };
        Then::block(Block::Frames(SPECIAL_ENTER_MAP_FRAMES)).then(Routine::SpecialEnterMap)
    }
}

impl Overworld {
    /// `EnterMapAnim`: the player above the screen, the fade in, then the bird or the spin.
    pub(super) fn enter_map_anim(&mut self, ctx: &mut Ctx) -> Flow {
        self.rt.saved_player = SavedPlayer { facing: self.sprites[0].image_index, y: self.sprites[0].y_pixels };
        self.init_facing_direction_list();
        self.sprites[0].y_pixels = OFF_SCREEN_Y;
        self.update_sprites(ctx);
        self.fade_in_from_white(ctx, WHITE - 1)
    }

    /// One palette of `GBFadeInFromWhite`.
    pub(super) fn fade_in_from_white(&mut self, ctx: &mut Ctx, palette: u8) -> Flow {
        self.set_fade_palette(ctx, palette);
        if palette != FADE_IN_LAST {
            return Then::block(Block::Frames(super::FADE_FRAMES)).then(Routine::FadeInFromWhite(palette - 1));
        }
        let then = match self.rt.entered_by.take() {
            Some(SpecialEnter::Fly) => Routine::EnterMapAnimFly,
            Some(SpecialEnter::Dungeon) => Routine::EnterMapAnimSpinDungeon,
            _ => Routine::EnterMapAnimSpin,
        };
        Then::block(Block::Frames(super::FADE_FRAMES)).then(then)
    }

    /// `.flyAnimation`: the bird down out of the sky to where the player stands.
    pub(super) fn enter_map_anim_fly(&mut self, ctx: &mut Ctx) -> Flow {
        load_bird_sprite_graphics(&mut ctx.screen.tiles);
        ctx.audio.play_sound(sounds::SFX_FLY);
        self.rt.fly_anim = FlyAnim {
            counter: FLAPS_IN,
            image: FACING_RIGHT,
            coords: Some((pokered_symbols::FlyAnimationEnterScreenCoords, 0)),
        };
        Then::call(Routine::FlyAnimStep).then(Routine::EnterMapAnimLanded)
    }

    /// The player's own graphics back over the bird's, then the map's music.
    pub(super) fn enter_map_anim_landed(&mut self, ctx: &mut Ctx) -> Flow {
        let tileset = self.view.tileset;
        load_player_sprite_graphics(&mut ctx.screen.tiles, &mut ctx.world.location, tileset);
        Then::call(Routine::PlayDefaultMusic).then(Routine::EnterMapAnimDone)
    }

    /// `.done`, then `EnterMap`'s `UpdateSprites` after the animation.
    pub(super) fn enter_map_anim_done(&mut self, ctx: &mut Ctx) -> Flow {
        self.restore_facing_direction_and_y_screen_pos();
        self.update_sprites(ctx);
        Flow::Return
    }

    /// `DoFlyAnimation`'s flap: the image index's bottom bit flipped, held for three frames.
    pub(super) fn fly_anim_step(&mut self) -> Flow {
        self.rt.fly_anim.image ^= 1;
        self.sprites[0].image_index = self.rt.fly_anim.image;
        Then::block(Block::Frames(FLAP_FRAMES)).then(Routine::FlyAnimCoords)
    }

    /// The rest of the iteration: the next pair of screen coordinates, and back for another flap.
    pub(super) fn fly_anim_coords(&mut self) -> Flow {
        let anim = &mut self.rt.fly_anim;
        if let Some((table, at)) = anim.coords {
            let pair = rom_slice(table + at as u16 * 2);
            self.sprites[0].y_pixels = pair[0];
            self.sprites[0].x_pixels = pair[1];
            anim.coords = Some((table, at + 1));
        }
        self.rt.fly_anim.counter -= 1;
        if self.rt.fly_anim.counter == 0 { Flow::Return } else { Flow::Jump(Routine::FlyAnimStep.into()) }
    }

    /// `RestoreFacingDirectionAndYScreenPos`.
    pub(super) fn restore_facing_direction_and_y_screen_pos(&mut self) {
        let saved = self.rt.saved_player;
        self.sprites[0].y_pixels = saved.y;
        self.sprites[0].image_index = saved.facing;
    }

    pub(super) fn set_fade_palette(&self, ctx: &mut Ctx, palette: u8) {
        let [bgp, obp0, obp1] = super::fade_palette(palette);
        ctx.screen.effects.bgp = bgp;
        ctx.screen.effects.obp0 = obp0;
        ctx.screen.effects.obp1 = obp1;
    }
}

#[cfg(test)]
mod tests {
    use poke_core::sprite::SpriteFacing;
    use crate::command::Decision;
    use crate::mode::{Mode, ModeUpdate, Status};
    use crate::rng::GameRng;
    use crate::systems::overworld::location::{Location, BIKING};
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    /// Pallet Town, on the bike, with the town map already answered with `destination`.
    fn flying(destination: Map) -> Game {
        let location = Location {
            map: Map::PalletTown, x: 5, y: 9, facing: SpriteFacing::Up, walk_bike_surf: BIKING,
            always_on_bike: true, fly_warp: Some(destination), ..Location::default()
        };
        let world = World { location, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        game.push(Mode::Overworld(overworld));
        game
    }

    fn settle(game: &mut Game) -> u32 {
        for frames in 1..2000 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::Overworld) {
                return frames;
            }
        }
        panic!("the fly never landed: {:?}", game.modes().last().map(Mode::status));
    }

    #[test]
    fn a_fly_lands_on_the_town_s_own_warp_square_and_takes_the_bike_away() {
        let mut game = flying(Map::ViridianCity);
        settle(&mut game);
        let location = &game.world().location;
        let warp = fly_warp(Map::ViridianCity).unwrap();
        assert_eq!((location.map, location.x, location.y), (Map::ViridianCity, warp.x, warp.y));
        assert_eq!(location.last_map, Map::ViridianCity, "`PrepareForSpecialWarp` sets `wLastMap` too");
        assert_eq!((location.walk_bike_surf, location.always_on_bike), (0, false));
        assert_eq!(location.fly_warp, None, "and the flag is spent");
    }

    /// The bird flaps eight times in place, twelve up the screen, waits forty frames off it, flies
    /// eleven away, and both fades take three palettes of eight; the arrival is the fade and twelve
    /// more flaps.
    #[test]
    fn the_two_halves_of_the_animation_take_the_frames_the_tables_give_them() {
        let leaving = (FLAPS_IN_PLACE + FLAPS_UP + FLAPS_AWAY) as u32 * FLAP_FRAMES as u32
            + OFF_SCREEN_FRAMES as u32 + 3 * super::super::FADE_FRAMES as u32;
        let arriving = 3 * super::super::FADE_FRAMES as u32 + FLAPS_IN as u32 * FLAP_FRAMES as u32;
        let mut game = flying(Map::PewterCity);
        let frames = settle(&mut game);
        assert!(frames > leaving + SPECIAL_ENTER_MAP_FRAMES as u32 + arriving,
            "{frames} frames is less than the animation alone");
    }

    #[test]
    fn a_save_mid_flight_resumes_identically() {
        let mut whole = flying(Map::CeruleanCity);
        for _ in 0..60 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..300 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events, a.status), (restored.ui(), b.events, b.status), "frame {frame}");
        }
        assert_eq!(restored.world().location.map, Map::CeruleanCity);
    }
}
