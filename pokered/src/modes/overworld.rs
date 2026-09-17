//! `OverworldLoop`: the player walking, turning, bumping, jumping ledges, going through warps and
//! across connections, and the NPCs wandering around; START opens the start menu and A reads a
//! sign or talks to a sprite. Every pass runs the map's script (`scripts`), trainers see the player,
//! grass starts wild battles, and a lost battle blacks out to the last Pokémon Center's town.
//!
//! A pass of the loop is two frames, `DelayFrame` twice and then the body, or one from
//! `OverworldLoopLessDelay`. A step is eight passes of two pixels, so sixteen frames; an NPC moves
//! a pixel a pass and takes thirty-two. The body polls the pad where `JoypadOverworld` does, reads
//! START and A as new presses and the directions as held.
//!
//! Exact: the frame counts above, the turn a press costs when it changes direction, collision from
//! the map's tiles and the tileset's tables, the ledge jump's sixteen passes and landing, a warp's
//! 32-frame fade and the step out of a door, connections, `UpdateSprites` and `PrepareOAMData`.
//! Left out as loading: the LCD-off map load, `UpdateMusic6Times`, and the tile copies behind the
//! text box. Seams, where another system takes over: hidden events and bookshelves, poison steps,
//! boulders, a map text that dispatches to a nurse or a PC, spinners, warp pads and holes, fly and
//! dungeon warps, the Safari Zone.
//!
//! On the bike a pass advances the player twice (`DoBikeSpeedup`), so a step is eight frames. On
//! the water `CollisionCheckOnWater` replaces the land's checks, and a step onto passable land gets
//! off. The bag gets on and off the bike and the party menu's SURF onto and off the water; what
//! their `ItemUseSurfboard` leaves to the overworld is the simulated step forward, taken when the
//! start menu closes.

pub mod script;
mod battles;
pub mod escape;
mod field_moves;
mod fly;
pub mod bike_surf;
pub mod events;
mod give;
pub(crate) mod movement;
pub(crate) mod spinners;
mod trainers;

use poke_core::map::Map;
use poke_core::map_gfx::tileset_entry;
use poke_core::map_header::MapHeader;
use poke_core::map_objects::{map_song, toggleable_objects, MapObjects, Sign, Warp, FIRST_ROUTE_MAP, LAST_MAP};
use poke_core::rom_gfx::rom_slice;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_symbols;
use poke_core::tilesets::{collision_tiles, is_dungeon_tileset};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, AudioBank, Sound, SoundId};
use crate::command::{Decision, Drive, Refusal};
use crate::gfx::layers::Object;
use crate::gfx::sgb::{OverworldPalette, PaletteCommand};
use crate::gfx::ui::SCREEN_TILES_X;
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::systems::map_data::tile_block_map;
use crate::systems::overworld::bike_surf::{collision_check_on_water, forced_bike_or_surf, OnWater};
use crate::systems::overworld::collision::{self, ExtraWarp};
use crate::systems::overworld::location::{BIKING, SURFING, WALKING};
use crate::systems::overworld::map_view::{MapView, TileMap};
use crate::systems::overworld::sprites::{self, SpriteEnv, SpriteSet, SpriteState, Sprites, NUM_SPRITES};
use poke_core::symbols::pokered_events::EVENT_IN_SAFARI_ZONE;
use crate::scripts::Code;
use poke_core::symbols::pokered_map_scripts::{SCRIPT_SEAFOAMISLANDSB3F_MOVE_OBJECT, SCRIPT_SEAFOAMISLANDSB4F_MOVE_OBJECT};
use crate::systems::events::hidden_events;
use script::{Routine, Runtime, Waiting};
use crate::systems::overworld::{Direction, Location};
use crate::world::World;

const PAD_CTRL_PAD: Joypad = Joypad::UP.union(Joypad::DOWN).union(Joypad::LEFT).union(Joypad::RIGHT);
/// `IgnoreInputForHalfSecond`, less the frames `EnterMap` spends loading after it with VBlank still
/// counting them down (the wait for the LCD to turn off and `LoadPlayerSpriteGraphics`' two
/// `CopyVideoData`s). The lock then ends the same number of passes into the new map.
const IGNORE_INPUT_FRAMES: u8 = 30 - ENTER_MAP_LOADING;
const ENTER_MAP_LOADING: u8 = 5;
/// The frame a step's first pass takes beyond its own on the cartridge.
const STEP_START_LAG: u8 = 1;
/// A step's passes: `wWalkCounter` starts at 8.
const WALK_PASSES: u8 = 8;
/// `GBFadeOutToBlack`: four palettes, eight frames each.
const FADE_FRAMES: u8 = 8;
/// `.finishedJump`'s `Delay3`, a beat on landing.
const LANDING_FRAMES: u8 = 3;
/// `PlayerJumpingYScreenCoords`' length.
const JUMP_PASSES: u8 = 0x10;
/// `DisplayTextID` sets `hFrameCounter` to half a second, the text's joypad poll timer.
const TEXT_POLL_TIMER: u8 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// `OverworldLoop`: frames left to wait before the body runs.
    Loop(u8),
    /// `.finishedJump`'s wait, after which the body carries on from where it called
    /// `HandleMidJump`.
    Landing(u8),
    /// `GBFadeOutToBlack`: the palettes still to come and this one's frames, and what follows.
    FadeOut { palettes: u8, frames: u8, then: AfterFade },
    /// `MapEntryAfterBattle`'s `GBFadeInFromWhite`, then the rest of `EnterMap`.
    FadeIn { palettes: u8, frames: u8 },
    /// A script is running or waiting: a text, a menu, a battle, a delay (`script::Runtime`).
    Script,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AfterFade {
    /// `EnterMap` through the warp.
    Warp,
    /// `HandleBlackOut`'s `StopMusic`.
    BlackOut,
}

/// The pad state a fixture's player stands in, which a press is judged against: what turns and
/// what walks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Standing {
    /// `wPlayerDirection`.
    pub player_direction: u8,
    /// `wPlayerMovingDirection`.
    pub moving_direction: u8,
    /// `wPlayerLastStopDirection`.
    pub last_stop_direction: u8,
    /// `wCheckFor180DegreeTurn`.
    pub check_for_180_degree_turn: u8,
    /// `BIT_STANDING_ON_WARP`.
    pub standing_on_warp: bool,
    /// `wDestinationWarpID`, left from the last warp.
    pub destination_warp: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overworld {
    phase: Phase,
    view: MapView,
    warps: Vec<Warp>,
    signs: Vec<Sign>,
    /// `wNumSprites`.
    num_sprites: u8,
    sprites: Sprites,
    sprite_set: SpriteSet,
    /// `wToggleableObjectList`: `(sprite, global index)`.
    toggleable: Vec<(u8, u8)>,
    /// `wWalkCounter`.
    walk_counter: u8,
    standing: Standing,
    /// `BIT_STANDING_ON_DOOR`, `BIT_EXITING_DOOR` and `BIT_LEDGE_OR_FISHING`.
    standing_on_door: bool,
    /// `wStandingOnWarpPadOrHole`.
    #[serde(default)]
    standing_on_warp_pad_or_hole: u8,
    /// `BIT_SPINNING`: an arrow tile is carrying the player, so the walking animation is replaced by
    /// the spin and the arrows themselves are animated.
    #[serde(default)]
    spinning: bool,
    exiting_door: bool,
    jumping: bool,
    /// `BIT_TURNING`.
    turning: bool,
    /// `wSimulatedJoypadStatesEnd` and `wSimulatedJoypadStatesIndex`, with
    /// `BIT_SCRIPTED_MOVEMENT_STATE`.
    simulated: Vec<Joypad>,
    simulated_index: u8,
    scripted: bool,
    /// `wIgnoreInputCounter`, `BIT_DISABLE_JOYPAD` and `BIT_UNKNOWN_5_2`, which ignores A.
    ignore_input: u8,
    joypad_disabled: bool,
    a_disabled: bool,
    /// `wPlayerJumpingYScreenCoordsIndex`.
    jump_index: u8,
    /// `BIT_FONT_LOADED`.
    font_loaded: bool,
    /// `BIT_NO_NPC_FACE_PLAYER`.
    no_face_player: bool,
    /// `wMapPalOffset`.
    map_pal_offset: u8,
    /// `hWarpDestinationMap`.
    warp_destination_map: u8,
    /// The fixture's sprites, which `enter` keeps rather than loading the map's own.
    keep_sprites: bool,
    /// The last body found no button pressed with nothing under way, so a press now is a new decision.
    polled: bool,
    /// Presses answered after a text, so a driver can see its own land.
    answered: u32,
    /// `wWalkBikeSurfStateCopy`, as `DisplayStartMenu` leaves it.
    #[serde(default)]
    walk_bike_surf_copy: u8,
    /// `wCutTile`, which tells the cut animation a tree from grass.
    #[serde(default)]
    cut_tile: u8,
    /// The script runtime's stack and the WRAM it shares with the trainer and battle routines.
    rt: Runtime,
}

impl Default for Overworld {
    fn default() -> Self {
        Self::new()
    }
}

