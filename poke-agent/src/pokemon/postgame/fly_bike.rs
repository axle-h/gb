//! Fly, the Bicycle, and Cycling Road.

use gb::joypad::JoypadButton;
use gb::mmu::MMU;
use crate::pokemon::agent::{AgentEvent, AgentState, PokemonAgent};
use crate::pokemon::encoding::{GameMode, PokemonEncoding};
use crate::pokemon::font::FontAware;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::policy::PolicyStep;
use crate::pokemon::{PokemonApi, PokemonApiTrait};

/// Fly reaches map ids `0..FLY_DESTINATIONS`, which is also the order the town-map cursor walks
/// them (`BuildFlyLocationsList`).
pub(crate) const FLY_DESTINATIONS: u8 = 11;

/// `wStatusFlags6` bit 3: set when the town map accepts a destination, cleared as the warp runs.
const BIT_FLY_WARP: u8 = 1 << 3;

/// `wStatusFlags7` bit 7: set alongside `BIT_FLY_WARP`, cleared by the bird animation on arrival.
const BIT_USED_FLY: u8 = 1 << 7;

/// Tilesets `CheckIfInOutsideMap` accepts: `OVERWORLD` and `PLATEAU`.
const OUTSIDE_TILESETS: [u8; 2] = [0, 23];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlyState {
    pub to: Map,
    /// Press/release alternation, so every input is a fresh rising edge.
    press: bool,
    entered_menu: bool,
    /// When FLY was chosen, after which nothing is pressed until the town map is on screen.
    chose_fly_at: Option<u16>,
    /// Set once the font has been seen loaded during this flight's menu chain.
    saw_font: bool,
    /// Ticks spent driving, so a wedge reports itself.
    ticks: u16,
}

/// How long `LoadTownMap_Fly` may take to put the town map on screen, in agent ticks.
const TOWN_MAP_LOAD_TICKS: u16 = 150;

const TICK_BUDGET: u16 = 1200;

impl FlyState {
    pub fn new(to: Map) -> Self {
        Self { to, press: true, entered_menu: false, chose_fly_at: None, saw_font: false, ticks: 0 }
    }

    fn blocked_by(&self, mmu: &MMU) -> Option<String> {
        if (self.to as u8) >= FLY_DESTINATIONS {
            return Some(format!("{} is not a Fly destination (only the {FLY_DESTINATIONS} towns are)", self.to));
        }
        if !visited_towns(mmu).contains(&self.to) {
            return Some(format!("{} has never been visited, so it is not on the town map", self.to));
        }
        let tileset = mmu.read_pointer(&pokered_symbols::wCurMapTileset);
        if !OUTSIDE_TILESETS.contains(&tileset) {
            return Some(format!("Fly needs an outside map (tileset {tileset} is indoors)"));
        }
        if flyer_slot(mmu).is_none() {
            return Some("no party member knows FLY".into());
        }
        None
    }
}

/// The towns Fly can reach: `wTownVisitedFlag` is 16-bit little-endian, bit n being map id n.
pub(crate) fn visited_towns(mmu: &MMU) -> Vec<Map> {
    let flags = mmu.read_pointer_u16_le(&pokered_symbols::wTownVisitedFlag);
    (0..FLY_DESTINATIONS)
        .filter(|i| flags & (1 << i) != 0)
        .filter_map(Map::from_repr)
        .collect()
}

fn flyer_slot(mmu: &MMU) -> Option<u8> {
    mmu.read_player_pokemon_party().ok()?
        .iter().position(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Fly))
        .map(|i| i as u8)
}

/// `(party slot, FLY's row in its field-move box)` from one party read, as this runs every tick.
fn fly_menu_indices(api: &PokemonApi<'_>) -> (u8, u8) {
    let Ok(party) = api.mmu().read_player_pokemon_party() else { return (0, 0) };
    let flyer = party.iter().position(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Fly));
    match flyer {
        Some(slot) => (slot as u8, crate::pokemon::policy::field_move_index_of(&party[slot], PokemonMoveName::Fly)),
        None => (0, 0),
    }
}

/// Packed town-map coordinate of `map`, low nibble x and high nibble y (`LoadTownMapEntry`).
fn town_map_coords(mmu: &MMU, map: Map) -> u8 {
    mmu.read_pointer(&(pokered_symbols::ExternalMapEntries + map as u16 * 3))
}

fn is_town_coordinate(mmu: &MMU, packed: u8) -> bool {
    (0..FLY_DESTINATIONS).filter_map(Map::from_repr).any(|m| town_map_coords(mmu, m) == packed)
}

