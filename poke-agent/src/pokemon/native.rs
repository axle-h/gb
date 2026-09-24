//! The recreation read as a [`GameState`], so a policy sees one game whichever half is running it.
//!
//! The emulated side infers its state from WRAM and the drawn tilemap; the recreation holds the
//! same state as typed fields, so nearly every read here is a field copy. The exception is the
//! map, whose tables are in the cartridge either way: the ROM is kept beside the game as a lookup
//! table and the existing readers find the tables in it, because a map's blocks, warps, tileset and
//! collisions are the same bytes whichever half is playing.

use std::sync::Arc;

use gb::mmu::MMU;
use poke_core::geometry::Point8;
use poke_core::sprite::{PictureId, PlayerFacingDirection, Sprite, SpriteFacing};
use pokered::mode::{Mode, ModeUpdate, Status};
use pokered::modes::overworld::Overworld;
use pokered::party::{Named, PartyMon};
use pokered::systems::overworld::location::{Location, SURFING};
use pokered::world::World;
use pokered::Game;

use crate::pokemon::bag::Bag;
use crate::pokemon::badge::Badge;
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::map_metadata::{
    closed_door_blocks, script_cancelled_warps, strong_current_below, CurrentMap, EventGates,
    MapMetadata, MapMetadataCache,
};
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::party::PokemonParty;
use crate::pokemon::pokedex::Pokedex;
use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};
use crate::pokemon::postgame;
use crate::pokemon::strings::PokemonString;
use crate::pokemon::tile_map::MetaTileMap;
use crate::pokemon::{trash_can_position, GameState, TrashCanPuzzle};

/// The recreation, with the cartridge's own tables beside it.
pub struct NativeGame {
    game: Game,
    /// The ROM with no machine to run it: [`MapMetadataReader`] reads only ROM, and this is where
    /// it looks.
    rom: MMU,
    maps: MapMetadataCache,
}

impl std::fmt::Debug for NativeGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeGame").field("maps", &self.maps).finish_non_exhaustive()
    }
}

impl NativeGame {
    pub fn new(game: Game) -> Result<Self, String> {
        Ok(Self {
            game,
            rom: MMU::from_rom(crate::pokemon::roms::POKERED).map_err(|e| e.to_string())?,
            maps: MapMetadataCache::default(),
        })
    }

    pub fn game(&self) -> &Game {
        &self.game
    }

    pub fn game_mut(&mut self) -> &mut Game {
        &mut self.game
    }

    fn world(&self) -> &World {
        self.game.world()
    }

    /// The overworld under whatever is on top: a battle, a menu and a text box all leave it in
    /// place, and it is where the map and the sprites live.
    fn overworld(&self) -> Option<&Overworld> {
        self.game.modes().iter().rev().find_map(|mode| match mode {
            Mode::Overworld(overworld) => Some(overworld),
            _ => None,
        })
    }

    /// What kind of decision is on the table. The emulated side has to infer this from
    /// `wIsInBattle`, `wFontLoaded` and the shape of the drawn menu; here the mode stack says so.
    fn game_mode(&self) -> GameMode {
        let Some(top) = self.game.modes().last() else { return GameMode::Overworld };
        match top {
            // `wTrainerClass`, which is 0 in a wild battle: the same byte `wIsInBattle` follows.
            Mode::Battle(battle) => match battle.battle().is_some_and(|b| b.trainer_class != 0) {
                true => GameMode::TrainerBattle,
                false => GameMode::WildBattle,
            },
            Mode::NamingScreen(_) => GameMode::NamingScreen,
            // An overworld that is not waiting on the player is running a script or a scripted walk.
            Mode::Overworld(_) => match top.status() {
                Status::Waiting(_) => GameMode::Overworld,
                _ => GameMode::Script,
            },
            _ => GameMode::TextBox,
        }
    }

    /// The map as the pathfinder wants it: the cartridge's tables, with the state the running game
    /// holds laid over them.
    pub fn current_map(&self) -> Result<CurrentMap, String> {
        let location = &self.world().location;
        let map = location.map;
        let overworld = self.overworld().ok_or("no overworld under the mode stack")?;
        let gates = NativeGates(self.world());
        let wild = poke_core::wild::encounters(map);

        let metadata = if crate::pokemon::map_metadata::map_uses_runtime_blocks(map) {
            Arc::new(self.runtime_metadata(map, overworld)?)
        } else {
            self.maps.read_map(&self.rom, map)?
        };

        Ok(CurrentMap {
            player_position: Point8 { x: location.x, y: location.y },
            player_direction: facing_as_player_direction(location.facing),
            sprites: self.sprites(map, location, overworld),
            metadata,
            closed_doors: closed_door_blocks(&gates, map),
            // `LoadWildData` copies these out of the map's own table, which is in the cartridge
            // either way.
            grass_encounter_rate: wild.as_ref().map_or(0, |w| w.grass_rate),
            water_encounter_rate: wild.as_ref().map_or(0, |w| w.water_rate),
            card_key_locked: crate::pokemon::map_metadata::map_has_card_key_doors(map)
                && !self.world().bag.items.iter().any(|slot| slot.id == ItemId::CardKey),
            // The recreation loads a map's header in the same pass that changes `location.map`, so
            // there is no frame where the two disagree; the emulated side needs the check because
            // `wCurMap` is written before the load.
            header_loaded: true,
            surfing: location.walk_bike_surf == SURFING,
            sprites_loaded: true,
            standing_on_warp: overworld.standing_on_warp(),
            script_cancelled_warps: script_cancelled_warps(&gates, map),
            strong_current_below: strong_current_below(&gates, map),
        })
    }