impl Overworld {
    /// Enters the map `World::location` names, as `SpecialEnterMap` does from a save.
    pub fn new() -> Self {
        let mut sprites = [SpriteState::default(); NUM_SPRITES];
        // The player's sprite never moves on the screen: the map and the other sprites do.
        sprites[0] = SpriteState { picture_id: 1, image_base_offset: 1, y_pixels: 0x3C, x_pixels: 0x40, ..SpriteState::default() };
        Self {
            phase: Phase::Loop(1),
            view: MapView::default(),
            warps: Vec::new(),
            signs: Vec::new(),
            num_sprites: 0,
            sprites,
            sprite_set: SpriteSet::default(),
            toggleable: Vec::new(),
            walk_counter: 0,
            standing: Standing { destination_warp: 0xFF, check_for_180_degree_turn: 1, ..Standing::default() },
            standing_on_door: false,
            standing_on_warp_pad_or_hole: 0,
            spinning: false,
            exiting_door: false,
            jumping: false,
            turning: false,
            simulated: Vec::new(),
            simulated_index: 0,
            scripted: false,
            ignore_input: 0,
            joypad_disabled: false,
            a_disabled: false,
            jump_index: 0,
            font_loaded: false,
            no_face_player: false,
            map_pal_offset: 0,
            warp_destination_map: 0,
            keep_sprites: false,
            polled: false,
            answered: 0,
            walk_bike_surf_copy: WALKING,
            cut_tile: 0,
            rt: Runtime::default(),
        }
    }

    /// Standing where a running game left the player, with its sprites mid-wander: `sprites` are
    /// the sixteen slots as the cartridge has them.
    pub fn standing(sprites: Sprites, num_sprites: u8, standing: Standing) -> Self {
        Self { sprites, num_sprites, standing, keep_sprites: true, ..Self::new() }
    }

    /// `BIT_NO_BATTLES`, `BIT_WILD_ENCOUNTER_COOLDOWN` and `wNumberOfNoRandomBattleStepsLeft`, as a
    /// running game left them.
    pub fn with_battle_flags(mut self, no_battles: bool, cooldown: bool, steps_left: u8) -> Self {
        self.rt.no_battles = no_battles;
        self.rt.wild_encounter_cooldown = cooldown;
        self.rt.no_random_battle_steps = steps_left;
        self
    }

    /// `wStepCounter`, which poison and the Safari Zone count on.
    pub fn with_step_counter(mut self, steps: u8) -> Self {
        self.rt.step_counter = steps;
        self
    }

    pub fn sprites(&self) -> &Sprites {
        &self.sprites
    }

    pub fn num_sprites(&self) -> u8 {
        self.num_sprites
    }

    pub fn view(&self) -> &MapView {
        &self.view
    }

    pub fn walk_counter(&self) -> u8 {
        self.walk_counter
    }

    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// Mid-step, mid-jump, fading or exiting a door: something the player started is under way.
    pub fn moving(&self) -> bool {
        self.walk_counter != 0 || self.jumping || self.scripted || matches!(self.phase, Phase::FadeOut { .. } | Phase::Landing(_))
    }

    pub fn is_turning(&self) -> bool {
        self.turning
    }

    fn player(&self) -> &SpriteState {
        &self.sprites[0]
    }

    /// `wTileMap`: the UI where it covers the map, else the map through the view.
    fn tile_map(&self, ctx: &Ctx) -> TileMap {
        let mut tiles = self.view.tile_map();
        for (i, tile) in tiles.iter_mut().enumerate() {
            if let Some(cover) = ctx.screen.ui.cover(i % SCREEN_TILES_X, i / SCREEN_TILES_X) {
                *tile = cover;
            }
        }
        tiles
    }

    fn header(map: Map) -> MapHeader {
        MapHeader::read(map).expect("the player stands on a map with a header")
    }

    fn hidden(&self, location: &Location) -> [bool; NUM_SPRITES] {
        std::array::from_fn(|slot| {
            self.toggleable.iter().any(|&(sprite, global)| sprite as usize == slot && location.is_hidden(global))
        })
    }

    /// `UpdateSprites`.
    fn update_sprites(&mut self, ctx: &mut Ctx) {
        if self.rt.sprites_frozen {
            return;
        }
        let tiles = self.tile_map(ctx);
        let surrounding = self.view.surrounding_tiles();
        let tileset = self.view.tileset;
        let collision = collision_tiles(tileset);
        let env = SpriteEnv {
            tiles: &tiles,
            x: ctx.world.location.x,
            y: ctx.world.location.y,
            walk_counter: self.walk_counter,
            font_loaded: self.font_loaded,
            collision: &collision,
            grass_tile: tileset_entry(tileset).grass_tile,
            hidden: self.hidden(&ctx.world.location),
            no_face_player: self.no_face_player,
            player_direction: self.standing.player_direction,
            moving_direction: self.standing.moving_direction,
            spinning: self.spinning,
            simulating: self.scripted,
            beyond: Some(&surrounding),
        };
        sprites::update_sprites(&mut self.sprites, &env, &mut self.rt.paths, ctx.rng);
        if std::mem::take(&mut self.rt.paths.path_ended) {
            self.simulated_index = 0;
        }
        if let Some(facing) = SpriteFacing::from_repr(self.player().facing) {
            ctx.world.location.facing = facing;
        }
    }

    /// `PrepareOAMData` and the screen's view of the map, which VBlank takes every frame.
    /// `UpdateSprites` for a mode drawn over the overworld, and the objects into OAM.
    pub(crate) fn update_sprites_under(&mut self, ctx: &mut Ctx) {
        // A battle clears `wUpdateSpritesEnabled`, so its bag's list updates nothing.
        if self.rt.waiting == Waiting::Battle {
            return;
        }
        self.update_sprites(ctx);
        if !self.rt.sprites_frozen {
            let mut objects = std::mem::take(&mut ctx.screen.sprites);
            sprites::prepare_oam(&mut self.sprites, &mut objects, self.jumping);
            ctx.screen.sprites = objects;
        }
    }

    fn present(&mut self, ctx: &mut Ctx) {
        if !self.rt.sprites_frozen {
            let mut objects = std::mem::take(&mut ctx.screen.sprites);
            sprites::prepare_oam(&mut self.sprites, &mut objects, self.jumping);
            ctx.screen.sprites = objects;
        }
        let (mut x, mut y) = self.view.camera();
        if self.walk_counter != 0 {
            // `hSCX`/`hSCY`: the view has already moved a half block, the scroll catches it up.
            let offset = 2 * (WALK_PASSES - self.walk_counter) as i32 - 16;
            x += self.player().x_step as i8 as i32 * offset;
            y += self.player().y_step as i8 as i32 * offset;
        }
        ctx.screen.map.tileset = Some(self.view.tileset);
        ctx.screen.map.blocks_wide = self.view.stride() as usize;
        if ctx.screen.map.blocks != self.view.blocks {
            ctx.screen.map.blocks = self.view.blocks.clone();
        }
        ctx.screen.map.camera = (x, y);
    }

    /// `LoadGBPal`.
    fn load_gb_pal(&self, ctx: &mut Ctx) {
        let [bgp, obp0, obp1] = fade_palette(4 - self.map_pal_offset / 3);
        ctx.screen.effects.bgp = bgp;
        ctx.screen.effects.obp0 = obp0;
        ctx.screen.effects.obp1 = obp1;
    }

    // ---- EnterMap and the loads behind it ----

    /// `EnterMap`, from `LoadMapData` to the loop.
    fn enter_map(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.pad.ignore = Joypad::all();
        self.load_map_data(ctx);
        // `ClearVariablesOnEnterMap`.
        ctx.pad.held = Joypad::empty();
        ctx.pad.pressed = Joypad::empty();
        ctx.pad.released = Joypad::empty();
        if self.rt.wild_encounter_cooldown {
            self.rt.no_random_battle_steps = battles::NO_RANDOM_BATTLE_STEPS;
        }
        self.rt.cur_map_loaded = [true; 2];
        // `ResetUsingStrengthOutOfBattleBit`: STRENGTH lasts only as long as the map it was used on,
        // and a map re-entered after a battle is the same map.
        if !self.rt.battle_over_or_blackout {
            ctx.world.location.strength_active = false;
        }
        if std::mem::take(&mut self.rt.battle_over_or_blackout) {
            // `MapEntryAfterBattle`.
            self.is_player_standing_on_warp(ctx);
            if self.map_pal_offset == 0 {
                return self.fade_in_step(ctx, 7);
            }
            self.load_gb_pal(ctx);
        }
        // `res BIT_FLY_WARP`, then `EnterMapAnim`, which takes the flag itself once it knows which
        // arrival to run.
        if self.rt.entered_by.is_some() {
            return self.run_script_from(ctx, vec![Routine::EnterMapRest.into(), Routine::EnterMapAnim.into()]);
        }
        self.enter_map_rest(ctx)
    }

    /// `EnterMap` from `CheckForceBikeOrSurf` to the loop.
    fn enter_map_rest(&mut self, ctx: &mut Ctx) -> Transition {
        let location = &mut ctx.world.location;
        if !location.always_on_bike && let Some(state) = forced_bike_or_surf(location.map, location.x, location.y) {
            let map = location.map;
            location.always_on_bike |= state == BIKING;
            location.walk_bike_surf = state;
            // `ForceBikeOrSurf`.
            sprites::load_player_sprite_graphics(&mut ctx.screen.tiles, location, self.view.tileset);
            // Every square in the table arms Seafoam B3F's move-object script on its way past, and
            // every square but B3F's arms B4F's too, so a Cycling Road gate arms both floors.
            let scripts = &mut ctx.world.scripts.maps;
            scripts.seafoam_islands_b3f.cur_script = SCRIPT_SEAFOAMISLANDSB3F_MOVE_OBJECT;
            if map != poke_core::map::Map::SeafoamIslandsB3F {
                scripts.seafoam_islands_b4f.cur_script = SCRIPT_SEAFOAMISLANDSB4F_MOVE_OBJECT;
            }
            return self.run_script_from(ctx, vec![Routine::EnterMapEnd.into(), Routine::PlayDefaultMusic.into()]);
        }
        self.enter_map_end(ctx)
    }

