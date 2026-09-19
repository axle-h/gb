//! `ItemUseOldRod`, `ItemUseGoodRod` and `ItemUseSuperRod` from `FishingInit`'s text on, with
//! `RodResponse` and `FishingAnim`. The bag has already turned a rod away from anything but water,
//! or while surfing (`fishing_refused`), and answers with the rod otherwise; the rest runs here, over
//! the map, before the start menu's `CloseTextDisplay`, as it does on the cartridge.
//!
//! A bite leaves `wCurOpponent` and `wCurEnemyLevel` for `.checkForOpponent`, which starts the battle
//! as it would any other. `RodResponse` clears `wWalkBikeSurfState` around the animation and puts it
//! back, and nothing between reads it.

use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::map_header::MapHeader;
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::{pokered_symbols, DmgBank, DmgPointer};
use crate::audio::data::sounds;
use crate::gfx::layers::Object;
use crate::gfx::tiles::V_CHARS0;
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::mode::{Ctx, Outcome};
use crate::rng::Rng;
use crate::systems::overworld::bike_surf::is_next_tile_shore_or_water;
use crate::systems::overworld::location::SURFING;
use super::script::{text_at, Block, Flow, Routine, Then};
use super::Overworld;

/// `wRodResponse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RodResponse {
    NoBite,
    Bite { species: PokemonSpecies, level: u8 },
    /// The Super Rod on a map `SuperRodData` does not list.
    NoFish,
}

impl RodResponse {
    fn byte(self) -> u8 {
        match self {
            Self::NoBite => 0,
            Self::Bite { .. } => 1,
            Self::NoFish => 2,
        }
    }
}

/// `FishingInit`'s refusals, judged on what was in front of the player as the start menu opened.
pub fn fishing_refused(ctx: &Ctx) -> bool {
    let location = &ctx.world.location;
    let tileset = MapHeader::read(location.map).expect("the player stands on a map with a header").tileset;
    !is_next_tile_shore_or_water(tileset, location.ahead.tile) || location.walk_bike_surf == SURFING
}

/// The rod the start menu closed on, if it closed on one. No field move's id is a rod's, so the
/// party menu's answers never read as one.
pub(super) fn rod_chosen(outcome: Option<Outcome>) -> Option<ItemId> {
    let Some(Outcome::Chosen(id)) = outcome else { return None };
    ItemId::from_repr(id).filter(|rod| matches!(rod, ItemId::OldRod | ItemId::GoodRod | ItemId::SuperRod))
}

fn mon(entry: &[u8]) -> RodResponse {
    let species = PokemonSpecies::from_repr(entry[1]).expect("a fishing entry names a species");
    RodResponse::Bite { species, level: entry[0] }
}

/// The rod's own half: a Magikarp every time, `GoodRodMons`, or `ReadSuperRodData`, with the
/// `Random` calls each makes.
pub fn rod_response(rod: ItemId, map: Map, rng: &mut impl Rng) -> RodResponse {
    match rod {
        ItemId::OldRod => RodResponse::Bite { species: PokemonSpecies::Magikarp, level: 5 },
        ItemId::GoodRod => good_rod(rng),
        _ => read_super_rod_data(map, rng),
    }
}

/// `ItemUseGoodRod.RandomLoop`: bit 0 set is no bite, else the next two bits pick one of the two
/// mons, drawing again when they name neither.
fn good_rod(rng: &mut impl Rng) -> RodResponse {
    loop {
        let a = rng.random();
        if a & 1 != 0 {
            return RodResponse::NoBite;
        }
        let pick = (a >> 1) & 0b11;
        if pick < 2 {
            return mon(&rom_slice(pokered_symbols::GoodRodMons + 2 * pick as u16)[..2]);
        }
    }
}

/// `ReadSuperRodData`: the map's group from `SuperRodData`, then half the time no bite and otherwise
/// a mon, drawing again when the two bits pass the group's end.
fn read_super_rod_data(map: Map, rng: &mut impl Rng) -> RodResponse {
    let index = rom_slice(pokered_symbols::SuperRodData);
    let Some(row) = index.chunks(3).take_while(|row| row[0] != 0xFF).find(|row| row[0] == map as u8) else {
        return RodResponse::NoFish;
    };
    let group = rom_slice(DmgPointer { bank: pokered_symbols::SuperRodData.bank, address: u16::from_le_bytes([row[1], row[2]]) });
    let count = group[0];
    loop {
        let a = rng.random();
        if a & 1 != 0 {
            return RodResponse::NoBite;
        }
        let pick = (a >> 1) & 0b11;
        if pick < count {
            let at = 1 + 2 * pick as usize;
            return mon(&group[at..at + 2]);
        }
    }
}