/// One tick of the Fly driver: START, POKéMON, the Fly mon, FLY, then Up on the town map until the
/// cursor is on the target, A, and hands off until the bird lands.
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: FlyState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    // `wCurMap` rather than `game_state()`, which decodes far more than a map id every tick.
    let on_target = api.mmu().read_pointer(&pokered_symbols::wCurMap) == s.to as u8;
    // The town map is not a menu: it shows as the font unloading after FLY, cursor on a town.
    let cursor_on = api.mmu().read_pointer(&pokered_symbols::wTownMapCoords);
    let font_loaded = api.mmu().pokemon_font_loaded();
    let s = FlyState { saw_font: s.saw_font || font_loaded, ..s };
    let town_map_open = s.chose_fly_at.is_some()
        && s.saw_font
        && !font_loaded
        && game_mode != GameMode::Overworld
        && is_town_coordinate(api.mmu(), cursor_on);
    // Between them the two flags cover the flight from the town map accepting to the bird landing.
    let in_flight = api.mmu().read_pointer(&pokered_symbols::wStatusFlags6) & BIT_FLY_WARP != 0
        || api.mmu().read_pointer(&pokered_symbols::wStatusFlags7) & BIT_USED_FLY != 0;

    let abort = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, why: String| {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("Fly: {why}") });
        agent.set_state(AgentState::Idle);
    };

    if s.entered_menu && on_target && !in_flight && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("Flew to {}", s.to) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // Checked before the in-flight wait: a state restored mid-animation keeps `BIT_USED_FLY` set
    // with no flight to clear it.
    if s.ticks > TICK_BUDGET {
        abort(agent, api, format!("no progress in {TICK_BUDGET} ticks (still not on {})", s.to));
        return Ok(());
    }

    let mut s = FlyState { ticks: s.ticks + 1, ..s };
    let wait = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: FlyState| {
        api.release_all_buttons();
        agent.set_state(AgentState::Flying(FlyState { press: true, ..s }));
    };

    // In the air, the animation and the warp own the joypad.
    if in_flight {
        s.entered_menu = true; // a state restored mid-flight has never opened a menu of ours
        wait(agent, api, s);
        return Ok(());
    }

    if !s.entered_menu {
        if let Some(why) = s.blocked_by(api.mmu()) {
            abort(agent, api, format!("cannot fly to {} — {why}", s.to));
            return Ok(());
        }
    }

    if town_map_open {
        if !s.press {
            wait(agent, api, s);
            return Ok(());
        }
        api.release_all_buttons();
        api.press_button(if cursor_on == town_map_coords(api.mmu(), s.to) { JoypadButton::A } else { JoypadButton::Up });
        agent.set_state(AgentState::Flying(FlyState { press: false, ..s }));
        return Ok(());
    }

    // Back in the overworld on the wrong map: the attempt fizzled.
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }
    s.entered_menu |= game_mode != GameMode::Overworld;

    if let Some(chosen_at) = s.chose_fly_at {
        if s.ticks.saturating_sub(chosen_at) < TOWN_MAP_LOAD_TICKS {
            wait(agent, api, s);
            return Ok(());
        }
        // Not up after that long, so the A was swallowed by a field-move menu still being drawn.
        s.chose_fly_at = None;
    }

    if !s.press {
        wait(agent, api, s);
        return Ok(());
    }

    let (slot, move_index) = fly_menu_indices(api);
    let button = if game_mode == GameMode::Overworld {
        JoypadButton::Start
    } else {
        crate::pokemon::agent::field_move_menu_button(api, slot, move_index)
    };
    // A on the field-move box chooses FLY, the last input until the town map is up.
    let choosing_fly = button == JoypadButton::A
        && api.menu_state().is_some_and(|m| m.is_field_move_menu());
    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::Flying(FlyState {
        press: false,
        chose_fly_at: if choosing_fly { Some(s.ticks) } else { s.chose_fly_at },
        ..s
    }));
    Ok(())
}

impl PolicyStep {
    /// Viridian City → Vermilion City the short way: Diglett's Cave, not the Pewter/Mt Moon loop.
    fn viridian_to_vermilion() -> Vec<Self> {
        vec![
            Self::enter(Map::ViridianCity),
            // Venusaur is the only Cut holder; the Cut executor only ever asks slot 0.
            Self::MovePokemonToFront { target: crate::pokemon::policy::PartyRef::Slot(1) },
            Self::enter(Map::Route2),
            Self::CutTree { map: Map::Route2 },
            Self::enter(Map::Route2Gate),
            Self::enter_at(Map::Route2, 16, 35),
            Self::CutTree { map: Map::Route2 },
            Self::enter(Map::DiglettsCaveRoute2),
            Self::enter(Map::DiglettsCave),
            Self::enter(Map::DiglettsCaveRoute11),
            Self::enter(Map::Route11),
            Self::enter(Map::VermilionCity),
        ]
    }