    /// `EnterMap` after `CheckForceBikeOrSurf`.
    pub(super) fn enter_map_end(&mut self, ctx: &mut Ctx) -> Transition {
        self.no_face_player = false;
        self.update_sprites(ctx);
        ctx.pad.ignore = Joypad::empty();
        self.phase = Phase::Loop(1);
        Transition::Stay
    }

    /// One palette of `GBFadeInFromWhite`, `FadePal7` up to `FadePal5`.
    fn fade_in_step(&mut self, ctx: &mut Ctx, palette: u8) -> Transition {
        let [bgp, obp0, obp1] = fade_palette(palette);
        ctx.screen.effects.bgp = bgp;
        ctx.screen.effects.obp0 = obp0;
        ctx.screen.effects.obp1 = obp1;
        self.phase = Phase::FadeIn { palettes: palette - 5, frames: FADE_FRAMES };
        Transition::Stay
    }

    /// `LoadMapData`. The LCD is off for all of it, which is loading.
    fn load_map_data(&mut self, ctx: &mut Ctx) {
        if self.rt.battle_over_or_blackout {
            // Battles leave the sprites' state as it was.
            self.keep_sprites = true;
        }
        self.walk_counter = 0;
        self.sprite_set.id = 0;
        // The map view is copied over the whole of the screen.
        ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, crate::gfx::ui::SCREEN_TILES_Y);
        ctx.screen.tiles.load_text_box_tiles();
        self.load_map_header(ctx);
        let location = &ctx.world.location;
        sprites::init_map_sprites(&mut self.sprites, &mut self.sprite_set, location.map, location.x, location.y,
            self.num_sprites, self.font_loaded, &mut ctx.screen.tiles);
        self.view.blocks = tile_block_map(location.map).expect("the map has blocks");
        ctx.screen.tiles.load_tileset(self.view.tileset);
        self.set_pal_overworld(ctx);
        sprites::load_player_sprite_graphics(&mut ctx.screen.tiles, &mut ctx.world.location, self.view.tileset);
        // A fly warp leaves the music to `EnterMapAnim`, which plays it once the bird has landed.
        if !self.rt.no_map_music && self.rt.entered_by.is_none() {
            play_default_music_fade_out_current(ctx, self.rt.battle_over_or_blackout);
        }
    }

    fn set_pal_overworld(&self, ctx: &mut Ctx) {
        let location = &ctx.world.location;
        let palette = OverworldPalette { map: location.map, tileset: self.view.tileset, last_map: location.last_map };
        ctx.screen.sgb.run(&PaletteCommand::Overworld(palette));
    }

    /// `LoadMapHeader`, with `MarkTownVisitedAndLoadToggleableObjects` and `LoadTilesetHeader`.
    fn load_map_header(&mut self, ctx: &mut Ctx) {
        let location = &mut ctx.world.location;
        let map = location.map;
        if (map as u8) < FIRST_ROUTE_MAP {
            location.towns_visited |= 1 << map as u8;
        }
        self.toggleable = toggleable_objects(map);
        let previous_tileset = self.view.tileset;
        let header = Self::header(map);
        let objects = MapObjects::read(map).expect("the map has objects");
        self.view.tileset = header.tileset;
        self.view.width = header.width;
        self.view.height = header.height;
        self.warps = objects.warps.clone();
        self.signs = objects.signs.clone();
        if std::mem::take(&mut self.keep_sprites) {
            // The fixture's slots already hold these objects, wherever they have wandered to.
        } else {
            self.num_sprites = objects.objects.len() as u8;
            for (slot, sprite) in self.sprites.iter_mut().enumerate().skip(1) {
                *sprite = SpriteState { image_index: 0xFF, ..SpriteState::default() };
                if let Some(object) = objects.objects.get(slot - 1) {
                    sprite.picture_id = object.picture;
                    sprite.map_y = object.map_y;
                    sprite.map_x = object.map_x;
                    sprite.movement1 = object.movement1;
                    sprite.movement2 = object.movement2;
                    sprite.text_id = object.text_id;
                }
            }
        }
        self.rt.wild_mons.load(map);
        self.rt.text_pointers = None;
        let entry = tileset_entry(header.tileset);
        ctx.screen.tiles.animation.kind = entry.animation;
        ctx.screen.tiles.animation.counter1 = 0;
        let warp = self.standing.destination_warp;
        if (is_dungeon_tileset(header.tileset) || header.tileset != previous_tileset) && warp != 0xFF {
            if let Some(to) = objects.warp_to.get(warp as usize) {
                self.view.view = MapView::view_from_address(to.view);
                location.y = to.y;
                location.x = to.x;
                self.view.y_block = to.y & 1;
                self.view.x_block = to.x & 1;
            }
        }
    }

    // ---- The loop ----

    /// The body of `OverworldLoopLessDelay`, after its `DelayFrame`.
    fn body(&mut self, ctx: &mut Ctx) -> Transition {
        self.load_gb_pal(ctx);
        if self.jumping {
            // `_HandleMidJump`.
            let index = self.jump_index;
            if index + 1 < JUMP_PASSES {
                self.jump_index = index + 1;
                self.sprites[0].y_pixels = rom_slice(pokered_symbols::PlayerJumpingYScreenCoords)[index as usize];
            } else if self.walk_counter == 0 {
                self.update_sprites(ctx);
                self.phase = Phase::Landing(LANDING_FRAMES);
                return Transition::Stay;
            }
        }
        self.after_mid_jump(ctx)
    }

    /// `.finishedJump` once its `Delay3` is over.
    fn land(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.pad.held = Joypad::empty();
        ctx.pad.pressed = Joypad::empty();
        ctx.pad.released = Joypad::empty();
        self.jump_index = 0;
        self.jumping = false;
        self.scripted = false;
        ctx.pad.ignore = Joypad::empty();
        self.after_mid_jump(ctx)
    }

    fn after_mid_jump(&mut self, ctx: &mut Ctx) -> Transition {
        self.polled = false;
        if self.walk_counter != 0 {
            // `.moveAhead`.
            if self.spinning {
                self.load_spinner_arrow_tiles(ctx);
            }
            self.update_sprites(ctx);
            return self.move_ahead2(ctx);
        }
        // `JoypadOverworld`, which runs the map's script before it reads the pad.
        self.sprites[0].y_step = 0;
        self.sprites[0].x_step = 0;
        self.rt.stack = vec![Routine::AfterRunMapScript.into(), Routine::MapScript.into()];
        self.run_script(ctx)
    }

    /// The rest of `JoypadOverworld` after `RunMapScript`, and the loop body after it.
    fn after_run_map_script(&mut self, ctx: &mut Ctx) -> Transition {
        self.joypad_overworld(ctx);
        // `SafariZoneCheck`.
        if ctx.world.events.is_set(EVENT_IN_SAFARI_ZONE) && ctx.world.safari_balls == 0 {
            self.rt.stack = vec![Routine::SafariWarp.into(), events::Label::SafariZoneGameOver.into()];
            return self.run_script(ctx);
        }
        // `BIT_WARP_FROM_CUR_SCRIPT`: the bit is read and cleared before anything else can warp.
        if let Some((destination_map, destination_warp)) = self.rt.script_warp.take() {
            let location = &ctx.world.location;
            let warp = Warp { x: location.x, y: location.y, destination_map, destination_warp };
            return self.warp_found(ctx, warp);
        }
        if let Some(destination) = ctx.world.location.fly_warp.take() {
            return self.handle_fly_warp(ctx, destination);
        }
        if self.rt.dungeon_warp.is_some() {
            return self.handle_dungeon_warp(ctx);
        }
        if self.rt.cur_opponent != 0 {
            return self.new_battle(ctx);
        }
        let joy = if self.scripted { ctx.pad.held } else { ctx.pad.pressed };
        if joy.contains(Joypad::START) {
            return self.display_dialogue(ctx, 0);
        }
        if joy.contains(Joypad::A) {
            if self.a_disabled {
                return self.no_direction_buttons_pressed(ctx);
            }
            if self.exiting_door || self.scripted {
                return self.check_for_opponent(ctx);
            }
            if ctx.pad.held.contains(Joypad::A) {
                if let Some(code) = self.check_for_hidden_event_or_bookshelf_or_card_key_door(ctx) {
                    self.rt.stack = vec![code, events::Label::HiddenEventOrBookshelf.into()];
                    return self.run_script(ctx);
                }
            }
            return self.sprite_or_sign_dialogue(ctx);
        }
        let held = ctx.pad.held;
        let direction = [Direction::Down, Direction::Up, Direction::Left, Direction::Right]
            .into_iter()
            .find(|direction| held.contains(direction.button()));
        let Some(direction) = direction else {
            return self.no_direction_buttons_pressed(ctx);
        };
        let (dx, dy) = direction.delta();
        if dy != 0 {
            self.sprites[0].y_step = dy as u8;
        } else {
            self.sprites[0].x_step = dx as u8;
        }
        self.standing.player_direction = direction as u8;
        if !self.scripted && self.standing.check_for_180_degree_turn != 0
            && self.standing.last_stop_direction != direction as u8
        {
            // The intermediate facing of a 180-degree turn is set and replaced before a VBlank can
            // show it.
            self.turning = true;
            self.standing.check_for_180_degree_turn = 0;
            self.standing.moving_direction = direction as u8;
            // `NewBattle`: a turn in grass can start one.
            if let Some(battle) = self.start_new_battle(ctx) {
                return battle;
            }
            if let Some(text) = self.repel_wore_off_text(Routine::OverworldLoop) {
                return self.run_script_from(ctx, text);
            }
            return self.overworld_loop();
        }
        self.standing.moving_direction = direction as u8;
        self.update_sprites(ctx);
        if ctx.world.location.walk_bike_surf == SURFING {
            return match self.collision_check_on_water(ctx) {
                OnWater::Blocked => self.overworld_loop(),
                OnWater::Swim => self.start_step(ctx),
                // `.stopSurfing`'s `PlayDefaultMusic` waits for a sound still playing.
                OnWater::GetOff => self.run_script_from(ctx, vec![Routine::StartStep.into(), Routine::PlayDefaultMusic.into()]),
            };
        }
        if self.collision_check_on_land(ctx) {
            if self.standing.standing_on_warp && self.extra_warp_check(ctx) {
                return self.check_warps_collision(ctx);
            }
            return self.overworld_loop();
        }
        self.start_step(ctx)
    }

    /// `.noCollision`.
    pub(super) fn start_step(&mut self, ctx: &mut Ctx) -> Transition {
        self.walk_counter = WALK_PASSES;
        self.move_ahead2(ctx)
    }

    /// `IsSpriteOrSignInFrontOfPlayer` and on into the text it finds.
    fn sprite_or_sign_dialogue(&mut self, ctx: &mut Ctx) -> Transition {
        let text_id = self.sprite_or_sign_in_front(ctx);
        if text_id == 0 {
            return self.overworld_loop();
        }
        self.display_dialogue(ctx, text_id)
    }

    /// `CheckForHiddenEventOrBookshelfOrCardKeyDoor`'s finding, before anything takes frames: a
    /// hidden event or a bookshelf goes back to the loop once dealt with, whatever it did, and a card
    /// key door on to the sprite or sign in front. The event found is left in the runtime.
    fn check_for_hidden_event_or_bookshelf_or_card_key_door(&mut self, ctx: &mut Ctx) -> Option<Code> {
        let location = &ctx.world.location;
        let facing = self.player().facing;
        self.rt.events.hidden = None;
        if let Some(event) = hidden_events::check_for_hidden_event(location.map, location.x, location.y, facing) {
            self.rt.events.hidden_event_index = self.rt.events.hidden_event_index.wrapping_add(event.skipped);
            self.rt.events.hidden = Some(events::Hidden::Event(event));
            return Some(Routine::OverworldLoop.into());
        }
        let tiles = self.tile_map(ctx);
        if facing == SpriteFacing::Up as u8 && let Some(text) = hidden_events::bookshelf_text(self.view.tileset, tiles[7 * 20 + 8]) {
            self.rt.events.hidden = Some(events::Hidden::Bookshelf(text));
            return Some(Routine::OverworldLoop.into());
        }
        let front = collision::in_front(&tiles, location.x, location.y, SpriteFacing::from_repr(facing).unwrap_or_default());
        if hidden_events::card_key_door(location.map, front.tile) {
            self.rt.events.hidden = Some(events::Hidden::CardKeyDoor);
            return Some(Routine::AfterCardKeyText.into());
        }
        None
    }

    /// `JoypadOverworld` from its `Joypad`.
    fn joypad_overworld(&mut self, ctx: &mut Ctx) {
        self.polled = false;
        ctx.pad.poll();
        if self.joypad_disabled {
            ctx.pad.held = Joypad::empty();
            ctx.pad.pressed = Joypad::empty();
            ctx.pad.released = Joypad::empty();
        }
        if !self.scripted || ctx.pad.held.intersects(self.rt.override_simulated) {
            return;
        }
        self.simulated_index = self.simulated_index.wrapping_sub(1);
        if self.simulated_index == 0xFF {
            self.simulated_index = 0;
            self.simulated.clear();
            ctx.pad.ignore = Joypad::empty();
            ctx.pad.held = Joypad::empty();
            self.standing_on_door = false;
            self.exiting_door = false;
            self.standing.standing_on_warp = false;
            self.scripted = false;
            return;
        }
        let state = self.simulated.get(self.simulated_index as usize).copied().unwrap_or_default();
        ctx.pad.held = state;
        if state.is_empty() {
            ctx.pad.pressed = Joypad::empty();
            ctx.pad.released = Joypad::empty();
        }
    }

    /// `PlayerStepOutFromDoor`.
    fn player_step_out_from_door(&mut self, ctx: &mut Ctx) {
        let standing = self.tile_map(ctx)[9 * 20 + 8];
        if collision::on_door_tile(self.view.tileset, standing) {
            // The step out starts in this pass, and on the cartridge a step's first pass always runs
            // into the next frame, which VBlank counts off the lock. Taking it here ends the lock
            // on the same pass as the cartridge's.
            self.ignore_input = self.ignore_input.saturating_sub(STEP_START_LAG);
            ctx.pad.ignore = Joypad::SELECT | Joypad::START | PAD_CTRL_PAD;
            self.exiting_door = true;
            self.simulated = vec![Joypad::DOWN];
            self.simulated_index = 1;
            self.sprites[0].image_index = 0;
            self.scripted = true;
            self.sprites[0].movement1 = 0;
        } else {
            self.simulated_index = 0;
            self.simulated.clear();
            self.standing_on_door = false;
            self.exiting_door = false;
            self.scripted = false;
        }
    }
}


