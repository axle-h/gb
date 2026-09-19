//! `DisplayTownMap` and `LoadTownMap_Fly`: the world map from the bag, and the same picture used to
//! pick a town to fly to. `LoadTownMap_Nest`, the Pokédex's AREA, is the same picture again with an
//! icon on every place a species can be met wild, and waits for A or B.
//!
//! Both are the picture (`LoadTownMap`), a marker sprite on one square, and that square's name along
//! the top row. The bag's screen walks `TownMapOrder`, a list of 47 places that is nothing like map
//! order, and A or B closes it. Fly's walks the towns the player has visited, skipping the rest,
//! and A leaves `Location::fly_warp` for the overworld to warp on.
//!
//! The screen the bag's opens on is the map the player is standing on rather than the first row of
//! the list, while the list index stays 0: pressing UP first therefore steps to `TownMapOrder`'s
//! *second* entry, and PALLET TOWN is only reachable by going all the way round.
//!
//! Loading, and not modelled: `LoadTownMap`'s `DisableLCD` and its tile copies, and the
//! `GBPalWhiteOutWithDelay3` each way out.

use poke_core::map::Map;
use poke_core::map_objects::FIRST_INDOOR_MAP;
use poke_core::species::PokemonSpecies;
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::command::Decision;
use crate::gfx::layers::Object;
use crate::gfx::mon_icons::clear_sprites;
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::tiles::{V_CHARS0, V_CHARS1, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::systems::overworld::sprites::load_player_sprite_graphics;

/// `NOT_VISITED`, and the `$ff` at either end of `wFlyLocationsList`.
const NOT_VISITED: u8 = 0xFE;
/// `NUM_CITY_MAPS`: the towns `BuildFlyLocationsList` runs through, which are the map ids below it.
const NUM_CITY_MAPS: u8 = 11;
/// `BIRD_BASE_TILE`, where the cursor's two tiles and the bird's twelve both land.
const BIRD_BASE_TILE: u8 = 0x04;
/// The OAM blocks `WriteTownMapSpriteOAM` and `WritePlayerOrBirdSpriteOAM` write their four
/// objects into: `wShadowOAMSprite04`, `32` and `36`.
const CURSOR_BLOCK: usize = 4;
const BIRD_BLOCK: usize = 32;
const PLAYER_BLOCK: usize = 36;
/// `TownMapSpriteBlinkingAnimation`: the objects it takes down, and the counts it does it on.
const BLINKING: usize = 36;
const HIDE_AT: u8 = 25;
const SHOW_AT: u8 = 50;
/// `SCREEN_HEIGHT_PX + OAM_Y_OFS`, which parks an object below the screen.
const HIDDEN_Y: u8 = 144 + 16;
/// `.townMapFlyLoop`'s `DelayFrames 15`, which the arrow the player pressed is missing for.
const ARROW_BEAT: u8 = 15;
/// `'▲'` and `'▼'` at the right of the fly screen's top row. The up arrow is the menu cursor's own
/// tile, which is why the screen loads `TownMapUpArrow` over it.
const UP_ARROW: (usize, u8) = (18, 0xED);
const DOWN_ARROW: (usize, u8) = (19, 0xEE);
/// `NUM_WILDMONS`, the slots in a grass or a water block.
const NUM_WILDMONS: usize = 10;
/// Cerulean Cave's square, which `DisplayWildLocations` never marks.
const CERULEAN_CAVE: u8 = 0x19;
/// `'@'`, which ends a name in the cartridge's tables.
const TERMINATOR: u8 = 0x50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TownMap {
    fly: bool,
    /// `wWhichTownMapLocation`, an index into `TownMapOrder`.
    which: u8,
    /// `wFlyLocationsList`: `NOT_VISITED` or the town's map id, one per town in map order.
    list: Vec<u8>,
    /// Where in that list the bird stands. The cartridge walks it with a pointer that runs onto the
    /// `$ff` at either end, which is what makes the wrap skip nothing.
    at: u8,
    /// `wAnimCounter`.
    anim: u8,
    /// `wShadowOAMBackup`, which the blinking animation puts back.
    backup: Vec<Object>,
    phase: Phase,
    /// Whether the pad has been read since the screen was drawn.
    polled: bool,
    /// `LoadTownMap_Nest`'s species, by its index.
    #[serde(default)]
    nest: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Input,
    /// The beat after the fly screen redraws, before both arrows go back up.
    Beat(u8),
}

impl TownMap {
    /// `DisplayTownMap`, which the Town Map item and `TownMapText` both open.
    pub fn item() -> Self {
        Self { fly: false, which: 0, list: Vec::new(), at: 0, anim: 0, backup: Vec::new(),
               phase: Phase::Input, polled: false, nest: None }
    }

    /// `LoadTownMap_Nest` for `species`.
    pub fn nest(species: PokemonSpecies) -> Self {
        Self { nest: Some(species as u8), ..Self::item() }
    }

    /// `LoadTownMap_Fly`.
    pub fn fly() -> Self {
        Self { fly: true, phase: Phase::Beat(ARROW_BEAT), ..Self::item() }
    }

    /// The row a `ChooseOption` names: how far down the visited towns the bird stands.
    pub fn selected(&self) -> u8 {
        self.list.iter().take(self.at as usize).filter(|&&entry| entry != NOT_VISITED).count() as u8
    }

    /// How many towns the fly screen can be answered with.
    pub fn rows(&self) -> u8 {
        self.list.iter().filter(|&&entry| entry != NOT_VISITED).count() as u8
    }

    /// `LoadTownMap`: the picture over the whole screen, its tiles, and the map's own palette. The
    /// `TextBoxBorder` under it is covered by all 360 tiles of the picture.
    fn load_town_map(&mut self, ctx: &mut Ctx) {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        let graphics = pokered_symbols::WorldMapTileGraphics;
        let len = (pokered_symbols::WorldMapTileGraphicsEnd.address - graphics.address) as usize;
        ctx.screen.tiles.load(V_CHARS2 + 0x60, &rom_slice(graphics)[..len]);
        ctx.screen.tiles.load_1bpp(V_CHARS0 + BIRD_BASE_TILE as usize, rom_bytes(pokered_symbols::MonNestIcon,
            pokered_symbols::MonNestIconEnd));
        for (i, tile) in compressed_map().into_iter().enumerate() {
            ctx.screen.ui.set(i % SCREEN_TILES_X, i / SCREEN_TILES_X, tile);
        }
        ctx.screen.sgb.run(&PaletteCommand::TownMap);
        ctx.screen.sprites.resize(40, Object { y: HIDDEN_Y, ..Object::default() });
        self.backup = ctx.screen.sprites.clone();
    }

    /// `DrawPlayerOrBirdSprite`: the marker on the square `map` sits on, and its name.
    fn draw_player_or_bird_sprite(&mut self, ctx: &mut Ctx, map: u8, base_tile: u8) -> Vec<u8> {
        let Some((coords, name)) = load_town_map_entry(map) else { return Vec::new() };
        let block = if base_tile == 0 { PLAYER_BLOCK } else { BIRD_BLOCK };
        write_town_map_sprite_oam(&mut ctx.screen.sprites, block, coords, base_tile);
        self.backup = ctx.screen.sprites.clone();
        name
    }

    /// `.enterLoop`: the cursor moved to the square of the entry showing, and its name in the top row.
    fn draw_entry(&mut self, ctx: &mut Ctx, map: u8) {
        let Some((coords, name)) = load_town_map_entry(map) else { return };
        write_town_map_sprite_oam(&mut ctx.screen.sprites, CURSOR_BLOCK, coords, BIRD_BASE_TILE);
        ctx.screen.ui.place(1, 0, &name);
        for i in 0..4 {
            self.backup[CURSOR_BLOCK + i] = ctx.screen.sprites[CURSOR_BLOCK + i];
        }
    }

    /// `.townMapLoop`: the top row cleared, then the entry `wWhichTownMapLocation` names.
    fn town_map_loop(&mut self, ctx: &mut Ctx) {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, 1, UiSurface::BLANK);
        let map = town_map_order()[self.which as usize];
        self.draw_entry(ctx, map);
    }

    /// `TownMapSpriteBlinkingAnimation`, which is also the bag screen's one `DelayFrame`.
    fn blink(&mut self, ctx: &mut Ctx) {
        let next = self.anim.wrapping_add(1);
        self.anim = match next {
            HIDE_AT => {
                ctx.screen.sprites.resize(40, Object { y: HIDDEN_Y, ..Object::default() });
                for object in &mut ctx.screen.sprites[..BLINKING] {
                    object.y = HIDDEN_Y;
                }
                HIDE_AT
            }
            SHOW_AT => {
                ctx.screen.sprites[..BLINKING].copy_from_slice(&self.backup[..BLINKING]);
                0
            }
            counter => counter,
        };
    }

    /// `ExitTownMap`: the picture off the screen, the walking graphics back, and the palette the
    /// caller had. Fly's own way out does none of this, since whatever it goes back to redraws.
    fn exit_town_map(&self, ctx: &mut Ctx) {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        clear_sprites(&mut ctx.screen.sprites);
        let tileset = ctx.screen.map.tileset.unwrap_or_default();
        load_player_sprite_graphics(&mut ctx.screen.tiles, &mut ctx.world.location, tileset);
        ctx.screen.tiles.load_font();
        ctx.update_sprites = true;
        ctx.screen.sgb.run(&PaletteCommand::Default);
    }

    /// `DisplayWildLocations` and the title over it. A place is marked by one object with the nest
    /// icon on its square, from OAM slot 0 up; with none marked the box saying so goes up instead of
    /// the player's marker.
    fn open_nest(&mut self, ctx: &mut Ctx, species: u8) {
        let mut marked = 0;
        for map in zero_out_duplicates(find_wild_locations_of_mon(species)) {
            // A zeroed duplicate and Pallet Town look the same, and neither is marked.
            let Some((coords, _)) = (map != 0).then(|| load_town_map_entry(map)).flatten() else { continue };
            if coords == CERULEAN_CAVE || marked == ctx.screen.sprites.len() {
                continue;
            }
            let (y, x) = town_map_coords_to_oam_coords(coords);
            ctx.screen.sprites[marked] = Object { y, x, tile: BIRD_BASE_TILE, attributes: 0 };
            marked += 1;
        }
        if marked == 0 {
            ctx.screen.ui.text_box_border(1, 7, 15, 2);
            ctx.screen.ui.place(2, 9, &poke_core::charmap::encode(" AREA UNKNOWN").expect("`AreaUnknownText` encodes"));
        } else {
            self.draw_player_or_bird_sprite(ctx, ctx.world.location.map as u8, 0);
        }
        self.backup = ctx.screen.sprites.clone();
        let name = PokemonSpecies::from_repr(species).map(|species| species.name()).unwrap_or_default();
        ctx.screen.ui.place(1, 0, &name);
        ctx.screen.ui.place(1 + name.len(), 0, &poke_core::charmap::encode("'s NEST").expect("`MonsNestText` encodes"));
    }

    /// `BuildFlyLocationsList`, then the fly screen's own graphics and its `To`.
    fn open_fly(&mut self, ctx: &mut Ctx) {
        let tileset = ctx.screen.map.tileset.unwrap_or_default();
        load_player_sprite_graphics(&mut ctx.screen.tiles, &mut ctx.world.location, tileset);
        ctx.screen.tiles.load_font();
        ctx.screen.tiles.load(V_CHARS0 + BIRD_BASE_TILE as usize, &rom_slice(pokered_symbols::BirdSprite)[..12 * TILE_BYTES]);
        ctx.screen.tiles.load_1bpp(V_CHARS1 + 0x6D, rom_bytes(pokered_symbols::TownMapUpArrow, pokered_symbols::TownMapUpArrowEnd));
        let visited = ctx.world.location.towns_visited;
        self.list = (0..NUM_CITY_MAPS).map(|town| if visited & 1 << town != 0 { town } else { NOT_VISITED }).collect();
        ctx.screen.ui.place(0, 0, &poke_core::charmap::encode("To").expect("`ToText` encodes"));
        self.draw_player_or_bird_sprite(ctx, ctx.world.location.map as u8, 0);
        self.town_map_fly_loop(ctx, UP_ARROW.0);
    }

    /// `.townMapFlyLoop`: the arrow the player pressed rubbed out, the bird moved to the town the
    /// list now points at, and its name in place of the last.
    fn town_map_fly_loop(&mut self, ctx: &mut Ctx, arrow: usize) {
        ctx.screen.ui.set(arrow, 0, UiSurface::BLANK);
        ctx.screen.ui.fill(3, 0, 15, 1, UiSurface::BLANK);
        let map = self.list[self.at as usize];
        let name = self.draw_player_or_bird_sprite(ctx, map, BIRD_BASE_TILE);
        ctx.screen.ui.place(3, 0, &name);
        self.phase = Phase::Beat(ARROW_BEAT);
    }

    /// `.pressedUp` and `.pressedDown`: a step through the list, over the towns not yet visited and
    /// round the `$ff` at either end.
    fn step(&mut self, up: bool) {
        loop {
            self.at = match (up, self.at) {
                (true, at) if at + 1 == NUM_CITY_MAPS => 0,
                (true, at) => at + 1,
                (false, 0) => NUM_CITY_MAPS - 1,
                (false, at) => at - 1,
            };
            if self.list[self.at as usize] != NOT_VISITED {
                return;
            }
        }
    }
}

