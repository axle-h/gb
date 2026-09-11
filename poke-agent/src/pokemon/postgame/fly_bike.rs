//! Workstream B — Fly, the Bicycle, and Cycling Road.

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

/// The maps Fly can reach: map ids `0..NUM_CITY_MAPS` — Pallet, Viridian, Pewter, Cerulean,
/// Lavender, Vermilion, Celadon, Fuchsia, Cinnabar, Indigo Plateau, Saffron, in that (map-id)
/// order, which is also the order the town-map cursor walks through them
/// (`BuildFlyLocationsList`).
pub(crate) const FLY_DESTINATIONS: u8 = 11;

/// `wStatusFlags6` bit 3 — set when the town map accepts a destination, cleared by the overworld
/// loop as it performs the warp (`constants/ram_constants.asm:119`, `home/overworld.asm:25-27`).
const BIT_FLY_WARP: u8 = 1 << 3;

/// `wStatusFlags7` bit 7 — set alongside `BIT_FLY_WARP` and cleared by the bird animation on
/// arrival (`engine/overworld/player_animations.asm:9-10`).
const BIT_USED_FLY: u8 = 1 << 7;

/// Tilesets `CheckIfInOutsideMap` accepts: `OVERWORLD` (towns and routes) and `PLATEAU` (Route 23
/// / Indigo Plateau).
const OUTSIDE_TILESETS: [u8; 2] = [0, 23];

/// Live state of an in-progress flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlyState {
    /// Where we are flying to.
    pub to: Map,
    /// Press/release alternation, so every input is a fresh rising edge.
    press: bool,
    /// Set once we have left the overworld, i.e. the menu chain has started.
    entered_menu: bool,
    /// The tick FLY was chosen from the field-move menu, after which the driver presses nothing
    /// until the town map is on screen.
    chose_fly_at: Option<u16>,
    /// Set once the font has been seen *loaded* during this flight's menu chain.
    saw_font: bool,
    /// Ticks spent driving, so a wedge reports itself instead of pulsing buttons for the whole
    /// budget.
    ticks: u16,
}

/// How long `LoadTownMap_Fly` may take to put the town map on screen, in agent ticks (20 ms
/// each).
const TOWN_MAP_LOAD_TICKS: u16 = 150;

/// Ceiling on driver ticks for one flight.
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

/// The towns Fly can currently reach, read from `wTownVisitedFlag` (a 16-bit little-endian
/// bitfield, bit *n* = map id *n*).
pub(crate) fn visited_towns(mmu: &MMU) -> Vec<Map> {
    let flags = mmu.read_pointer_u16_le(&pokered_symbols::wTownVisitedFlag);
    (0..FLY_DESTINATIONS)
        .filter(|i| flags & (1 << i) != 0)
        .filter_map(Map::from_repr)
        .collect()
}

/// The first party slot that knows Fly, if any.
fn flyer_slot(mmu: &MMU) -> Option<u8> {
    mmu.read_player_pokemon_party().ok()?
        .iter().position(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Fly))
        .map(|i| i as u8)
}

/// `(party slot, FLY's row in that mon's field-move box)`, both from one party read: this is on
/// the per-tick path, and a `GameState` would decode the map, the bag and the PC box along the
/// way.
fn fly_menu_indices(api: &PokemonApi<'_>) -> (u8, u8) {
    let Ok(party) = api.mmu().read_player_pokemon_party() else { return (0, 0) };
    let flyer = party.iter().position(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Fly));
    match flyer {
        Some(slot) => (slot as u8, crate::pokemon::policy::field_move_index_of(&party[slot], PokemonMoveName::Fly)),
        None => (0, 0),
    }
}

/// Packed town-map coordinate of `map`: `ExternalMapEntries + 3 * map_id`, low nibble x, high
/// nibble y (`data/maps/town_map_entries.asm`, `LoadTownMapEntry` at
/// `engine/items/town_map.asm:559-587`).
fn town_map_coords(mmu: &MMU, map: Map) -> u8 {
    mmu.read_pointer(&(pokered_symbols::ExternalMapEntries + map as u16 * 3))
}