impl Overworld {
    /// `jp OverworldLoop`: two frames to the next body.
    fn overworld_loop(&mut self) -> Transition {
        self.phase = Phase::Loop(1);
        Transition::Stay
    }

    /// `.noDirectionButtonsPressed`.
    fn no_direction_buttons_pressed(&mut self, ctx: &mut Ctx) -> Transition {
        self.polled = !self.scripted && !self.exiting_door && !self.joypad_disabled && ctx.pad.ignore.is_empty()
            && !self.rt.paths.scripted_npc_movement && self.rt.npc_movement_script_table == 0;
        self.turning = false;
        self.update_sprites(ctx);
        self.standing.check_for_180_degree_turn = 1;
        if self.standing.moving_direction != 0 {
            self.standing.last_stop_direction = self.standing.moving_direction;
            self.standing.moving_direction = 0;
        }
        self.overworld_loop()
    }

    /// `.moveAhead2` through the end of the step's checks.
    fn move_ahead2(&mut self, ctx: &mut Ctx) -> Transition {
        self.turning = false;
        self.polled = false;
        if ctx.world.location.walk_bike_surf == BIKING && !self.jumping {
            self.do_bike_speedup(ctx);
        }
        self.advance_player_sprite(ctx);
        if self.walk_counter != 0 {
            return self.check_map_connections(ctx);
        }
        if !self.scripted {
            self.rt.step_counter = self.rt.step_counter.wrapping_sub(1);
            if self.rt.wild_encounter_cooldown {
                self.rt.no_random_battle_steps = self.rt.no_random_battle_steps.wrapping_sub(1);
                if self.rt.no_random_battle_steps == 0 {
                    self.rt.wild_encounter_cooldown = false;
                }
            }
        }
        if ctx.world.events.is_set(EVENT_IN_SAFARI_ZONE) {
            // `SafariZoneCheckSteps`.
            if ctx.world.safari_steps == 0 {
                self.rt.stack = vec![Routine::SafariWarp.into(), events::Label::SafariZoneGameOver.into()];
                return self.run_script(ctx);
            }
            ctx.world.safari_steps -= 1;
        }
        if events::poison_step_takes_frames(self, ctx) {
            self.rt.stack = vec![Routine::AfterPoison.into(), events::Label::ApplyOutOfBattlePoisonDamage.into()];
            return self.run_script(ctx);
        }
        self.new_battle(ctx)
    }

    /// Back from `ApplyOutOfBattlePoisonDamage` when it took frames.
    fn after_poison(&mut self, ctx: &mut Ctx) -> Transition {
        if std::mem::take(&mut self.rt.out_of_battle_blackout) {
            return self.handle_black_out(ctx);
        }
        self.new_battle(ctx)
    }

    /// `SafariZoneGameOver`'s warp: `WarpFound2` to the gate's fourth warp.
    fn safari_warp(&mut self, ctx: &mut Ctx) -> Transition {
        const SAFARI_ZONE_GATE_WARP: u8 = 3;
        self.standing.moving_direction = 0;
        let location = &ctx.world.location;
        let warp = Warp { x: location.x, y: location.y, destination_map: Map::SafariZoneGate as u8, destination_warp: SAFARI_ZONE_GATE_WARP };
        self.warp_found(ctx, warp)
    }

    /// `AdvancePlayerSprite`: two pixels, the coordinates at the end, the view at the start.
    fn advance_player_sprite(&mut self, ctx: &mut Ctx) {
        let (dy, dx) = (self.player().y_step, self.player().x_step);
        self.walk_counter -= 1;
        let location = &mut ctx.world.location;
        if self.walk_counter == 0 {
            location.y = location.y.wrapping_add(dy);
            location.x = location.x.wrapping_add(dx);
        }
        if self.walk_counter == WALK_PASSES - 1 {
            self.view.step(dx as i8, dy as i8);
        }
        let (dy, dx) = (dy.wrapping_mul(2), dx.wrapping_mul(2));
        for sprite in self.sprites.iter_mut().skip(1).take(self.num_sprites as usize) {
            sprite.y_pixels = sprite.y_pixels.wrapping_sub(dy);
            sprite.x_pixels = sprite.x_pixels.wrapping_sub(dx);
        }
    }