    /// The Bike Voucher, from `postgame-entry.bin` (the Viridian Pokémon Center).
    pub fn bike_voucher_steps() -> Vec<Self> {
        let mut s = Self::viridian_to_vermilion();
        s.push(Self::enter(Map::PokemonFanClub));
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::POKEMONFANCLUB_CHAIRMAN), 3));
        s
    }

    /// The Bicycle, from `postgame-bike-voucher.bin` (inside the Pokémon Fan Club).
    pub fn bicycle_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::VermilionCity), // out of the Fan Club
            Self::enter(Map::Route6),
            Self::enter(Map::UndergroundPathRoute6),
            Self::enter(Map::UndergroundPathNorthSouth),
            Self::enter(Map::UndergroundPathRoute5),
            Self::enter(Map::Route5),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::BikeShop),
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::BIKESHOP_CLERK), 3));
        s
    }

    /// HM02 Fly, from `postgame-bicycle.bin` (inside the Cerulean Bike Shop).
    pub fn hm02_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::CeruleanCity),           // out of the Bike Shop, into the main terrace
            Self::enter(Map::CeruleanTrashedHouse),   // front door (27,11)
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door → the Route-5 pocket
            Self::enter(Map::Route5),
            Self::enter(Map::Route5Gate),             // north door (9/10,29)
            Self::enter_at(Map::Route5, 10, 33),      // south door — `BIT_GAVE_SAFFRON_GUARDS_DRINK` is set
            Self::enter(Map::SaffronCity),
            // The plain Route 7 connection lands in a ledge-sealed pocket with no path to the gate.
            Self::enter_at(Map::Route7, 19, 10),
            Self::enter(Map::Route7Gate),             // east door (18,9/10)
            Self::enter_at(Map::Route7, 11, 10),      // west door → the Celadon side
            Self::enter(Map::CeladonCity),
            Self::enter(Map::Route16),                // Celadon west → the lower (Cycling Road) road
            Self::CutTree { map: Map::Route16 },      // pops harmlessly if nothing is cuttable here
            Self::enter(Map::Route16Gate1F),          // east-lower door (24,10/11)
            Self::enter_at(Map::Route16, 17, 4),      // past the guard, out of the west-upper door
            Self::CutTree { map: Map::Route16 },
            Self::enter(Map::Route16FlyHouse),
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE16FLYHOUSE_BRUNETTE_GIRL), 3));
        s
    }

    /// Teach Fly to the one compatible party member, then fly out of Route 16.
    pub fn teach_and_use_fly_steps(to: Map) -> Vec<Self> {
        vec![
            Self::TeachMove { item: crate::pokemon::item::ItemId::Hm02Fly,
                             target: crate::pokemon::policy::PartyRef::Slot(1) },
            Self::enter(Map::Route16), // outside, so the town map will open
            Self::Fly { to },
        ]
    }

    /// Wake the Route 16 Snorlax, then ride Cycling Road to Fuchsia.
    pub fn cycling_road_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::Route16),
            Self::CutTree { map: Map::Route16 },
            Self::UseFieldItem { item: crate::pokemon::item::ItemId::PokeFlute, target: MapSprite::ROUTE16_SNORLAX },
            Self::CutTree { map: Map::Route16 },
            Self::enter(Map::Route16Gate1F),     // east-lower door (24,10/11)
            Self::enter_at(Map::Route16, 17, 10), // west-lower door → forced onto the bike
            Self::enter(Map::Route17),           // Cycling Road, southbound
            // Route 18's top edge is water on both flanks, and a plain `enter` picks water and
            // tries to mount Surf on Cycling Road for ever.
            Self::enter_at(Map::Route18, 13, 0),
            // Route 18's gate is a plain east-west corridor with the Fuchsia connection beyond it.
            Self::enter(Map::Route18Gate1F),
            Self::enter_at(Map::Route18, 40, 8),
            Self::enter(Map::FuchsiaCity),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb::ram::RAM;

    /// The town-map coordinate table the Fly driver steers by matches the ROM, and no two agree.
    #[test]
    fn town_map_coordinates_match_the_rom_table() {
        let mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();

        // (map, x, y) — `external_map x, y, <name>`, in map-id order.
        const EXPECTED: [(Map, u8, u8); FLY_DESTINATIONS as usize] = [
            (Map::PalletTown, 2, 11),
            (Map::ViridianCity, 2, 8),
            (Map::PewterCity, 2, 3),
            (Map::CeruleanCity, 10, 2),
            (Map::LavenderTown, 14, 5),
            (Map::VermilionCity, 10, 9),
            (Map::CeladonCity, 7, 5),
            (Map::FuchsiaCity, 8, 13),
            (Map::CinnabarIsland, 2, 15),
            (Map::IndigoPlateau, 0, 2),
            (Map::SaffronCity, 10, 5),
        ];

        for (map, x, y) in EXPECTED {
            let packed = town_map_coords(&mmu, map);
            assert_eq!((packed & 0x0F, packed >> 4), (x, y), "{map} town-map coordinate");
        }

        let all: Vec<u8> = EXPECTED.iter().map(|&(m, ..)| town_map_coords(&mmu, m)).collect();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "two towns share a town-map coordinate: {all:02x?}");
    }

    /// `wTownVisitedFlag` decodes to the towns Fly can reach, empty on a fresh ROM.
    #[test]
    fn visited_towns_reads_the_bitfield() {
        let mut mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
        assert!(visited_towns(&mmu).is_empty(), "a fresh ROM has visited no towns");

        // Bit 10 (Saffron) lives in the second byte, which a byte-at-a-time reader would miss.
        mmu.write(pokered_symbols::wTownVisitedFlag.address, 0b0000_0101);
        mmu.write(pokered_symbols::wTownVisitedFlag.address + 1, 0b0000_0100);
        assert_eq!(visited_towns(&mmu), vec![Map::PalletTown, Map::PewterCity, Map::SaffronCity]);
    }
}