impl ModeUpdate for TownMap {
    fn enter(&mut self, ctx: &mut Ctx) {
        if self.fly {
            clear_sprites(&mut ctx.screen.sprites);
        }
        self.load_town_map(ctx);
        if self.fly {
            return self.open_fly(ctx);
        }
        if let Some(species) = self.nest {
            return self.open_nest(ctx, species);
        }
        let map = ctx.world.location.map as u8;
        let name = self.draw_player_or_bird_sprite(ctx, map, 0);
        ctx.screen.ui.place(1, 0, &name);
        ctx.screen.tiles.load_1bpp(V_CHARS0 + BIRD_BASE_TILE as usize,
            rom_bytes(pokered_symbols::TownMapCursor, pokered_symbols::TownMapCursorEnd));
        self.draw_entry(ctx, map);
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        if let Phase::Beat(frames) = self.phase {
            self.phase = if frames > 1 { Phase::Beat(frames - 1) } else { Phase::Input };
            if self.phase == Phase::Input {
                ctx.screen.ui.set(UP_ARROW.0, 0, UP_ARROW.1);
                ctx.screen.ui.set(DOWN_ARROW.0, 0, DOWN_ARROW.1);
            }
            return Transition::Stay;
        }
        if !self.fly {
            self.blink(ctx);
        }
        self.polled = true;
        let keys = ctx.pad.low_sensitivity(ctx.frame_counter);
        let watched = keys & (Joypad::A | Joypad::B | Joypad::UP | Joypad::DOWN);
        if watched.is_empty() {
            return Transition::Stay;
        }
        self.polled = false;
        // `WaitForTextScrollButtonPress`, which makes no sound of its own.
        if self.nest.is_some() {
            if watched.intersects(Joypad::A | Joypad::B) {
                self.exit_town_map(ctx);
                return Transition::Pop(Outcome::Done);
            }
            self.polled = true;
            return Transition::Stay;
        }
        if self.fly {
            if watched.contains(Joypad::A) {
                // `.pressedA`: `BIT_FLY_WARP` and `BIT_USED_FLY`, which the overworld's next pass takes.
                ctx.audio.play_sound(sounds::SFX_HEAL_AILMENT);
                ctx.world.location.fly_warp = Map::from_repr(self.list[self.at as usize]);
                return Transition::Pop(Outcome::Done);
            }
            ctx.audio.play_sound(sounds::SFX_TINK);
            if watched.intersects(Joypad::UP | Joypad::DOWN) {
                let up = watched.contains(Joypad::UP);
                self.step(up);
                self.town_map_fly_loop(ctx, if up { UP_ARROW.0 } else { DOWN_ARROW.0 });
                return Transition::Stay;
            }
            return Transition::Pop(Outcome::Done);
        }
        ctx.audio.play_sound(sounds::SFX_TINK);
        let entries = town_map_order().len() as u8;
        if watched.contains(Joypad::UP) {
            self.which = (self.which + 1) % entries;
        } else if watched.contains(Joypad::DOWN) {
            self.which = (self.which + entries - 1) % entries;
        } else {
            self.exit_town_map(ctx);
            return Transition::Pop(Outcome::Done);
        }
        self.town_map_loop(ctx);
        Transition::Stay
    }