/// `FishingAnim`'s `DelayFrames 10`, its `DelayFrames 100`, and the ten shakes a `Delay3` apart.
const BEFORE_CAST_FRAMES: u8 = 10;
const CAST_FRAMES: u8 = 100;
const SHAKES: u8 = 10;
const SHAKE_FRAMES: u8 = 3;
/// `FishingInit`'s `DelayFrames 80`.
const INIT_FRAMES: u8 = 80;
/// `wShadowOAMSprite39`, the last of the four `BIT_LEDGE_OR_FISHING` keeps out of `PrepareOAMData`.
const ROD: usize = 39;
const OAM_HIDDEN_Y: u8 = 160;
/// Where the rod goes back to when the player faces up, once the bubble is gone.
const ROD_UP_Y: u8 = 0x44;
/// `EXCLAMATION_BUBBLE`.
const EXCLAMATION_BUBBLE: u8 = 0;
/// `vNPCSprites`, which `FishingAnim` loads the player's standing frames and the rod into.
const V_NPC_SPRITES: u16 = 0x8000;

impl Overworld {
    /// `FishingInit` once the refusals have passed: `ItemUseReloadOverworldData`, then the rod's
    /// `ItemUseText00`.
    pub(super) fn fishing_init(&mut self, ctx: &mut Ctx, rod: ItemId) -> Flow {
        ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
        self.update_sprites(ctx);
        Then::block(Block::PrintText(text_at(pokered_symbols::ItemUseText00))).then(Routine::FishingInitSound(rod))
    }

    /// `FishingInit`'s sound and its wait, then `RodResponse`.
    pub(super) fn fishing_init_sound(&mut self, ctx: &mut Ctx, rod: ItemId) -> Flow {
        ctx.audio.play_sound(sounds::SFX_HEAL_AILMENT);
        Then::block(Block::Frames(INIT_FRAMES)).then(Routine::RodResponse(rod))
    }

    /// `RodResponse`: a bite leaves its mon to be fought, then `FishingAnim`'s first wait.
    pub(super) fn rod_response(&mut self, ctx: &mut Ctx, rod: ItemId) -> Flow {
        let response = rod_response(rod, ctx.world.location.map, ctx.rng);
        if let RodResponse::Bite { species, level } = response {
            self.rt.cur_enemy_level = level;
            self.rt.cur_opponent = species as u8;
        }
        Then::block(Block::Frames(BEFORE_CAST_FRAMES)).then(Routine::FishingCast(response.byte()))
    }

    /// `FishingAnim` after its first wait: the player's standing frames with the fishing ones over
    /// them, and the rod, which `BIT_LEDGE_OR_FISHING` keeps in the last four objects.
    pub(super) fn fishing_cast(&mut self, ctx: &mut Ctx, response: u8) -> Flow {
        // `BIT_LEDGE_OR_FISHING`, which is what `jumping` is.
        self.jumping = true;
        let vram = |address: u16| V_CHARS0 + (address - V_NPC_SPRITES) as usize / TILE_BYTES;
        ctx.screen.tiles.load(V_CHARS0, &rom_slice(pokered_symbols::RedSprite)[..12 * TILE_BYTES]);
        // `LoadAnimSpriteGfx` over `RedFishingTiles`: the tiles, their count, their bank and where.
        for entry in rom_slice(pokered_symbols::RedFishingTiles)[..4 * 6].chunks(6) {
            let source = DmgPointer { bank: DmgBank::ROM { bank: entry[3] }, address: u16::from_le_bytes([entry[0], entry[1]]) };
            let count = entry[2] as usize;
            ctx.screen.tiles.load(vram(u16::from_le_bytes([entry[4], entry[5]])), &rom_slice(source)[..count * TILE_BYTES]);
        }
        let oam = rom_slice(pokered_symbols::FishingRodOAM + self.player().image_index as u16);
        ctx.screen.sprites.resize(40, Object { y: OAM_HIDDEN_Y, ..Object::default() });
        ctx.screen.sprites[ROD] = Object { y: oam[0], x: oam[1], tile: oam[2], attributes: oam[3] };
        let next = match response {
            0 => Routine::FishingText(pokered_symbols::NoNibbleText),
            2 => Routine::FishingText(pokered_symbols::NothingHereText),
            _ => Routine::FishingShake(SHAKES),
        };
        Then::block(Block::Frames(CAST_FRAMES)).then(next)
    }