    /// `DoBikeSpeedup`: a second advance in the pass, except under an NPC's movement script and on
    /// Cycling Road while anything but Down is held.
    fn do_bike_speedup(&mut self, ctx: &mut Ctx) {
        if self.rt.npc_movement_script_table != 0 {
            return;
        }
        if ctx.world.location.map == Map::Route17 && ctx.pad.held.intersects(Joypad::UP | Joypad::LEFT | Joypad::RIGHT) {
            return;
        }
        self.advance_player_sprite(ctx);
    }

    /// `CollisionCheckOnWater`, whose bump sounds as the land's does. Getting off loads Red walking.
    fn collision_check_on_water(&mut self, ctx: &mut Ctx) -> OnWater {
        if self.scripted {
            return OnWater::Swim;
        }
        let tiles = self.tile_map(ctx);
        let location = &ctx.world.location;
        let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
        let front = collision::in_front(&tiles, location.x, location.y, facing);
        let sprite = self.player().collision_data & self.standing.player_direction != 0;
        let found = collision_check_on_water(self.view.tileset, tiles[9 * 20 + 8], front.tile, sprite);
        match found {
            OnWater::Blocked => play_collision_sound(ctx),
            OnWater::GetOff => {
                ctx.world.location.walk_bike_surf = WALKING;
                sprites::load_player_sprite_graphics(&mut ctx.screen.tiles, &mut ctx.world.location, self.view.tileset);
            }
            OnWater::Swim => {}
        }
        found
    }

    /// `ItemUseSurfboard`'s `.makePlayerMoveForward`, when the start menu closes having got the player
    /// onto the water or off it: the simulated press that steps there. Getting off goes the way
    /// `IsSpriteInFrontOfPlayer2` set.
    pub(super) fn surf_step(&mut self, ctx: &mut Ctx) {
        let (now, then) = (ctx.world.location.walk_bike_surf, self.walk_bike_surf_copy);
        if now == then || now != SURFING && then != SURFING {
            return;
        }
        if now == WALKING {
            self.standing.player_direction = Direction::of_facing(ctx.world.location.facing) as u8;
        }
        let button = Direction::from_bits(self.standing.player_direction).unwrap_or(Direction::Right).button();
        self.simulated = vec![button];
        self.simulated_index = 1;
        self.scripted = true;
    }

    /// `CollisionCheckOnLand`, with `CheckForJumpingAndTilePairCollisions` and the ledge it starts.
    fn collision_check_on_land(&mut self, ctx: &mut Ctx) -> bool {
        if self.jumping || self.simulated_index != 0 {
            return false;
        }
        let collided = (|| {
            if self.player().collision_data & self.standing.player_direction != 0 {
                return true;
            }
            let text_id = sprites::sprite_in_front_of_player(&mut self.sprites, self.num_sprites, 0x10, &mut self.standing.player_direction);
            if text_id != 0 {
                return true;
            }
            let tiles = self.tile_map(ctx);
            let location = &ctx.world.location;
            let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
            let front = collision::in_front(&tiles, location.x, location.y, facing);
            let standing = tiles[9 * 20 + 8];
            if let Some(input) = collision::ledge_input(self.view.tileset, facing, standing, front.tile)
                && ctx.pad.held.bits() & input != 0
            {
                // The jump is armed, not taken: this pass still bumps into the ledge tile, and the
                // simulated presses carry the player over it from the next.
                self.start_ledge_jump(ctx, input);
            } else if collision::tile_pair_collision(self.view.tileset as u8, standing, front.tile, false) {
                return true;
            }
            !collision::tile_passable(self.view.tileset, front.tile)
        })();
        if collided {
            play_collision_sound(ctx);
        }
        collided
    }

    /// `HandleLedges` once it has found one.
    fn start_ledge_jump(&mut self, ctx: &mut Ctx, input: u8) {
        ctx.pad.ignore = Joypad::all();
        self.jumping = true;
        self.scripted = true;
        self.sprites[0].movement1 = 0;
        let held = Joypad::from_bits_truncate(input);
        self.simulated = vec![held, held];
        self.simulated_index = 2;
        // `LoadHoppingShadowOAM`.
        let start = pokered_symbols::LedgeHoppingShadow;
        let len = (pokered_symbols::LedgeHoppingShadowEnd.address - start.address) as usize;
        ctx.screen.tiles.load_1bpp(crate::gfx::tiles::V_CHARS1 + 0x7F, &rom_slice(start)[..len]);
        const OAM_PAL1: u8 = 0x10;
        let attributes = [OAM_PAL1, Object::X_FLIP, Object::Y_FLIP, Object::X_FLIP | Object::Y_FLIP];
        ctx.screen.sprites.resize(40, Object { y: 160, ..Object::default() });
        for (i, attributes) in attributes.into_iter().enumerate() {
            let (dy, dx) = ((i / 2) as u8 * 8, (i % 2) as u8 * 8);
            ctx.screen.sprites[36 + i] = Object { y: 0x54 + dy, x: 0x48 + dx, tile: 0xFF, attributes };
        }
        ctx.audio.play_sound(sounds::SFX_LEDGE);
    }

    fn extra_warp_check(&self, ctx: &Ctx) -> bool {
        let tiles = self.tile_map(ctx);
        let location = &ctx.world.location;
        let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
        collision::extra_warp_check(ExtraWarp {
            map: location.map as u8,
            tileset: self.view.tileset,
            facing,
            x: location.x,
            y: location.y,
            width: self.view.width,
            height: self.view.height,
            front: collision::in_front(&tiles, location.x, location.y, facing).tile,
        })
    }

    /// `CheckWarpsNoCollision`: a step has ended on a warp.
    fn check_warps_no_collision(&mut self, ctx: &mut Ctx) -> Transition {
        let (x, y) = (ctx.world.location.x, ctx.world.location.y);
        for index in 0..self.warps.len() {
            let warp = self.warps[index];
            if warp.y != y || warp.x != x {
                continue;
            }
            self.standing.standing_on_warp = true;
            let standing = self.tile_map(ctx)[9 * 20 + 8];
            let (warps, clears) = collision::on_door_or_warp_tile(self.view.tileset, standing);
            if clears {
                self.standing.standing_on_warp = false;
            }
            if warps {
                return self.warp_found(ctx, warp);
            }
            if self.extra_warp_check(ctx) {
                if self.rt.forced_warp {
                    return self.warp_found(ctx, warp);
                }
                ctx.pad.poll();
                if ctx.pad.held.intersects(PAD_CTRL_PAD) {
                    return self.warp_found(ctx, warp);
                }
            }
        }
        self.check_map_connections(ctx)
    }

    /// `CheckWarpsCollision`: walking into a wall from a warp that takes the player that way.
    fn check_warps_collision(&mut self, ctx: &mut Ctx) -> Transition {
        let location = &ctx.world.location;
        match self.warps.iter().copied().find(|warp| warp.y == location.y && warp.x == location.x) {
            Some(warp) => self.warp_found(ctx, warp),
            None => self.overworld_loop(),
        }
    }

    /// `WarpFound1` and `WarpFound2`, up to the fade that ends in `EnterMap`.
    fn warp_found(&mut self, ctx: &mut Ctx, warp: Warp) -> Transition {
        self.standing.destination_warp = warp.destination_warp;
        self.warp_destination_map = warp.destination_map;
        let location = &mut ctx.world.location;
        let destination = |id: u8| Map::from_repr(id).expect("a warp leads to a map");
        // `GBFadeOutToBlack`, from `PlayMapChangeSound` unless the map is dark, or first of all into
        // Rock Tunnel.
        let mut fade = self.map_pal_offset == 0;
        if collision::is_outside(self.view.tileset) {
            location.last_map = location.map;
            location.map = destination(warp.destination_map);
            if location.map == Map::RockTunnel1F {
                self.map_pal_offset = 6;
                fade = true;
            }
        } else if warp.destination_map == LAST_MAP {
            location.map = location.last_map;
            self.map_pal_offset = 0;
        } else {
            location.map = destination(warp.destination_map);
            // A warp pad spins the player out and in instead of the map-change sound and the fade.
            if self.standing_on_warp_pad_or_hole(ctx) == escape::WARP_PAD {
                self.rt.entered_by = Some(script::SpecialEnter::Spin);
                self.standing_on_door = false;
                self.exiting_door = false;
                return self.run_script_from(ctx, vec![Routine::WarpPadFaded.into(), Routine::LeaveMapAnim.into()]);
            }
            self.standing_on_door = false;
            self.exiting_door = false;
        }
        self.play_map_change_sound(ctx);
        if fade { self.fade_step(ctx, 4, AfterFade::Warp) } else { self.warp_faded(ctx) }
    }

    /// `WarpFound2`'s `.done`, after the fade: the door to step out of, and half a second of input
    /// ignored from here.
    pub(super) fn warp_faded(&mut self, ctx: &mut Ctx) -> Transition {
        self.standing_on_door = true;
        // `IgnoreInputForHalfSecond`.
        self.ignore_input = IGNORE_INPUT_FRAMES;
        self.joypad_disabled = true;
        self.a_disabled = true;
        self.enter_map(ctx)
    }

