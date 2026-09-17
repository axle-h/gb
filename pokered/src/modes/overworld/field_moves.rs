//! What the party menu leaves to the overworld: the rest of `UsedCut`, FLASH's palette, and
//! Strength's `TryPushingBoulder` with the dust that follows it.
//!
//! The three animations are one shape: four objects written as a block beside the player, moved a
//! pixel or two a frame with `rOBP1` flickering under them, with OAM held still around them
//! (`wUpdateSpritesEnabled` at `$ff`). Their frame counts are exact. Faithful rather than exact:
//! cut grass drifts its four leaves at the two speeds `AnimCutGrass_UpdateOAMEntries` gives them but
//! neither swaps the pairs over nor creeps them down the screen.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::{pokered_symbols, DmgPointer};
use crate::audio::data::sounds;
use crate::gfx::layers::Object;
use crate::gfx::tiles::V_CHARS1;
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::systems::overworld::boulder::{boulder_blocked, BOULDER_MOVEMENT_BYTE_2};
use crate::systems::overworld::cut::{replace_tree_tile_block, CUT_GRASS};
use crate::systems::overworld::sprites::{load_player_sprite_graphics, sprite_in_front_of_player, FACE_PLAYER};
use crate::systems::overworld::location::UsedFieldMove;
use super::script::{text_at, Block, Flow, Routine, Then};
use super::Overworld;

/// The first of the four objects `WriteOAMBlock` writes for an animation, and the tile ids they
/// draw, which are `vChars1` tiles `$7c` to `$7f`.
const BLOCK: usize = 36;
const FIRST_TILE: u8 = 0xFC;
const FIRST_PATTERN: usize = V_CHARS1 + 0x7C;
/// `AnimCut`'s `cutTreeLoop`, and `AnimCutGrass`'s two rounds of eight twice over.
const CUT_TREE_FRAMES: u8 = 8;
const CUT_GRASS_FRAMES: u8 = 32;
/// `AnimateBoulderDust`'s eight steps, each a `Delay3`.
const DUST_STEPS: u8 = 8;
const DUST_STEP_FRAMES: u8 = 3;
/// The `rOBP1` each animation flickers.
const OBP1_NORMAL: u8 = 0b11100100;
const OBP1_FLICKER: u8 = 0b01100100;

impl Overworld {
    /// `AfterStartMenu`: the field move the party menu answered with, if it left one. `None` carries
    /// straight on to `CloseTextDisplay`, which is where both `.cut` and `.goBackToMap` end.
    pub(super) fn used_field_move(&mut self, ctx: &mut Ctx) -> Option<Flow> {
        match ctx.world.location.used_field_move.take()? {
            // `.flash`. Darkness is `rBGP` alone, from the next pass's `LoadGBPal`: `SetPal_Overworld`
            // reads the map and not this, so on an SGB a dark map was never dark.
            UsedFieldMove::Flash => {
                self.map_pal_offset = 0;
                None
            }
            UsedFieldMove::Cut(tile) => Some(self.used_cut(ctx, tile)),
        }
    }

    /// `UsedCut` from `.canCut`'s text on. Everything before it, the whiteout and the map view put
    /// back under the menu, is loading.
    fn used_cut(&mut self, ctx: &mut Ctx, tile: u8) -> Flow {
        ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
        self.cut_tile = tile;
        // `BIT_NO_TEXT_DELAY` around the text, which prints whole.
        ctx.world.no_text_delay = true;
        Then::block(Block::PrintText(text_at(pokered_symbols::UsedCutText))).then(Routine::UsedCutAnimation)
    }

    /// `UsedCut` after its text: the tree taken out of the map and the animation over where it was.
    pub(super) fn used_cut_animation(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.world.no_text_delay = false;
        ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
        // `wUpdateSpritesEnabled` at `$ff`, so the block stays where the animation puts it.
        self.rt.sprites_frozen = true;
        self.init_cut_anim_oam(ctx);
        // `ReplaceTreeTileBlock` and `RedrawMapView`: the screen draws from the blocks, so the tree
        // goes as the block does. Grass has no block without it, and the table leaves it alone.
        let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
        replace_tree_tile_block(&mut self.view, facing);
        let frames = if self.cut_tile == CUT_GRASS { CUT_GRASS_FRAMES } else { CUT_TREE_FRAMES };
        Then::call(Routine::AnimCut(frames)).then(Routine::CloseTextDisplay)
    }