    fn status(&self) -> Status {
        match (self.polled, self.fly) {
            (true, true) => Status::Waiting(Decision::FlyDestination),
            (true, false) => Status::Waiting(Decision::TownMap),
            (false, _) => Status::Busy,
        }
    }
}

/// The bytes between two labels of the same table.
fn rom_bytes(from: DmgPointer, to: DmgPointer) -> &'static [u8] {
    &rom_slice(from)[..(to.address - from.address) as usize]
}

/// `FindWildLocationsOfMon`: a map for every grass or water slot of its table that holds `species`,
/// in map order and with a map repeated for each slot. `WildDataPointers` ends at a pointer of `-1`.
fn find_wild_locations_of_mon(species: u8) -> Vec<u8> {
    let pointers = pokered_symbols::WildDataPointers;
    let table = rom_slice(pointers);
    let mut maps = Vec::new();
    for (map, pointer) in table.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).enumerate() {
        if pointer >> 8 == 0xFF {
            break;
        }
        let data = rom_slice(DmgPointer { bank: pointers.bank, address: pointer });
        let mut at = 0;
        // `CheckMapForMon`, over the grass block and then the water block.
        for _ in 0..2 {
            let rate = data[at];
            at += 1;
            if rate == 0 {
                continue;
            }
            for slot in 0..NUM_WILDMONS {
                if data[at + 2 * slot + 1] == species {
                    maps.push(map as u8);
                }
            }
            at += 2 * NUM_WILDMONS;
        }
    }
    maps
}