    fn facing_up(&self) -> bool {
        self.player().image_index == SpriteFacing::Up as u8
    }

    /// `FishingAnim.loop`: the player and the rod a pixel up or down, then a `Delay3`; after the
    /// last, the exclamation bubble over the player, with the rod out of its way when they face up.
    pub(super) fn fishing_shake(&mut self, ctx: &mut Ctx, left: u8) -> Flow {
        if left > 0 {
            self.sprites[0].y_pixels ^= 1;
            ctx.screen.sprites[ROD].y ^= 1;
            return Then::block(Block::Frames(SHAKE_FRAMES)).then(Routine::FishingShake(left - 1));
        }
        if self.facing_up() {
            ctx.screen.sprites[ROD].y = OAM_HIDDEN_Y;
        }
        self.emotion_bubble(ctx, 0, EXCLAMATION_BUBBLE);
        let bubble = Block::Chain(Box::new(Block::Frames(super::movement::EMOTION_BUBBLE_FRAMES)), Routine::EmotionBubbleEnd);
        Then::block(bubble).then(Routine::FishingBite)
    }

    /// `.skipHidingFishingRod` on: the rod back where it was, and `ItsABiteText`.
    pub(super) fn fishing_bite(&mut self, ctx: &mut Ctx) -> Flow {
        if self.facing_up() {
            ctx.screen.sprites[ROD].y = ROD_UP_Y;
        }
        self.fishing_text(pokered_symbols::ItsABiteText)
    }

    pub(super) fn fishing_text(&mut self, text: DmgPointer) -> Flow {
        Then::block(Block::PrintText(text_at(text))).then(Routine::FishingEnd)
    }

    /// `FishingAnim.done` after its text, and the start menu's close that follows `UseItem`.
    pub(super) fn fishing_end(&mut self, ctx: &mut Ctx) -> Flow {
        self.jumping = false;
        ctx.screen.tiles.load_font();
        Flow::Jump(Routine::CloseTextDisplay.into())
    }
}

#[cfg(test)]
mod tests {
    use crate::rng::GameRng;
    use super::*;

    fn response(rod: ItemId, map: Map, tape: Vec<u8>) -> (RodResponse, usize) {
        let mut rng = GameRng::tape(tape);
        let response = rod_response(rod, map, &mut rng);
        let GameRng::Tape { cursor, .. } = rng else { unreachable!() };
        (response, cursor)
    }

    #[test]
    fn the_old_rod_always_hooks_a_level_five_magikarp_and_draws_nothing() {
        assert_eq!(response(ItemId::OldRod, Map::PalletTown, vec![]),
            (RodResponse::Bite { species: PokemonSpecies::Magikarp, level: 5 }, 0));
    }

    #[test]
    fn the_good_rod_draws_until_bit_zero_or_a_mon_answers() {
        assert_eq!(response(ItemId::GoodRod, Map::PalletTown, vec![0b001]), (RodResponse::NoBite, 1));
        assert_eq!(response(ItemId::GoodRod, Map::PalletTown, vec![0b000]),
            (RodResponse::Bite { species: PokemonSpecies::Goldeen, level: 10 }, 1));
        // Two and three name no mon, so a draw is taken again.
        assert_eq!(response(ItemId::GoodRod, Map::PalletTown, vec![0b100, 0b110, 0b010]),
            (RodResponse::Bite { species: PokemonSpecies::Poliwag, level: 10 }, 3));
    }

    #[test]
    fn the_super_rod_reads_the_map_s_group_and_finds_nothing_where_there_is_none() {
        assert_eq!(response(ItemId::SuperRod, Map::Route1, vec![]), (RodResponse::NoFish, 0));
        assert_eq!(response(ItemId::SuperRod, Map::PalletTown, vec![0b1]), (RodResponse::NoBite, 1));
        // Pallet Town's group has two mons, so a third is drawn again.
        assert_eq!(response(ItemId::SuperRod, Map::PalletTown, vec![0b100, 0b010]),
            (RodResponse::Bite { species: PokemonSpecies::Poliwag, level: 15 }, 2));
    }

    #[test]
    fn no_field_move_answers_with_a_rod_s_id() {
        use poke_core::move_name::PokemonMoveName::*;
        for field_move in [Cut, Fly, Surf, Strength, Flash, Dig, Teleport, Softboiled] {
            assert_eq!(rod_chosen(Some(Outcome::Chosen(field_move as u8))), None, "{field_move:?}");
        }
        assert_eq!(rod_chosen(Some(Outcome::Chosen(ItemId::GoodRod as u8))), Some(ItemId::GoodRod));
    }
}