    /// `PlayMapChangeSound`, whose fade is the caller's.
    fn play_map_change_sound(&self, ctx: &mut Ctx) {
        const DOOR_TILE: u8 = 0x0B;
        let sound = if self.tile_map(ctx)[8 * 20 + 8] == DOOR_TILE { sounds::SFX_GO_INSIDE } else { sounds::SFX_GO_OUTSIDE };
        ctx.audio.play_sound(sound);
    }

    /// One palette of `GBFadeOutToBlack`, `FadePal4` down to `FadePal1`.
    fn fade_step(&mut self, ctx: &mut Ctx, palette: u8, then: AfterFade) -> Transition {
        let [bgp, obp0, obp1] = fade_palette(palette);
        ctx.screen.effects.bgp = bgp;
        ctx.screen.effects.obp0 = obp0;
        ctx.screen.effects.obp1 = obp1;
        self.phase = Phase::FadeOut { palettes: palette - 1, frames: FADE_FRAMES, then };
        Transition::Stay
    }

    /// `GBFadeOutToBlack`.
    fn fade_out_to_black(&mut self, ctx: &mut Ctx, then: AfterFade) -> Transition {
        self.fade_step(ctx, 4, then)
    }

    /// `CheckMapConnections`: a step that ended off the map's edge is a step into its neighbour.
    fn check_map_connections(&mut self, ctx: &mut Ctx) -> Transition {
        let location = &ctx.world.location;
        let header = Self::header(location.map);
        let (width2, height2) = (self.view.width.wrapping_mul(2), self.view.height.wrapping_mul(2));
        let (connection, horizontal) = if location.x == 0xFF {
            (header.west_connection, true)
        } else if location.x == width2 {
            (header.east_connection, true)
        } else if location.y == 0xFF {
            (header.north_connection, false)
        } else if location.y == height2 {
            (header.south_connection, false)
        } else {
            return self.overworld_loop();
        };
        let Some(connection) = connection else { return self.overworld_loop() };
        let location = &mut ctx.world.location;
        location.map = connection.map;
        let view = MapView::view_from_address(connection.view_pointer);
        self.view.view = if horizontal {
            location.x = connection.x_alignment as u8;
            location.y = location.y.wrapping_add(connection.y_alignment as u8);
            let rows = (location.y >> 1) as u16;
            view.wrapping_add(rows.wrapping_mul(connection.connected_map_width as u16 + 6))
        } else {
            location.y = connection.y_alignment as u8;
            location.x = location.x.wrapping_add(connection.x_alignment as u8);
            view.wrapping_add((location.x >> 1) as u16)
        };
        // `.loadNewMap`.
        self.load_map_header(ctx);
        play_default_music_fade_out_current(ctx, false);
        self.set_pal_overworld(ctx);
        let location = &ctx.world.location;
        sprites::init_map_sprites(&mut self.sprites, &mut self.sprite_set, location.map, location.x, location.y,
            self.num_sprites, self.font_loaded, &mut ctx.screen.tiles);
        self.view.blocks = tile_block_map(location.map).expect("the map has blocks");
        self.phase = Phase::Loop(0);
        Transition::Stay
    }

    /// `IsSpriteOrSignInFrontOfPlayer`: a sign's text id, or the slot of a sprite in front, or
    /// across a counter tile two squares away.
    fn sprite_or_sign_in_front(&mut self, ctx: &mut Ctx) -> u8 {
        let tiles = self.tile_map(ctx);
        let location = &ctx.world.location;
        let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
        let front = collision::in_front(&tiles, location.x, location.y, facing);
        if let Some(sign) = self.signs.iter().find(|sign| sign.y == front.y && sign.x == front.x) {
            return sign.text_id;
        }
        let counter = tileset_entry(self.view.tileset).talking_over.contains(&front.tile);
        let range = if counter { 0x20 } else { 0x10 };
        sprites::sprite_in_front_of_player(&mut self.sprites, self.num_sprites, range, &mut self.standing.player_direction)
    }

    /// `.displayDialogue` into `DisplayTextID`: the start menu for text id 0, else a sign's or a
    /// sprite's text.
    fn display_dialogue(&mut self, ctx: &mut Ctx, text_id: u8) -> Transition {
        self.polled = false;
        let tiles = self.tile_map(ctx);
        let location = &ctx.world.location;
        let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
        ctx.world.location.ahead.tile = collision::in_front(&tiles, location.x, location.y, facing).tile;
        self.update_sprites(ctx);
        if self.turning || self.rt.seen_by_trainer {
            return self.check_for_opponent(ctx);
        }
        ctx.world.location.ahead.standing_on = self.tile_map(ctx)[9 * 20 + 8];
        self.rt.stack = vec![Routine::AfterDisplayDialogue.into(), Routine::DisplayTextId(text_id).into()];
        self.run_script(ctx)
    }

    /// `.checkForOpponent`: a battle a text has set up, or back to the loop.
    fn check_for_opponent(&mut self, ctx: &mut Ctx) -> Transition {
        if self.rt.cur_opponent != 0 {
            return self.new_battle(ctx);
        }
        self.overworld_loop()
    }

    // ---- What a command is judged against ----

    /// Why a step `direction` would go nowhere, judged as `CollisionCheckOnLand` would judge it
    /// now: a sprite, a wall, a tile pair. A ledge or a warp taken off the edge is not blocked.
    pub fn step_refusal(&self, direction: Direction, world: &World) -> Option<String> {
        let location = &world.location;
        if location.walk_bike_surf == SURFING {
            let mut sprites = self.sprites;
            let (dx, dy) = direction.delta();
            sprites[0].y_step = dy as u8;
            sprites[0].x_step = dx as u8;
            sprites[0].facing = direction.facing() as u8;
            sprites::detect_collision_between_sprites(&mut sprites, 0);
            let tiles = self.view.tile_map();
            let front = collision::in_front(&tiles, location.x, location.y, direction.facing());
            let sprite = sprites[0].collision_data & direction as u8 != 0;
            let blocked = collision_check_on_water(self.view.tileset, tiles[9 * 20 + 8], front.tile, sprite) == OnWater::Blocked;
            return blocked.then(|| if sprite { "a sprite is in the way".to_string() } else { format!("tile ${:02X} is in the way", front.tile) });
        }
        let mut sprites = self.sprites;
        let (dx, dy) = direction.delta();
        sprites[0].y_step = dy as u8;
        sprites[0].x_step = dx as u8;
        sprites[0].facing = direction.facing() as u8;
        sprites::detect_collision_between_sprites(&mut sprites, 0);
        let mut player_direction = direction as u8;
        let blocked = if sprites[0].collision_data & direction as u8 != 0
            || sprites::sprite_in_front_of_player(&mut sprites, self.num_sprites, 0x10, &mut player_direction) != 0
        {
            Some("a sprite is in the way".to_string())
        } else {
            let tiles = self.view.tile_map();
            let facing = direction.facing();
            let front = collision::in_front(&tiles, location.x, location.y, facing);
            let standing = tiles[9 * 20 + 8];
            if collision::ledge_input(self.view.tileset, facing, standing, front.tile)
                .is_some_and(|input| input & direction.button().bits() != 0)
            {
                None
            } else if collision::tile_pair_collision(self.view.tileset as u8, standing, front.tile, false)
                || !collision::tile_passable(self.view.tileset, front.tile)
            {
                Some(format!("tile ${:02X} is in the way", front.tile))
            } else {
                None
            }
        };
        let off_the_edge = || {
            let tiles = self.view.tile_map();
            let facing = direction.facing();
            self.standing.standing_on_warp && collision::extra_warp_check(ExtraWarp {
                map: location.map as u8,
                tileset: self.view.tileset,
                facing,
                x: location.x,
                y: location.y,
                width: self.view.width,
                height: self.view.height,
                front: collision::in_front(&tiles, location.x, location.y, facing).tile,
            })
        };
        blocked.filter(|_| !off_the_edge())
    }

    /// The slot or sign text A would open, as `IsSpriteOrSignInFrontOfPlayer` finds it.
    pub fn in_front_text(&self, world: &World) -> u8 {
        let mut probe = self.clone();
        let tiles = probe.view.tile_map();
        let location = &world.location;
        let facing = SpriteFacing::from_repr(probe.player().facing).unwrap_or_default();
        let front = collision::in_front(&tiles, location.x, location.y, facing);
        if let Some(sign) = probe.signs.iter().find(|sign| sign.y == front.y && sign.x == front.x) {
            return sign.text_id;
        }
        let counter = tileset_entry(probe.view.tileset).talking_over.contains(&front.tile);
        let range = if counter { 0x20 } else { 0x10 };
        let num_sprites = probe.num_sprites;
        sprites::sprite_in_front_of_player(&mut probe.sprites, num_sprites, range, &mut probe.standing.player_direction)
    }
}