    /// `InitCutAnimOAM`: the tree's own tiles for a tree, four copies of one leaf for grass, then
    /// the block of four beside the player.
    fn init_cut_anim_oam(&mut self, ctx: &mut Ctx) {
        ctx.screen.effects.obp1 = OBP1_NORMAL;
        if self.cut_tile == CUT_GRASS {
            let leaf = rom_slice(pokered_symbols::MoveAnimationTiles1 + 6 * TILE_BYTES as u16);
            for i in 0..4 {
                ctx.screen.tiles.load(FIRST_PATTERN + i, &leaf[..TILE_BYTES]);
            }
        } else {
            let tree = |tile: u16| rom_slice(pokered_symbols::Overworld_GFX + tile * TILE_BYTES as u16);
            ctx.screen.tiles.load(FIRST_PATTERN, &tree(0x2D)[..2 * TILE_BYTES]);
            ctx.screen.tiles.load(FIRST_PATTERN + 2, &tree(0x3D)[..2 * TILE_BYTES]);
        }
        self.write_animation_oam_block(ctx, pokered_symbols::CutAnimationOffsets);
        if self.cut_tile == CUT_GRASS {
            // The four leaves are one tile flipped two ways.
            for (i, flip) in [Object::X_FLIP, Object::Y_FLIP, Object::X_FLIP, Object::Y_FLIP].into_iter().enumerate() {
                ctx.screen.sprites[BLOCK + i].attributes = flip | Object::OBP1;
            }
        }
    }

    /// `WriteCutOrBoulderDustAnimationOAMBlock` over `GetCutOrBoulderDustAnimationOffsets`: a two by
    /// two block of objects, offset from the player by the table's entry for the way they face.
    fn write_animation_oam_block(&mut self, ctx: &mut Ctx, offsets: DmgPointer) {
        let entry = rom_slice(offsets + (self.player().facing >> 1) as u16);
        let (y, x) = (self.player().y_pixels.wrapping_add(entry[1]), self.player().x_pixels.wrapping_add(entry[0]));
        ctx.screen.sprites.resize(40, Object { y: 160, ..Object::default() });
        for (i, (dy, dx)) in [(0, 0), (0, 8), (8, 0), (8, 8)].into_iter().enumerate() {
            ctx.screen.sprites[BLOCK + i] = Object {
                y: y.wrapping_add(dy),
                x: x.wrapping_add(dx),
                tile: FIRST_TILE + i as u8,
                attributes: Object::OBP1,
            };
        }
    }

    /// One frame of `AnimCut`: the left pair drifts right and the right pair left, and `rOBP1`
    /// flickers. The grass animation's four leaves drift at two speeds.
    pub(super) fn anim_cut_frame(&mut self, ctx: &mut Ctx, left: u8) -> Flow {
        if left == 0 {
            ctx.screen.effects.obp1 = OBP1_NORMAL;
            self.rt.sprites_frozen = false;
            ctx.audio.play_sound(sounds::SFX_CUT);
            self.update_sprites(ctx);
            return Flow::Return;
        }
        let steps: [i8; 4] = if self.cut_tile == CUT_GRASS { [1, 2, -2, -1] } else { [1, 1, -1, -1] };
        for (i, step) in steps.into_iter().enumerate() {
            let object = &mut ctx.screen.sprites[BLOCK + i];
            object.x = object.x.wrapping_add(step as u8);
        }
        ctx.screen.effects.obp1 ^= OBP1_FLICKER;
        Then::block(Block::Frames(1)).then(Routine::AnimCut(left - 1))
    }
}