    /// The maps whose blocks a script rewrites, where the live block map is the map.
    fn runtime_metadata(&self, map: Map, overworld: &Overworld) -> Result<MapMetadata, String> {
        const BORDER: usize = pokered::systems::map_data::MAP_BORDER;
        let view = overworld.view();
        let (w, h) = (view.width as usize, view.height as usize);
        let stride = w + BORDER * 2;
        let blocks = (0..h)
            .flat_map(|by| (0..w).map(move |bx| (by, bx)))
            .map(|(by, bx)| view.blocks[(by + BORDER) * stride + (bx + BORDER)])
            .collect();
        crate::pokemon::map_metadata::metadata_from_live_blocks(&self.rom, map, blocks)
    }

    /// The sixteen sprite slots as [`Sprite`], which is what the tile map reads people off.
    fn sprites(&self, map: Map, location: &Location, overworld: &Overworld) -> Vec<Sprite> {
        let map_sprites = map.sprites();
        overworld.sprites().iter().enumerate().skip(1) // slot 0 is the player
            .filter_map(|(index, state)| {
                let picture_id = PictureId::from_repr(state.picture_id)?;
                let map_sprite = map_sprites.get(index - 1)?;
                Some(Sprite {
                    index: index as u8,
                    picture_id,
                    position: if picture_id == PictureId::Red {
                        Point8 { x: location.x, y: location.y }
                    } else {
                        // The 4 squares `object_event` adds, taken back off; wrapping because an
                        // unfilled slot reads below the border, as it does on the cartridge.
                        Point8 { x: state.map_x.wrapping_sub(4), y: state.map_y.wrapping_sub(4) }
                    },
                    on_screen: state.image_index != 0xFF,
                    hidden: map_sprite.hidden_object_id
                        .is_some_and(|bit| location.is_hidden(bit)),
                    facing: SpriteFacing::from_repr(state.facing).unwrap_or_default(),
                    name: map_sprite.name,
                })
            })
            .collect()
    }

    /// Everything a policy is shown, from the recreation's own fields.
    pub fn game_state(&self) -> Result<GameState, String> {
        let world = self.world();
        let location = &world.location;
        let badges = Badge::from_bits(world.badges).ok_or("cannot parse badges")?;
        let pokemon = party(&world.party)?;

        let has_move = |want: PokemonMoveName| {
            pokemon.iter().any(|mon| mon.moves.iter().flatten().any(|m| m.name == want))
        };
        let in_safari_zone = matches!(location.map, Map::SafariZoneCenter | Map::SafariZoneEast
                                                  | Map::SafariZoneNorth | Map::SafariZoneWest);
        let can_use_surf = badges.contains(Badge::SoulBadge) && has_move(PokemonMoveName::Surf)
            && !location.always_on_bike && !in_safari_zone;
        let can_use_cut = badges.contains(Badge::CascadeBadge) && has_move(PokemonMoveName::Cut);

        let bag = bag(&world.bag);
        let mut map = MetaTileMap::new(&self.current_map()?);
        map.can_surf = can_use_surf;
        map.can_cut = can_use_cut;
        map.can_strength = badges.contains(Badge::RainbowBadge) && has_move(PokemonMoveName::Strength);
        map.best_rod = postgame::fishing::Rod::best_in_bag(&bag);

        let gates = NativeGates(world);
        let trash_cans = (location.map == Map::VermilionGym).then(|| TrashCanPuzzle {
            // EVENT_1ST_LOCK_OPENED (0x161), EVENT_2ND_LOCK_OPENED (0x160).
            first_opened: gates.event_byte(44) & 0x02 != 0,
            second_opened: gates.event_byte(44) & 0x01 != 0,
            first_target: trash_can_position(world.scripts.trash_cans[0]),
            second_target: trash_can_position(world.scripts.trash_cans[1]),
        });

        Ok(GameState {
            player_id: world.player_id,
            name: PokemonString::from(world.player_name.as_slice()),
            rival_name: PokemonString::from(world.rival_name.as_slice()),
            badges,
            money: bcd(&world.money),
            coins: bcd(&world.coins) as u16,
            map_is_dark: self.overworld().is_some_and(|o| o.map_pal_offset() != 0),
            pokemon,
            mode: self.game_mode(),
            map,
            bag,
            // The battle's own state is the next slice; until then a policy sees the battle in
            // `mode` and nothing else, so nothing may drive a battle through this backend yet.
            battle: None,
            boxed_pokemon: Vec::new(),
            current_box: world.current_box,
            can_use_cut,
            can_use_surf,
            has_pokedex: gates.holds_event(poke_core::symbols::pokered_events::EVENT_GOT_POKEDEX),
            pokedex_owned: Pokedex::try_from_slice(&world.pokedex.owned)?,
            pokedex_seen: Pokedex::try_from_slice(&world.pokedex.seen)?,
            trash_cans,
            // EVENT_FOUND_ROCKET_HIDEOUT = 0x1b9, EVENT_MANSION_SWITCH_ON = 0x278.
            found_rocket_hideout: gates.event_byte(55) & 0x02 != 0,
            mansion_switch_on: gates.event_byte(79) & 0x01 != 0,
            safari: None,
            strength_active: location.strength_active,
            hall_of_fame_teams: world.hall_of_fame_teams,
            repel_steps: location.repel_steps,
            on_bicycle: location.walk_bike_surf == pokered::systems::overworld::location::BIKING,
            day_care_in_use: world.day_care.is_some(),
        })
    }
}