impl ModeUpdate for Overworld {
    /// `SpecialEnterMap`'s load, or for a fixture the same with its sprites kept. The view is the
    /// one a warp onto the player's square would give.
    fn enter(&mut self, ctx: &mut Ctx) {
        let location = &ctx.world.location;
        let width = Self::header(location.map).width as u16;
        let view = 7 + width + (width + 6) * (location.y >> 1) as u16 + (location.x >> 1) as u16;
        self.view.view = view;
        self.view.x_block = location.x & 1;
        self.view.y_block = location.y & 1;
        if !self.keep_sprites {
            self.sprites[0].facing = location.facing as u8;
        }
        let destination = std::mem::replace(&mut self.standing.destination_warp, 0xFF);
        self.view.tileset = Self::header(location.map).tileset;
        let resuming = self.keep_sprites;
        if resuming {
            // A running game is long past the fade into its map's song, so the song is already on.
            play_default_music_common(ctx, 0);
        }
        self.load_map_data(ctx);
        self.standing.destination_warp = destination;
        if resuming {
            // Stopped where the cartridge's loop had just polled, so its sprites are already updated.
            self.polled = true;
        } else {
            self.update_sprites(ctx);
        }
        self.load_gb_pal(ctx);
        self.present(ctx);
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        // `CountDownIgnoreInputBitReset`, from VBlank.
        if self.ignore_input != 0 {
            self.ignore_input -= 1;
            if self.ignore_input == 0 && self.joypad_disabled {
                self.joypad_disabled = false;
                self.a_disabled = false;
                ctx.pad.pressed = Joypad::empty();
                ctx.pad.held = Joypad::empty();
            }
        }
        let transition = match self.phase.clone() {
            Phase::Loop(0) => self.body(ctx),
            Phase::Loop(frames) => {
                self.phase = Phase::Loop(frames - 1);
                Transition::Stay
            }
            Phase::Landing(1) => self.land(ctx),
            Phase::Landing(frames) => {
                self.phase = Phase::Landing(frames - 1);
                Transition::Stay
            }
            Phase::FadeOut { palettes: 0, frames: 1, then: AfterFade::Warp } => self.warp_faded(ctx),
            Phase::FadeOut { palettes: 0, frames: 1, then: AfterFade::BlackOut } => self.black_out_music(ctx),
            Phase::FadeOut { palettes, frames: 1, then } => self.fade_step(ctx, palettes, then),
            Phase::FadeOut { palettes, frames, then } => {
                self.phase = Phase::FadeOut { palettes, frames: frames - 1, then };
                Transition::Stay
            }
            Phase::FadeIn { palettes: 0, frames: 1 } => self.enter_map_rest(ctx),
            Phase::FadeIn { palettes, frames: 1 } => self.fade_in_step(ctx, palettes + 4),
            Phase::FadeIn { palettes, frames } => {
                self.phase = Phase::FadeIn { palettes, frames: frames - 1 };
                Transition::Stay
            }
            Phase::Script => self.script_frame(ctx),
        };
        if matches!(transition, Transition::Stay) {
            self.present(ctx);
        }
        transition
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        if self.phase != Phase::Script {
            return Transition::Stay;
        }
        let transition = self.script_resume(outcome, ctx);
        if matches!(transition, Transition::Stay) {
            self.present(ctx);
        }
        transition
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::Loop(_) if self.polled && !self.moving() && !self.joypad_disabled => Status::Waiting(Decision::Overworld),
            Phase::Script if self.rt.waiting == Waiting::TextScrollButton => Status::Waiting(Decision::Text),
            _ => Status::Busy,
        }
    }
}

/// `FadePal1` to `FadePal8`, as `rBGP`, `rOBP0` and `rOBP1`.
fn fade_palette(n: u8) -> [u8; 3] {
    let table = rom_slice(pokered_symbols::FadePal1 + (n as u16 - 1) * 3);
    [table[0], table[1], table[2]]
}

/// `CollisionCheckOnLand`'s bump, which does not restart while one is already playing.
fn play_collision_sound(ctx: &mut Ctx) {
    const CHAN5: usize = 4;
    if ctx.audio.channel_sound_id(CHAN5) != sounds::SFX_COLLISION.0 {
        ctx.audio.play_sound(sounds::SFX_COLLISION);
    }
}

/// `PlayDefaultMusicFadeOutCurrent` on walking into a map: the map's song, the current one faded.
/// After a battle the fade is faster and the song starts over even if it is already playing, and the
/// bike's or the water's leaves the bank to the fade.
pub(super) fn play_default_music_fade_out_current(ctx: &mut Ctx, after_battle: bool) {
    if !after_battle {
        return play_default_music_common(ctx, 10);
    }
    ctx.audio.set_last_music_sound_id(SoundId(0));
    match ctx.world.location.walk_bike_surf {
        WALKING => play_default_music_common(ctx, 8),
        SURFING => ctx.audio.play_map_music(sounds::MUSIC_SURFING, 8),
        _ => ctx.audio.play_map_music(sounds::MUSIC_BIKE_RIDING, 8),
    }
}

/// `PlayDefaultMusic` after its `WaitForSoundToFinish`: the song starts over even if it is already
/// playing.
pub(crate) fn play_default_music(ctx: &mut Ctx) {
    ctx.audio.set_last_music_sound_id(SoundId(0));
    play_default_music_common(ctx, 0);
}

/// `PlayDefaultMusicCommon`: the bike's song or the water's, else the map's, the current one faded
/// `fade` frames a step. The bike's and the water's change the bank at once even under a fade: the
/// routine tests the flag only the end of a battle sets.
pub(crate) fn play_default_music_common(ctx: &mut Ctx, fade: u8) {
    let sound = match ctx.world.location.walk_bike_surf {
        WALKING => {
            let (id, bank) = map_song(ctx.world.location.map);
            let bank = AudioBank::from_rom_bank(bank).expect("a map's song is in one of the audio banks");
            return ctx.audio.play_map_music(Sound { bank, id: SoundId(id) }, fade);
        }
        SURFING => sounds::MUSIC_SURFING,
        _ => sounds::MUSIC_BIKE_RIDING,
    };
    ctx.audio.set_bank(sound.bank);
    ctx.audio.play_map_music(sound, fade);
}

/// What the overworld's commands ask for, which the executor carries out as presses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverworldDriver {
    order: Order,
    /// Where the player stood and faced when the command was accepted.
    from: (Map, u8, u8, SpriteFacing),
    /// The press has been taken: a step, a turn, a warp or a mode on top has begun.
    started: bool,
    /// Bodies polled with the press held and nothing started, for a bump nothing foresaw.
    bumps: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Order {
    Step(Direction),
    Face(Direction),
    Interact,
    OpenStartMenu,
}

impl OverworldDriver {
    pub fn step(direction: Direction, overworld: &Overworld, world: &World) -> Result<Self, Refusal> {
        Self::waiting(overworld)?;
        if let Some(reason) = overworld.step_refusal(direction, world) {
            return Err(Refusal::Invalid(reason));
        }
        Ok(Self::new(Order::Step(direction), world))
    }

    pub fn face(direction: Direction, overworld: &Overworld, world: &World) -> Result<Self, Refusal> {
        Self::waiting(overworld)?;
        if world.location.facing == direction.facing() {
            return Err(Refusal::Invalid(format!("the player already faces {:?}", direction)));
        }
        if overworld.standing.last_stop_direction == direction as u8 {
            return Err(Refusal::Invalid(format!("a press {:?} would walk rather than turn", direction)));
        }
        Ok(Self::new(Order::Face(direction), world))
    }

    pub fn interact(overworld: &Overworld, world: &World) -> Result<Self, Refusal> {
        Self::waiting(overworld)?;
        if overworld.in_front_text(world) == 0 {
            return Err(Refusal::Invalid("nothing to talk to or read in front of the player".into()));
        }
        Ok(Self::new(Order::Interact, world))
    }

    pub fn open_start_menu(overworld: &Overworld, world: &World) -> Result<Self, Refusal> {
        Self::waiting(overworld)?;
        Ok(Self::new(Order::OpenStartMenu, world))
    }

    fn waiting(overworld: &Overworld) -> Result<(), Refusal> {
        if overworld.status() != Status::Waiting(Decision::Overworld) {
            return Err(Refusal::Invalid("the player is not free to move".into()));
        }
        Ok(())
    }

    fn new(order: Order, world: &World) -> Self {
        let location = &world.location;
        Self { order, from: (location.map, location.x, location.y, location.facing), started: false, bumps: 0 }
    }

    /// Holds the press until the overworld takes it, then lets go until it is waiting again.
    pub fn drive(&mut self, modes: &[Mode], world: &World) -> Drive {
        let Some(Mode::Overworld(overworld)) = modes.last() else {
            return match self.order {
                Order::Interact | Order::OpenStartMenu => Drive::Done,
                _ => Drive::Interrupted("something took over the screen".into()),
            };
        };
        let location = &world.location;
        let now = (location.map, location.x, location.y, location.facing);
        if !self.started {
            self.started = match self.order {
                Order::Step(_) => overworld.moving() || now != self.from,
                Order::Face(_) => overworld.is_turning() || now != self.from,
                Order::Interact | Order::OpenStartMenu => !matches!(overworld.phase, Phase::Loop(_)),
            };
        }
        if self.started {
            let settled = overworld.status() == Status::Waiting(Decision::Overworld) && !overworld.is_turning();
            let arrived = match self.order {
                Order::Face(direction) => location.facing == direction.facing(),
                _ => true,
            };
            return if settled && arrived { Drive::Done } else { Drive::Press(Joypad::empty()) };
        }
        if overworld.status() == Status::Waiting(Decision::Overworld) {
            self.bumps += 1;
            if self.bumps > 4 {
                return Drive::Interrupted("the press was taken by nothing".into());
            }
        }
        Drive::Press(match self.order {
            Order::Step(direction) | Order::Face(direction) => direction.button(),
            Order::Interact => Joypad::A,
            Order::OpenStartMenu => Joypad::START,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, Reply};
    use crate::rng::GameRng;
    use crate::{Game, Input, Pacing};

    fn game_at(map: Map, x: u8, y: u8) -> Game {
        let mut world = World::default();
        world.location = Location { map, x, y, facing: SpriteFacing::Up, last_map: Map::PalletTown, ..Location::default() };
        // Past Oak's stopping the player at the grass, and with no party to fight a wild battle.
        world.events.set(poke_core::symbols::pokered_events::EVENT_FOLLOWED_OAK_INTO_LAB);
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        game.push(Mode::Overworld(overworld));
        settle(&mut game);
        game
    }

    fn settle(game: &mut Game) -> u32 {
        for frames in 1..2000 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::Overworld) {
                return frames;
            }
        }
        panic!("the overworld never settled: {:?}", game.modes().last().map(|m| m.status()));
    }