impl Overworld {
    /// `TryPushingBoulder`, which `RunMapScript` runs before anything else. The player must push
    /// twice: the first pass only arms `BIT_TRIED_PUSH_BOULDER`.
    pub(super) fn try_pushing_boulder(&mut self, ctx: &mut Ctx) {
        if !ctx.world.location.strength_active || self.rt.boulder_dust {
            return;
        }
        let mut direction = 0;
        let slot = sprite_in_front_of_player(&mut self.sprites, self.num_sprites, 0x10, &mut direction);
        self.rt.boulder_sprite = slot;
        if slot == 0 {
            return self.reset_boulder_push_flags();
        }
        // `IsSpriteInFrontOfPlayer` armed the sprite to turn to the player, which a boulder does not.
        self.sprites[slot as usize].movement_status &= !FACE_PLAYER;
        if self.sprites[slot as usize].movement2 != BOULDER_MOVEMENT_BYTE_2 {
            return self.reset_boulder_push_flags();
        }
        if !std::mem::replace(&mut self.rt.tried_push_boulder, true) {
            return;
        }
        if !ctx.pad.held.intersects(super::PAD_CTRL_PAD) {
            return;
        }
        let tiles = self.tile_map(ctx);
        let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
        if boulder_blocked(self.view.tileset, &tiles, facing, &self.sprites, self.num_sprites, slot) {
            return self.reset_boulder_push_flags();
        }
        // The pushed direction must still be the one held: a boulder is not shoved sideways.
        let (button, movement) = match facing {
            SpriteFacing::Down => (Joypad::DOWN, NPC_MOVEMENT_DOWN),
            SpriteFacing::Up => (Joypad::UP, NPC_MOVEMENT_UP),
            SpriteFacing::Left => (Joypad::LEFT, NPC_MOVEMENT_LEFT),
            SpriteFacing::Right => (Joypad::RIGHT, NPC_MOVEMENT_RIGHT),
        };
        if !ctx.pad.held.contains(button) {
            return;
        }
        self.set_sprite_movement_bytes_to_ff(slot);
        self.move_sprite(ctx, slot, &[movement, STAY]);
        ctx.audio.play_sound(sounds::SFX_PUSH_BOULDER);
        self.rt.boulder_dust = true;
    }

    /// `ResetBoulderPushFlags`.
    fn reset_boulder_push_flags(&mut self) {
        self.rt.boulder_dust = false;
        self.rt.tried_push_boulder = false;
    }

    /// `DoBoulderDustAnimation`, once the boulder has finished its square: the smoke, then the flags
    /// and the boulder's movement byte put back.
    pub(super) fn do_boulder_dust_animation(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.screen.effects.obp1 = OBP1_NORMAL;
        self.rt.sprites_frozen = true;
        let smoke = rom_slice(pokered_symbols::SSAnneSmokePuffTile);
        for i in 0..4 {
            ctx.screen.tiles.load(FIRST_PATTERN + i, &smoke[..TILE_BYTES]);
        }
        self.write_animation_oam_block(ctx, pokered_symbols::BoulderDustAnimationOffsets);
        Flow::Jump(Routine::AnimateBoulderDust(DUST_STEPS).into())
    }