/// [`EventGates`] over the recreation's own flags and bag.
struct NativeGates<'a>(&'a World);

impl NativeGates<'_> {
    fn holds_event(&self, event: u16) -> bool {
        self.0.events.is_set(event)
    }
}

impl EventGates for NativeGates<'_> {
    fn event_byte(&self, index: u16) -> u8 {
        self.0.events.as_bytes().get(index as usize).copied().unwrap_or(0)
    }

    fn bag_holds(&self, item: ItemId) -> bool {
        self.0.bag.items.iter().any(|slot| slot.id == item)
    }
}

/// `wPlayerDirection`'s bits from `wSpritePlayerStateData1FacingDirection`'s: different byte,
/// different encoding, and the pathfinder wants the first.
fn facing_as_player_direction(facing: SpriteFacing) -> PlayerFacingDirection {
    match facing {
        SpriteFacing::Down => PlayerFacingDirection::Down,
        SpriteFacing::Up => PlayerFacingDirection::Up,
        SpriteFacing::Left => PlayerFacingDirection::Left,
        SpriteFacing::Right => PlayerFacingDirection::Right,
    }
}

/// BCD as the cartridge keeps money and coins, two digits to a byte.
fn bcd(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |total, byte| total * 100 + (byte >> 4) as u32 * 10 + (byte & 0xF) as u32)
}

fn bag(inventory: &pokered::systems::inventory::Inventory) -> Bag {
    Bag::new(inventory.items.clone())
}

fn party(mons: &[Named<PartyMon>]) -> Result<PokemonParty, String> {
    let mut party = PokemonParty::default();
    for mon in mons {
        party.push(party_mon(mon)?)?;
    }
    Ok(party)
}

/// `party_struct`, field for field. The emulated side decodes these same fields out of the
/// 44 bytes WRAM holds them in.
fn party_mon(named: &Named<PartyMon>) -> Result<Pokemon, String> {
    let mon = &named.mon.mon;
    let stats = |raw: [u16; 5]| PokemonStats {
        hp: raw[0], attack: raw[1], defense: raw[2], speed: raw[3], special: raw[4],
    };
    Ok(Pokemon {
        nickname: PokemonString::from(named.nick.as_slice()),
        trainer_name: PokemonString::from(named.ot.as_slice()),
        species: mon.species,
        current_hp: mon.hp,
        status: mon.status.into(),
        types: [
            PokemonType::from_repr(mon.types[0]).ok_or("Invalid Pokemon type")?,
            PokemonType::from_repr(mon.types[1]).ok_or("Invalid Pokemon type")?,
        ],
        // PP is the raw byte, PP Ups in the top two bits, as the emulated reader leaves it.
        moves: std::array::from_fn(|i| {
            mon.moves[i].map(|name| PokemonMove { name, pp: mon.pp[i] })
        }),
        trainer_id: mon.ot_id,
        experience: mon.exp,
        effort_values: stats(mon.stat_exp),
        individual_values: PokemonStats::from_iv_bytes(mon.dvs.0[0], mon.dvs.0[1]),
        level: named.mon.level,
        stats: stats(named.mon.stats),
    })
}