    fn command(game: &mut Game, command: Command) -> u32 {
        let frame = game.frame(Input::Command(command.clone()));
        assert_eq!(frame.reply, Some(Reply::Accepted), "{command:?}");
        settle(game)
    }

    fn at(game: &Game) -> (Map, u8, u8) {
        let location = &game.world().location;
        (location.map, location.x, location.y)
    }

    #[test]
    fn a_step_is_sixteen_frames_and_a_turn_costs_a_pass_more() {
        let mut game = game_at(Map::PalletTown, 5, 9);
        command(&mut game, Command::Step(Direction::Up));
        let straight = command(&mut game, Command::Step(Direction::Up));
        assert_eq!(at(&game), (Map::PalletTown, 5, 7));
        let turned = command(&mut game, Command::Step(Direction::Right));
        assert_eq!(at(&game), (Map::PalletTown, 6, 7));
        assert_eq!(turned, straight + 2, "a pass to turn before walking");
        command(&mut game, Command::Face(Direction::Left));
        assert_eq!(at(&game), (Map::PalletTown, 6, 7));
        assert_eq!(game.world().location.facing, SpriteFacing::Left);
    }

    #[test]
    fn a_wall_refuses_a_step_and_says_what_is_in_the_way() {
        let mut game = game_at(Map::PalletTown, 4, 6);
        let frame = game.frame(Input::Command(Command::Step(Direction::Up)));
        assert!(matches!(frame.reply, Some(Reply::Refused(Refusal::Invalid(_)))), "{:?}", frame.reply);
    }

    #[test]
    fn through_red_s_door_and_back_out_onto_the_step_below_it() {
        let mut game = game_at(Map::PalletTown, 5, 6);
        command(&mut game, Command::Step(Direction::Up));
        assert_eq!(at(&game), (Map::RedsHouse1F, 2, 7));
        assert_eq!(game.world().location.last_map, Map::PalletTown);
        command(&mut game, Command::Step(Direction::Down));
        assert_eq!(at(&game), (Map::PalletTown, 5, 6), "out of the door and a step down");
        assert_eq!(game.world().location.facing, SpriteFacing::Down);
    }

    #[test]
    fn walking_off_the_north_edge_is_walking_into_route_1() {
        let mut game = game_at(Map::PalletTown, 10, 1);
        command(&mut game, Command::Step(Direction::Up));
        assert_eq!(at(&game), (Map::PalletTown, 10, 0));
        command(&mut game, Command::Step(Direction::Up));
        assert_eq!(at(&game).0, Map::Route1);
        let header = MapHeader::read(Map::Route1).unwrap();
        assert_eq!(game.world().location.y, header.height * 2 - 1);
    }

    #[test]
    fn a_sign_prints_its_text_and_a_press_puts_the_map_back() {
        // PALLET TOWN's sign is at (7, 9); stand below it facing up.
        let mut game = game_at(Map::PalletTown, 7, 10);
        let frame = game.frame(Input::Command(Command::Interact));
        assert_eq!(frame.reply, Some(Reply::Accepted));
        for _ in 0..600 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::Text) && matches!(game.modes().last(), Some(Mode::Overworld(_))) {
                break;
            }
            if game.status() == Status::Waiting(Decision::Text) {
                game.frame(Input::Command(Command::Advance));
            }
        }
        assert!(matches!(game.modes().last(), Some(Mode::Overworld(_))));
        assert!(game.ui().cover(1, 14).is_some(), "the text is still up");
        game.frame(Input::Command(Command::Advance));
        settle(&mut game);
        assert!(game.ui().cover(1, 14).is_none(), "the map is back");
    }

    #[test]
    fn start_opens_the_menu_and_closing_it_goes_back_to_walking() {
        let mut game = game_at(Map::PalletTown, 5, 8);
        let frame = game.frame(Input::Command(Command::OpenStartMenu));
        assert_eq!(frame.reply, Some(Reply::Accepted));
        for _ in 0..100 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::StartMenu) {
                break;
            }
        }
        assert_eq!(game.status(), Status::Waiting(Decision::StartMenu));
        game.frame(Input::Command(Command::CloseStartMenu));
        settle(&mut game);
        assert!(game.ui().cover(12, 2).is_none());
    }

    #[test]
    fn a_step_on_the_bike_is_eight_frames_and_the_bike_s_song_plays() {
        let mut walking = game_at(Map::PalletTown, 5, 9);
        command(&mut walking, Command::Step(Direction::Up));
        let walked = command(&mut walking, Command::Step(Direction::Up));
        let mut world = walking.world().clone();
        world.location = Location { map: Map::PalletTown, x: 5, y: 9, facing: SpriteFacing::Up, walk_bike_surf: BIKING, ..world.location };
        let mut riding = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        riding.push(Mode::Overworld(overworld));
        settle(&mut riding);
        assert!((0..600).any(|_| { riding.frame(Input::None); riding.audio().channel_sound_id(0) == sounds::MUSIC_BIKE_RIDING.id.0 }), "the song");
        command(&mut riding, Command::Step(Direction::Up));
        let ridden = command(&mut riding, Command::Step(Direction::Up));
        assert_eq!(at(&riding), (Map::PalletTown, 5, 7));
        assert_eq!(walked - ridden, 8, "four passes rather than eight");
    }

    #[test]
    fn the_water_is_swum_and_a_step_onto_the_shore_gets_off() {
        let mut game = game_at(Map::PalletTown, 4, 13);
        let refused = game.frame(Input::Command(Command::Step(Direction::Down))).reply;
        assert!(matches!(refused, Some(Reply::Refused(_))), "on foot the water is a wall: {refused:?}");
        let mut world = game.world().clone();
        world.location = Location { y: 14, facing: SpriteFacing::Down, walk_bike_surf: SURFING, ..world.location };
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        game.push(Mode::Overworld(overworld));
        settle(&mut game);
        assert!((0..600).any(|_| { game.frame(Input::None); game.audio().channel_sound_id(0) == sounds::MUSIC_SURFING.id.0 }), "the song");
        command(&mut game, Command::Step(Direction::Down));
        assert_eq!(at(&game), (Map::PalletTown, 4, 15));
        command(&mut game, Command::Step(Direction::Up));
        command(&mut game, Command::Step(Direction::Up));
        assert_eq!(at(&game), (Map::PalletTown, 4, 13));
        assert_eq!(game.world().location.walk_bike_surf, WALKING);
    }

    #[test]
    fn commands_get_on_the_bike_from_the_bag_and_ride_it() {
        use crate::modes::start_menu::StartMenuEntry;
        let game = game_at(Map::PalletTown, 5, 9);
        let mut world = game.world().clone();
        world.bag = crate::systems::inventory::Inventory::bag(vec![poke_core::bag::BagItem::new(poke_core::item::ItemId::Bicycle, 1)]);
        world.player_name = poke_core::charmap::encode("RED").unwrap();
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        game.push(Mode::Overworld(overworld));
        settle(&mut game);
        let walked = command(&mut game, Command::Step(Direction::Up));
        let until = |game: &mut Game, decision: Decision| {
            for _ in 0..600 {
                if game.status() == Status::Waiting(decision.clone()) {
                    return;
                }
                game.frame(Input::None);
            }
            panic!("never waited for {decision:?}");
        };
        game.frame(Input::Command(Command::OpenStartMenu));
        until(&mut game, Decision::StartMenu);
        game.frame(Input::Command(Command::ChooseStartMenuEntry(StartMenuEntry::Item)));
        until(&mut game, Decision::List);
        game.frame(Input::Command(Command::ChooseListEntry(0)));
        until(&mut game, Decision::Text);
        game.frame(Input::Command(Command::Advance));
        settle(&mut game);
        assert_eq!(game.modes().len(), 1, "the start menu closed");
        assert_eq!(game.world().location.walk_bike_surf, BIKING);
        let ridden = command(&mut game, Command::Step(Direction::Up));
        assert_eq!(at(&game), (Map::PalletTown, 5, 7));
        assert!(ridden < walked, "{ridden} frames on the bike against {walked} on foot");
    }

    #[test]
    fn a_save_mid_step_resumes_mid_step() {
        let mut game = game_at(Map::PalletTown, 5, 8);
        game.frame(Input::Buttons(Joypad::UP));
        for _ in 0..6 {
            game.frame(Input::Buttons(Joypad::UP));
        }
        let mut loaded = Game::load(&game.save(), Pacing::Faithful).unwrap();
        for _ in 0..40 {
            game.frame(Input::None);
            loaded.frame(Input::None);
        }
        assert_eq!(loaded.save(), game.save());
    }
}