/// `ZeroOutDuplicatesInList`: every later copy of a map becomes 0, so each is marked once.
fn zero_out_duplicates(mut maps: Vec<u8>) -> Vec<u8> {
    for i in 0..maps.len() {
        let map = maps[i];
        for later in &mut maps[i + 1..] {
            if *later == map {
                *later = 0;
            }
        }
    }
    maps
}

/// `TownMapCoordsToOAMCoords`: the object's `y` and `x` for a square, with no centring.
fn town_map_coords_to_oam_coords(coords: u8) -> (u8, u8) {
    (((coords & 0xF0) >> 1) + 24, ((coords & 0x0F) << 3) + 24)
}

/// `TownMapOrder`: the places the bag's screen walks, in the order it walks them.
fn town_map_order() -> &'static [u8] {
    let start = pokered_symbols::TownMapOrder;
    rom_bytes(start, pokered_symbols::TownMapOrderEnd)
}

/// `CompressedMap`: runs of one tile each, `$60` up, filling all 360 cells of the screen.
fn compressed_map() -> Vec<u8> {
    let mut tiles = Vec::with_capacity(SCREEN_TILES_X * SCREEN_TILES_Y);
    for &byte in rom_slice(pokered_symbols::CompressedMap) {
        if byte == 0 {
            break;
        }
        tiles.extend(std::iter::repeat(0x60 + (byte >> 4)).take((byte & 0xF) as usize));
    }
    tiles
}