    /// One step of `AnimateBoulderDust`: the whole block drifts the way the boulder came from.
    pub(super) fn animate_boulder_dust(&mut self, ctx: &mut Ctx, left: u8) -> Flow {
        if left == 0 {
            ctx.screen.effects.obp1 = OBP1_NORMAL;
            self.rt.sprites_frozen = false;
            // `LoadPlayerSpriteGraphics` and no `UpdateSprites`: one more tick of it would leave
            // every NPC's wander delay a pass out from the cartridge's.
            load_player_sprite_graphics(&mut ctx.screen.tiles, &mut ctx.world.location, self.view.tileset);
            // `DiscardButtonPresses`, then the flags and the boulder put back where a script can see
            // that it moved.
            ctx.pad.ignore = Joypad::empty();
            self.reset_boulder_push_flags();
            self.rt.pushed_boulder = true;
            let boulder = self.rt.boulder_sprite as usize;
            self.sprites[boulder].movement2 = BOULDER_MOVEMENT_BYTE_2;
            ctx.audio.play_sound(sounds::SFX_CUT);
            return Flow::Return;
        }
        let facing = SpriteFacing::from_repr(self.player().facing).unwrap_or_default();
        let (step, horizontal) = match facing {
            SpriteFacing::Down => (-1i8, false),
            SpriteFacing::Up => (1, false),
            SpriteFacing::Left => (1, true),
            SpriteFacing::Right => (-1, true),
        };
        for i in 0..4 {
            let object = &mut ctx.screen.sprites[BLOCK + i];
            if horizontal {
                object.x = object.x.wrapping_add(step as u8);
            } else {
                object.y = object.y.wrapping_add(step as u8);
            }
        }
        ctx.screen.effects.obp1 ^= OBP1_FLICKER;
        Then::block(Block::Frames(DUST_STEP_FRAMES)).then(Routine::AnimateBoulderDust(left - 1))
    }
}

/// `NPC_MOVEMENT_*` and the `$ff` that ends a path, as `PushBoulder*MovementData` holds them.
const NPC_MOVEMENT_DOWN: u8 = 0x00;
const NPC_MOVEMENT_UP: u8 = 0x40;
const NPC_MOVEMENT_LEFT: u8 = 0x80;
const NPC_MOVEMENT_RIGHT: u8 = 0xC0;
const STAY: u8 = 0xFF;