/// Whether `packed` is one of the eleven towns' town-map coordinates.
fn is_town_coordinate(mmu: &MMU, packed: u8) -> bool {
    (0..FLY_DESTINATIONS).filter_map(Map::from_repr).any(|m| town_map_coords(mmu, m) == packed)
}

/// One agent tick of the Fly driver.
/// ```text
/// overworld (outside map only)                      START
///   → START menu                                    cursor → 1 (POKéMON), A
///   → party menu                                    cursor → the Fly mon, A
///   → field-move menu                               cursor → FLY's index, A
///   → the town map                                  Up until wTownMapCoords is the target, then A
///   → bird animation + warp                         no input; wait for the map to change
/// ```
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: FlyState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    // `wCurMap`, not `game_state()`: this runs every tick of the flight and the whole of that
    // decode — party, map, bag, PC box, both dex bitfields — would be thrown away except for the
    // map id.
    let on_target = api.mmu().read_pointer(&pokered_symbols::wCurMap) == s.to as u8;
    // How the fly screen is recognised — see the "Why the town map is not driven like a menu"
    // note.
    let cursor_on = api.mmu().read_pointer(&pokered_symbols::wTownMapCoords);
    let font_loaded = api.mmu().pokemon_font_loaded();
    let s = FlyState { saw_font: s.saw_font || font_loaded, ..s };
    let town_map_open = s.chose_fly_at.is_some()
        && s.saw_font
        && !font_loaded
        && game_mode != GameMode::Overworld
        && is_town_coordinate(api.mmu(), cursor_on);
    // The flight is committed from the moment the town map accepts a destination until the bird
    // animation finishes, and the two flags between them cover that whole window.
    let in_flight = api.mmu().read_pointer(&pokered_symbols::wStatusFlags6) & BIT_FLY_WARP != 0
        || api.mmu().read_pointer(&pokered_symbols::wStatusFlags7) & BIT_USED_FLY != 0;

    let abort = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, why: String| {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("Fly: {why}") });
        agent.set_state(AgentState::Idle);
    };

    // ── Landed
    // ────────────────────────────────────────────────────────────────────────────────────
    if s.entered_menu && on_target && !in_flight && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("Flew to {}", s.to) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // The budget is checked before anything that waits, including the in-flight branch below: a
    // state restored mid-animation has `BIT_USED_FLY` still set with no flight in progress to
    // clear it, and a wait with no ceiling in front of it never returns.
    if s.ticks > TICK_BUDGET {
        abort(agent, api, format!("no progress in {TICK_BUDGET} ticks (still not on {})", s.to));
        return Ok(());
    }

    // Every remaining path either waits or presses one button, and costs exactly one tick.
    let mut s = FlyState { ticks: s.ticks + 1, ..s };
    let wait = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: FlyState| {
        api.release_all_buttons();
        agent.set_state(AgentState::Flying(FlyState { press: true, ..s }));
    };

    // ── In the air: the animation and the warp own the next second or so; keep hands off
    // ─────────
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

    // ── The town map.
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

    // ── Back in the overworld on the wrong map: the attempt fizzled (a mis-navigated menu, or
    // "Can't use FLY here!").
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }
    s.entered_menu |= game_mode != GameMode::Overworld;

    // ── FLY has been chosen and the town map is loading: hands off the joypad
    // ─────────────────────
    if let Some(chosen_at) = s.chose_fly_at {
        if s.ticks.saturating_sub(chosen_at) < TOWN_MAP_LOAD_TICKS {
            wait(agent, api, s);
            return Ok(());
        }
        // Long enough that the selection cannot still be loading, and the town map is not up
        // (that is checked above) — so the A press was swallowed by a field-move menu that was
        // still being drawn, which happens routinely.
        s.chose_fly_at = None;
    }

    if !s.press {
        wait(agent, api, s);
        return Ok(());
    }

    // ── The menu chain: START → POKéMON → the Fly mon → FLY, shared with the other field-move
    // drivers (`crate::pokemon::agent::field_move_menu_button`).
    let (slot, move_index) = fly_menu_indices(api);
    let button = if game_mode == GameMode::Overworld {
        JoypadButton::Start
    } else {
        crate::pokemon::agent::field_move_menu_button(api, slot, move_index)
    };
    // Pressing A on the field-move box *is* choosing FLY — the last input until the town map is
    // up.
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

    /// B1 — the Bike Voucher, from `postgame-phase0.bin` (the Viridian Pokémon Center).
    pub fn bike_voucher_steps() -> Vec<Self> {
        let mut s = Self::viridian_to_vermilion();
        s.push(Self::enter(Map::PokemonFanClub));
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::POKEMONFANCLUB_CHAIRMAN), 3));
        s
    }

    /// B2 — the Bicycle, from `postgame-bike-voucher.bin` (inside the Pokémon Fan Club).
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

    /// B3 — HM02 Fly, from `postgame-bicycle.bin` (inside the Cerulean Bike Shop).
    pub fn hm02_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::CeruleanCity),           // out of the Bike Shop, into the main terrace
            Self::enter(Map::CeruleanTrashedHouse),   // front door (27,11)
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door → the Route-5 pocket
            Self::enter(Map::Route5),
            Self::enter(Map::Route5Gate),             // north door (9/10,29)
            Self::enter_at(Map::Route5, 10, 33),      // south door — `BIT_GAVE_SAFFRON_GUARDS_DRINK` is set
            Self::enter(Map::SaffronCity),
            // Saffron → Celadon must cross at Route 7 (19,10): the *plain* connection lands in a
            // ledge-sealed pocket at (20,2) with no path to the gate
            // (`eevee_vaporeon_surf_steps`).
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

    /// B4 + B5 — teach Fly to the one compatible party member, then fly out of Route 16.
    pub fn teach_and_use_fly_steps(to: Map) -> Vec<Self> {
        vec![
            Self::TeachMove { item: crate::pokemon::item::ItemId::Hm02Fly,
                             target: crate::pokemon::policy::PartyRef::Slot(1) },
            Self::enter(Map::Route16), // outside, so the town map will open
            Self::Fly { to },
        ]
    }

    /// B7 + B6 — wake the Route 16 Snorlax, then ride Cycling Road to Fuchsia.
    pub fn cycling_road_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeladonCity },
            // Heal before the ride.
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
            // Route 18's top edge is water on both flanks — the connection strip reads
            // `~~~~~CCCCCCCC~~~~~~`, i.e. `ConnectionWater` at x=1–5 and x=14–19 — and a plain
            // `enter(Route18)` picks one of those, at which point the agent stops on the last dry
            // tile and tries to mount Surf on Cycling Road for ever.
            Self::enter_at(Map::Route18, 13, 0),
            // Route 18 has a gate too, and unlike Route 16's it is a plain east-west corridor:
            // west doors at (33,8)/(33,9) — also Route 18's force-bike tiles — and east doors at
            // (40,8)/(40,9), beyond which the Fuchsia connection sits
            // (`data/maps/objects/Route18.asm:9-13`).
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

    /// The town-map coordinate table the Fly driver steers by, read straight out of ROM bank
    /// `$1c`.
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

        // Every town is distinct, which is what makes the comparison in `tick` unambiguous.
        let all: Vec<u8> = EXPECTED.iter().map(|&(m, ..)| town_map_coords(&mmu, m)).collect();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "two towns share a town-map coordinate: {all:02x?}");
    }

    /// `wTownVisitedFlag` decodes to the towns Fly can reach. A fresh ROM has visited nothing,
    /// which is also the case the driver's pre-flight guard exists for.
    #[test]
    fn visited_towns_reads_the_bitfield() {
        let mut mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
        assert!(visited_towns(&mmu).is_empty(), "a fresh ROM has visited no towns");

        // Bit n = map id n, little-endian across the two bytes — so bit 10 (Saffron) lives in the
        // second one, which is what a byte-at-a-time reader would miss.
        mmu.write(pokered_symbols::wTownVisitedFlag.address, 0b0000_0101);
        mmu.write(pokered_symbols::wTownVisitedFlag.address + 1, 0b0000_0100);
        assert_eq!(visited_towns(&mmu), vec![Map::PalletTown, Map::PewterCity, Map::SaffronCity]);
    }
}