/// `LoadTownMapEntry`: the square a map marks on the picture, packed as `y` in the high nibble and
/// `x` in the low, and the map's name. An outside map indexes its own row; an indoor one belongs to
/// the first group it is below, so a whole building shares one town's square.
fn load_town_map_entry(map: u8) -> Option<(u8, Vec<u8>)> {
    let (coords, name) = if map < FIRST_INDOOR_MAP {
        let row = rom_slice(pokered_symbols::ExternalMapEntries + map as u16 * 3);
        (row[0], u16::from_le_bytes([row[1], row[2]]))
    } else {
        let table = pokered_symbols::InternalMapEntries;
        let mut row = rom_slice(table);
        let mut at = 0;
        while row[0] != 0xFF && map >= row[0] {
            at += 4;
            row = rom_slice(table + at);
        }
        if row[0] == 0xFF {
            return None;
        }
        (row[1], u16::from_le_bytes([row[2], row[3]]))
    };
    let at = DmgPointer { bank: pokered_symbols::ExternalMapEntries.bank, address: name };
    Some((coords, rom_slice(at).iter().copied().take_while(|&byte| byte != TERMINATOR).collect()))
}

#[cfg(test)]
mod tables {
    use super::*;

    #[test]
    fn the_picture_is_exactly_the_screen() {
        assert_eq!(compressed_map().len(), SCREEN_TILES_X * SCREEN_TILES_Y);
        assert!(compressed_map().iter().all(|&tile| (0x60..0x70).contains(&tile)));
    }