#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::map::Map;
    use poke_core::move_name::PokemonMoveName;
    use poke_core::species::PokemonSpecies;
    use poke_core::symbols::pokered_events::EVENT_FOLLOWED_OAK_INTO_LAB;
    use crate::command::{Command, Decision};
    use crate::mode::{Mode, ModeUpdate, Status};
    use crate::modes::start_menu::StartMenuEntry;
    use crate::party::Named;
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::systems::overworld::sprites::SpriteState;
    use crate::systems::overworld::{Direction, Location};
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    /// `wObtainedBadges` full, so a refusal can take one badge away at a time.
    const ALL_BADGES: u8 = 0xFF;
    /// `wMapPalOffset` as the warp into Rock Tunnel leaves it.
    const DARK: u8 = 6;

    fn game_with(map: Map, x: u8, y: u8, facing: SpriteFacing, knows: PokemonMoveName, badges: u8,
        map_pal_offset: u8) -> Game
    {
        let mut world = World { player_name: encode("RED").unwrap(), badges, ..World::default() };
        world.location = Location { map, x, y, facing, last_map: Map::PalletTown, ..Location::default() };
        world.events.set(EVENT_FOLLOWED_OAK_INTO_LAB);
        let mut mon = new_party_mon(PokemonSpecies::Squirtle, 20, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.moves[0] = Some(knows);
        world.party = vec![Named { mon, ot: encode("RED").unwrap(), nick: encode("SQUIRT").unwrap() }];
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        let mut overworld = Overworld::new();
        overworld.rt.no_battles = true;
        overworld.map_pal_offset = map_pal_offset;
        game.push(Mode::Overworld(overworld));
        settle(&mut game);
        game
    }

    /// To the next overworld poll, answering any text on the way.
    fn settle(game: &mut Game) -> u32 {
        for frames in 1..3000 {
            if game.status() == Status::Waiting(Decision::Overworld) {
                return frames;
            }
            if game.status() == Status::Waiting(Decision::Text) {
                game.frame(Input::Command(Command::Advance));
            } else {
                game.frame(Input::None);
            }
        }
        panic!("the overworld never settled: {:?}", game.modes().last().map(Mode::status));
    }

    fn until(game: &mut Game, decision: Decision) {
        for _ in 0..900 {
            if game.status() == Status::Waiting(decision.clone()) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("never waited for {decision:?}, stuck at {:?}", game.status());
    }

    /// START, POKéMON, the only mon, and the field move at the top of its submenu.
    fn use_the_field_move(game: &mut Game) {
        game.frame(Input::Command(Command::OpenStartMenu));
        until(game, Decision::StartMenu);
        game.frame(Input::Command(Command::ChooseStartMenuEntry(StartMenuEntry::Pokemon)));
        until(game, Decision::PartyMenu);
        game.frame(Input::Command(Command::ChooseOption(0)));
        until(game, Decision::FieldMoveMenu);
        game.frame(Input::Command(Command::ChooseOption(0)));
    }

    /// The refusal's text, then the list it goes back to.
    fn refused(game: &mut Game) {
        until(game, Decision::Text);
        game.frame(Input::Command(Command::Advance));
        until(game, Decision::PartyMenu);
    }

    fn overworld(game: &Game) -> &Overworld {
        game.modes().iter().rev().find_map(|mode| match mode {
            Mode::Overworld(overworld) => Some(overworld),
            _ => None,
        }).expect("the overworld is on the stack")
    }

    /// Route 8's tree at (41, 10), cut from the square below it, and the block it leaves.
    const TREE: (Map, u8, u8) = (Map::Route8, 41, 11);
    const TREE_BLOCK: (u8, u8) = (0x35, 0x4C);

    fn cutting(facing: SpriteFacing, badges: u8) -> Game {
        let (map, x, y) = TREE;
        game_with(map, x, y, facing, PokemonMoveName::Cut, badges, 0)
    }

    #[test]
    fn cut_swaps_the_tree_s_block_for_the_one_without_it() {
        let mut game = cutting(SpriteFacing::Up, ALL_BADGES);
        let before = overworld(&game).view.blocks.clone();
        use_the_field_move(&mut game);
        settle(&mut game);
        assert_eq!(game.modes().len(), 1, "the start menu closed");
        let after = &overworld(&game).view.blocks;
        let changed: Vec<usize> = (0..before.len()).filter(|&i| before[i] != after[i]).collect();
        assert_eq!(changed.len(), 1, "one block swapped");
        assert_eq!((before[changed[0]], after[changed[0]]), TREE_BLOCK);
    }

    /// The swap is to the loaded block map alone, so leaving the map and coming back grows it again.
    #[test]
    fn a_cut_tree_grows_back_when_the_map_is_loaded_again() {
        let mut game = cutting(SpriteFacing::Up, ALL_BADGES);
        use_the_field_move(&mut game);
        settle(&mut game);
        let cut = overworld(&game).view.blocks.clone();
        let mut walked = game;
        for direction in [Direction::Down, Direction::Up] {
            walked.frame(Input::Command(Command::Step(direction)));
            settle(&mut walked);
        }
        assert_eq!(overworld(&walked).view.blocks, cut, "walking about does not load the map again");
        let fresh = cutting(SpriteFacing::Up, ALL_BADGES);
        assert_ne!(overworld(&fresh).view.blocks, cut, "a fresh load has the tree");
    }

    #[test]
    fn cut_without_the_cascade_badge_is_refused_and_the_list_comes_back() {
        let mut game = cutting(SpriteFacing::Up, ALL_BADGES & !(1 << 1));
        let before = overworld(&game).view.blocks.clone();
        use_the_field_move(&mut game);
        refused(&mut game);
        assert_eq!(overworld(&game).view.blocks, before, "the tree is still there");
    }

    /// Facing away from the trees: `UsedCut`'s `.nothingToCut`.
    #[test]
    fn nothing_to_cut_where_the_tile_in_front_is_not_a_tree() {
        let mut game = cutting(SpriteFacing::Down, ALL_BADGES);
        let before = overworld(&game).view.blocks.clone();
        use_the_field_move(&mut game);
        refused(&mut game);
        assert_eq!(overworld(&game).view.blocks, before);
    }

    fn flashing(badges: u8) -> Game {
        game_with(Map::PalletTown, 5, 9, SpriteFacing::Up, PokemonMoveName::Flash, badges, DARK)
    }

    #[test]
    fn flash_lights_a_dark_map() {
        let mut game = flashing(ALL_BADGES);
        use_the_field_move(&mut game);
        settle(&mut game);
        assert_eq!(game.modes().len(), 1, "the start menu closed");
        assert_eq!(overworld(&game).map_pal_offset, 0);
    }

    #[test]
    fn flash_without_the_boulder_badge_leaves_the_map_dark() {
        let mut game = flashing(ALL_BADGES & !1);
        use_the_field_move(&mut game);
        refused(&mut game);
        assert_eq!(overworld(&game).map_pal_offset, DARK);
    }

    /// Victory Road 1F's first boulder, at (5, 15), pushed up from the square below it: the rock
    /// either side of it leaves up and down the only ways it can go.
    fn victory_road(badges: u8) -> Game {
        game_with(Map::VictoryRoad1F, 5, 16, SpriteFacing::Up, PokemonMoveName::Strength, badges, 0)
    }

    /// The lowest slot holding a boulder. A boulder being pushed has `$ff` in its movement byte
    /// until the dust puts `BOULDER_MOVEMENT_BYTE_2` back, so the slot is what a test follows.
    fn boulder_slot(game: &Game) -> usize {
        overworld(game).sprites.iter()
            .position(|sprite| sprite.movement2 == BOULDER_MOVEMENT_BYTE_2)
            .expect("the floor has a boulder")
    }

    fn boulder(game: &Game, slot: usize) -> SpriteState {
        overworld(game).sprites[slot]
    }

    /// Leans on UP until the boulder has moved or the frames run out.
    fn push(game: &mut Game, slot: usize, frames: u32) -> u32 {
        let from = boulder(game, slot).map_y;
        for held in 1..=frames {
            game.frame(Input::Buttons(Joypad::UP));
            if boulder(game, slot).map_y != from {
                return held;
            }
        }
        0
    }

    #[test]
    fn strength_moves_a_boulder_a_square_and_only_on_the_second_push() {
        let mut game = victory_road(ALL_BADGES);
        assert!(!game.world().location.strength_active);
        use_the_field_move(&mut game);
        settle(&mut game);
        assert!(game.world().location.strength_active, "STRENGTH is armed");
        let slot = boulder_slot(&game);
        let from = boulder(&game, slot).map_y;
        // A pass is two frames, and the first only arms `BIT_TRIED_PUSH_BOULDER`.
        game.frame(Input::Buttons(Joypad::UP));
        game.frame(Input::Buttons(Joypad::UP));
        assert!(overworld(&game).rt.tried_push_boulder, "the first push armed it");
        assert_eq!(boulder(&game, slot).map_y, from, "and moved nothing");
        assert_ne!(push(&mut game, slot, 60), 0, "the second push moves it");
        assert_eq!(boulder(&game, slot).map_y, from - 1, "one square along");
        settle(&mut game);
        assert_eq!((game.world().location.x, game.world().location.y), (5, 16), "the player stayed where it was");
        assert!(overworld(&game).rt.pushed_boulder, "a script can see that one moved");
    }

    #[test]
    fn a_boulder_does_not_move_without_strength() {
        let mut game = victory_road(ALL_BADGES);
        let slot = boulder_slot(&game);
        assert_eq!(push(&mut game, slot, 200), 0);
    }

    #[test]
    fn strength_without_the_rainbow_badge_is_refused() {
        let mut game = victory_road(ALL_BADGES & !(1 << 3));
        use_the_field_move(&mut game);
        refused(&mut game);
        assert!(!game.world().location.strength_active);
    }

    /// `ResetUsingStrengthOutOfBattleBit`: a warp forgets it, a step about the map does not.
    #[test]
    fn a_new_map_forgets_strength() {
        let mut game = game_with(Map::PalletTown, 5, 6, SpriteFacing::Up, PokemonMoveName::Strength, ALL_BADGES, 0);
        use_the_field_move(&mut game);
        settle(&mut game);
        assert!(game.world().location.strength_active);
        game.frame(Input::Command(Command::Step(Direction::Up)));
        settle(&mut game);
        assert_eq!(game.world().location.map, Map::RedsHouse1F, "through the door");
        assert!(!game.world().location.strength_active);
    }
}