    #[test]
    fn the_bag_s_screen_walks_forty_seven_places_from_pallet_town() {
        assert_eq!(town_map_order().len(), 47);
        assert_eq!(town_map_order()[0], Map::PalletTown as u8);
    }

    /// An outside map has a row of its own; a building belongs to the group it is in, so Oak's lab
    /// marks the same square as the town around it.
    /// Route 1's grass holds Pidgey in more than one slot, and the list keeps the map once.
    #[test]
    fn a_species_is_found_once_per_slot_and_marked_once_per_map() {
        let found = find_wild_locations_of_mon(PokemonSpecies::Pidgey as u8);
        let route_1 = found.iter().filter(|&&map| map == Map::Route1 as u8).count();
        assert!(route_1 > 1, "a map a slot, {route_1} for Route 1");
        let marked = zero_out_duplicates(found);
        assert_eq!(marked.iter().filter(|&&map| map == Map::Route1 as u8).count(), 1);
        assert!(find_wild_locations_of_mon(PokemonSpecies::Bulbasaur as u8).is_empty());
    }

    #[test]
    fn a_map_s_square_and_name_come_off_the_two_tables() {
        let (coords, name) = load_town_map_entry(Map::PalletTown as u8).unwrap();
        assert_eq!(coords, 0xB2, "x 2, y 11");
        assert_eq!(name, poke_core::charmap::encode("PALLET TOWN").unwrap());
        assert_eq!(load_town_map_entry(Map::OaksLab as u8), Some((coords, name)));
    }
}

/// `TownMapCoordsToOAMCoords` and `WriteTownMapSpriteOAM`: a two by two block of objects on the
/// square, laid out in reading order from `base_tile`.
fn write_town_map_sprite_oam(objects: &mut Vec<Object>, block: usize, coords: u8, base_tile: u8) {
    objects.resize(40, Object { y: HIDDEN_Y, ..Object::default() });
    // The block is drawn from its centre, and the borrow out of the x subtraction takes one back
    // off y, so it moves 4 left and only 3 up.
    let y = ((coords & 0xF0) >> 1).wrapping_add(24).wrapping_sub(3);
    let x = ((coords & 0x0F) << 3).wrapping_add(24).wrapping_sub(4);
    for (i, (dy, dx)) in [(0, 0), (0, 8), (8, 0), (8, 8)].into_iter().enumerate() {
        objects[block + i] = Object {
            y: y.wrapping_add(dy),
            x: x.wrapping_add(dx),
            tile: base_tile + i as u8,
            attributes: 0,
        };
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::move_name::PokemonMoveName;
    use poke_core::species::PokemonSpecies;
    use poke_core::sprite::SpriteFacing;
    use poke_core::symbols::pokered_events::EVENT_FOLLOWED_OAK_INTO_LAB;
    use crate::command::{Command, Decision, Reply};
    use crate::mode::{Mode, Status};
    use crate::modes::overworld::Overworld;
    use crate::modes::pokemon_menu::PokemonMenu;
    use crate::party::{Named, PartyMon};
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::systems::overworld::Location;
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    /// `BIT_THUNDERBADGE`.
    const THUNDER: u8 = 1 << 2;
    /// `wTownVisitedFlag` with Pallet Town, Viridian City and Pewter City in it.
    const THREE_TOWNS: u16 = 0b111;

    fn flier() -> Named<PartyMon> {
        let mut mon = new_party_mon(PokemonSpecies::Pidgey, 20, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.moves[0] = Some(PokemonMoveName::Fly);
        Named { mon, ot: encode("RED").unwrap(), nick: encode("BIRD").unwrap() }
    }

    fn world(map: Map, badges: u8, visited: u16) -> World {
        let mut world = World { party: vec![flier()], player_name: encode("RED").unwrap(), badges, ..World::default() };
        // Pallet Town's doorstep, which is where the overworld test's fly takes off from.
        world.location = Location { map, x: 5, y: 8, facing: SpriteFacing::Right, towns_visited: visited,
            last_map: Map::PalletTown, ..Location::default() };
        world.events.set(EVENT_FOLLOWED_OAK_INTO_LAB);
        world
    }

    fn started(world: World) -> Game {
        Game::new(world, GameRng::seeded(0), Pacing::Faithful)
    }

    fn until(game: &mut Game, decision: Decision) {
        for _ in 0..3000 {
            if game.status() == Status::Waiting(decision.clone()) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("never waited for {decision:?}, stuck at {:?}", game.status());
    }

    fn command(game: &mut Game, command: Command) {
        let reply = game.frame(Input::Command(command.clone())).reply;
        assert_eq!(reply, Some(Reply::Accepted), "{command:?}");
        for _ in 0..3000 {
            if game.frame(Input::None).events.iter().any(|event| matches!(event, crate::Event::CommandDone(_))) {
                return;
            }
        }
        panic!("{command:?} never finished");
    }

    /// The name along the top row, which the picture's own tiles run right up against.
    fn name_row(game: &Game, from: usize, name: &str) -> bool {
        let want = encode(name).expect("the name encodes");
        game.ui().row(0)[from..from + want.len()] == want[..]
    }

    fn text_row(game: &Game, y: usize) -> Vec<u8> {
        let row = game.ui().row(y)[1..18].to_vec();
        let end = row.iter().rposition(|&tile| tile != UiSurface::BLANK).map_or(0, |i| i + 1);
        row[..end].to_vec()
    }

    /// The bag's screen, opened over nothing, with the player standing in Cerulean City.
    fn bag_screen() -> Game {
        let mut game = started(world(Map::CeruleanCity, 0, 0));
        game.push(Mode::TownMap(TownMap::item()));
        until(&mut game, Decision::TownMap);
        game
    }

    /// The party menu's FLY, as far as whatever it leads to.
    fn chose_fly(map: Map, badges: u8) -> Game {
        let mut game = started(world(map, badges, THREE_TOWNS));
        game.push(Mode::PokemonMenu(PokemonMenu::new()));
        until(&mut game, Decision::PartyMenu);
        command(&mut game, Command::ChooseOption(0));
        until(&mut game, Decision::FieldMoveMenu);
        game.frame(Input::Command(Command::ChooseOption(0)));
        game
    }

    #[test]
    fn the_bag_s_screen_opens_where_the_player_stands_but_up_steps_to_the_second_entry() {
        let mut game = bag_screen();
        assert!(name_row(&game, 1, "CERULEAN CITY"));
        // Nothing but the pad walks this screen: it answers no `ChooseOption`.
        game.frame(Input::Buttons(Joypad::UP));
        game.frame(Input::None);
        assert!(name_row(&game, 1, "ROUTE 1"),
            "`wWhichTownMapLocation` was left at 0, so the first step is to the second row");
        assert!(matches!(game.modes().last(), Some(Mode::TownMap(_))));
    }

    #[test]
    fn b_closes_the_bag_s_screen() {
        let mut game = bag_screen();
        command(&mut game, Command::CancelOption);
        assert!(game.modes().is_empty());
        assert!(game.ui().row(0).iter().all(|&tile| tile == UiSurface::BLANK), "`ExitTownMap` takes the picture down");
    }

    /// `TownMapSpriteBlinkingAnimation` hides objects 0 to 35, which is the cursor's block and not
    /// the player's.
    #[test]
    fn the_cursor_blinks_and_the_player_s_marker_does_not() {
        let mut game = bag_screen();
        let marker = game.screen().sprites[PLAYER_BLOCK];
        for _ in 0..HIDE_AT {
            game.frame(Input::None);
        }
        assert_eq!(game.screen().sprites[CURSOR_BLOCK].y, HIDDEN_Y);
        assert_eq!(game.screen().sprites[PLAYER_BLOCK], marker);
        for _ in 0..(SHOW_AT - HIDE_AT) {
            game.frame(Input::None);
        }
        assert_ne!(game.screen().sprites[CURSOR_BLOCK].y, HIDDEN_Y, "and it comes back");
    }

    #[test]
    fn fly_without_the_thunder_badge_is_refused() {
        let mut game = chose_fly(Map::PalletTown, 0);
        until(&mut game, Decision::Text);
        assert_eq!(text_row(&game, 14), encode("No! A new BADGE").unwrap());
        command(&mut game, Command::Advance);
        until(&mut game, Decision::PartyMenu);
    }

    /// `CheckIfInOutsideMap`, and `GetPartyMonName` before the text so it names the mon chosen.
    #[test]
    fn fly_indoors_is_refused_by_the_mon_s_name() {
        let mut game = chose_fly(Map::OaksLab, THUNDER);
        until(&mut game, Decision::Text);
        assert_eq!(text_row(&game, 14), encode("BIRD can't").unwrap());
        assert_eq!(text_row(&game, 16), encode("FLY here.").unwrap());
        command(&mut game, Command::Advance);
        until(&mut game, Decision::PartyMenu);
    }

    #[test]
    fn the_fly_screen_offers_the_towns_visited_and_b_goes_back_to_the_list() {
        let mut game = chose_fly(Map::PalletTown, THUNDER);
        until(&mut game, Decision::FlyDestination);
        assert!(name_row(&game, 0, "To") && name_row(&game, 3, "PALLET TOWN"), "the bird starts at the list's head");
        // `.townMapFlyLoop` clears from column 3, so the square between `To` and the name keeps the
        // picture's tile rather than a space.
        assert_ne!(game.ui().row(0)[2], UiSurface::BLANK);
        assert_eq!(game.ui().row(0)[18..20], [UP_ARROW.1, DOWN_ARROW.1]);
        let Some(Mode::TownMap(map)) = game.modes().last() else { panic!("the fly screen is up") };
        assert_eq!((map.rows(), map.selected()), (3, 0));
        command(&mut game, Command::CancelOption);
        until(&mut game, Decision::PartyMenu);
    }

    #[test]
    fn the_town_chosen_is_flown_to() {
        let mut game = started(world(Map::PalletTown, THUNDER, THREE_TOWNS));
        game.push(Mode::Overworld(Overworld::new()));
        until(&mut game, Decision::Overworld);
        command(&mut game, Command::OpenStartMenu);
        until(&mut game, Decision::StartMenu);
        command(&mut game, Command::ChooseStartMenuEntry(crate::modes::start_menu::StartMenuEntry::Pokemon));
        until(&mut game, Decision::PartyMenu);
        command(&mut game, Command::ChooseOption(0));
        until(&mut game, Decision::FieldMoveMenu);
        command(&mut game, Command::ChooseOption(0));
        until(&mut game, Decision::FlyDestination);
        command(&mut game, Command::ChooseOption(2));
        until(&mut game, Decision::Overworld);
        assert_eq!(game.world().location.map, Map::PewterCity);
    }
}
