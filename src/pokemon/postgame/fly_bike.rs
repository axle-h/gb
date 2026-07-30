//! Workstream **B — Fly, the Bicycle, and Cycling Road**. See `docs/postgame-coverage-plan.md` §6-B.
//!
//! The biggest quality-of-life win in the plan: Fly collapses cross-Kanto travel, which every other
//! workstream otherwise pays for in emulated minutes.
//!
//! Sub-steps: B1 Bike Voucher · B2 Bicycle · B3 HM02 · B4 teach Fly · B5 the Fly driver (the town map
//! is a bespoke screen, not a `HandleMenuInput` list) · B6 Cycling Road · B7 Route 16 Snorlax, then
//! `postgame-fly-bike.bin`.

use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::agent::{AgentEvent, AgentState, PokemonAgent};
use crate::pokemon::encoding::{GameMode, PokemonEncoding};
use crate::pokemon::font::FontAware;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::policy::PolicyStep;
use crate::pokemon::{PokemonApi, PokemonApiTrait};

/// The maps Fly can reach: map ids `0..NUM_CITY_MAPS` — Pallet, Viridian, Pewter, Cerulean, Lavender,
/// Vermilion, Celadon, Fuchsia, Cinnabar, Indigo Plateau, Saffron, in that (map-id) order, which is
/// also the order the town-map cursor walks through them (`BuildFlyLocationsList`).
pub(crate) const FLY_DESTINATIONS: u8 = 11;

/// `wStatusFlags6` bit 3 — set when the town map accepts a destination, cleared by the overworld loop
/// as it performs the warp (`constants/ram_constants.asm:119`, `home/overworld.asm:25-27`).
const BIT_FLY_WARP: u8 = 1 << 3;

/// `wStatusFlags7` bit 7 — set alongside `BIT_FLY_WARP` and cleared by the bird animation on arrival
/// (`engine/overworld/player_animations.asm:9-10`). Together the two cover the whole flight.
const BIT_USED_FLY: u8 = 1 << 7;

/// Tilesets `CheckIfInOutsideMap` accepts: `OVERWORLD` (towns and routes) and `PLATEAU` (Route 23 /
/// Indigo Plateau). Fly refuses to open the town map anywhere else — see [`FlyState::blocked_by`].
const OUTSIDE_TILESETS: [u8; 2] = [0, 23];

/// Live state of an in-progress flight. Carried in [`AgentState::Flying`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlyState {
    /// Where we are flying to. One of the 11 towns; anything else is refused up front.
    pub to: Map,
    /// Press/release alternation, so every input is a fresh rising edge.
    press: bool,
    /// Set once we have left the overworld, i.e. the menu chain has started.
    entered_menu: bool,
    /// The tick FLY was chosen from the field-move menu, after which **the driver presses nothing**
    /// until the town map is on screen.
    ///
    /// This is the single most important thing in this driver. Choosing FLY does not change any menu
    /// state the geometry tests can see — `wTopMenuItemY` still says field-move menu while
    /// `LoadTownMap_Fly` spends a second loading the map graphics — so a driver that keeps driving
    /// "the field-move menu" keeps pressing A, and the first of those A presses that survives into the
    /// fly screen's input loop confirms whatever the cursor starts on: **Pallet Town**, every time.
    /// If the town map has not appeared within [`TOWN_MAP_LOAD_TICKS`] the selection evidently did not
    /// take, so this is cleared and the menu chain is driven again.
    chose_fly_at: Option<u16>,
    /// Set once the font has been seen *loaded* during this flight's menu chain.
    ///
    /// The town map is recognised by its clobbered font (see [`tick`]), and the previous flight leaves
    /// the font clobbered until some menu redraws it — so on a save captured just after a landing the
    /// overworld itself looks like a town map. Every menu in the chain reloads the font, so requiring
    /// "loaded, then clobbered" separates a real fly screen from that leftover.
    saw_font: bool,
    /// Ticks spent driving, so a wedge reports itself instead of pulsing buttons for the whole budget.
    ticks: u16,
}

/// How long `LoadTownMap_Fly` may take to put the town map on screen, in agent ticks (20 ms each). It
/// disables the LCD, copies the world-map graphics and runs several palette fades, so this is a second
/// of emulated time with a wide margin — see [`FlyState::chose_fly_at`].
const TOWN_MAP_LOAD_TICKS: u16 = 150;

/// Ceiling on driver ticks for one flight. A flight is ~120 ticks (the START/party menus, up to ten
/// town-map cursor moves at 15 frames of redraw each, then the bird animation).
const TICK_BUDGET: u16 = 1200;

impl FlyState {
    pub fn new(to: Map) -> Self {
        Self { to, press: true, entered_menu: false, chose_fly_at: None, saw_font: false, ticks: 0 }
    }

    /// Why this flight cannot start, if it cannot — checked **before** any menu is opened, because
    /// every one of these fails in a way the driver cannot see: a mon with no Fly gets a field-move
    /// menu without a FLY entry, an indoor map gets "Can't use FLY here!" and drops back to the party
    /// list, and an unvisited town is simply **absent from the cursor's list**
    /// (`BuildFlyLocationsList` writes `NOT_VISITED` and the cursor skips it), so the driver would
    /// press Up round the list for ever looking for it.
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

/// The towns Fly can currently reach, read from `wTownVisitedFlag` (a 16-bit little-endian bitfield,
/// bit *n* = map id *n*). This is exactly the set the town-map cursor will stop on.
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

/// `(party slot, FLY's row in that mon's field-move box)`, both from **one** party read: this is on the
/// per-tick path, and a `GameState` would decode the map, the bag and the PC box along the way. Falls
/// back to `(0, 0)`, which no legitimate flight reaches — [`FlyState::blocked_by`] has already refused a
/// party with no Fly in it.
fn fly_menu_indices(api: &PokemonApi<'_>) -> (u8, u8) {
    let Ok(party) = api.mmu().read_player_pokemon_party() else { return (0, 0) };
    let flyer = party.iter().position(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Fly));
    match flyer {
        Some(slot) => (slot as u8, crate::pokemon::policy::field_move_index_of(&party[slot], PokemonMoveName::Fly)),
        None => (0, 0),
    }
}

/// Packed town-map coordinate of `map`: `ExternalMapEntries + 3 * map_id`, low nibble x, high nibble y
/// (`data/maps/town_map_entries.asm`, `LoadTownMapEntry` at `engine/items/town_map.asm:559-587`).
///
/// This is how the driver knows which town the cursor is on: the fly screen keeps its cursor in a
/// register, not in RAM, but every redraw calls `DrawPlayerOrBirdSprite`, which leaves the highlighted
/// town's packed coordinate in `wTownMapCoords`. All 11 towns have distinct coordinates, so the
/// comparison is exact.
fn town_map_coords(mmu: &MMU, map: Map) -> u8 {
    mmu.read_pointer(&(pokered_symbols::ExternalMapEntries + map as u16 * 3))
}

/// Whether `packed` is one of the eleven towns' town-map coordinates.
///
/// Used to sanity-check `wTownMapCoords` before trusting it: that byte is unioned with the party
/// menu's `wHPBarMaxHP` (`ram/wram.asm:953-965`), so outside the town map it holds a Pokémon's max HP.
fn is_town_coordinate(mmu: &MMU, packed: u8) -> bool {
    (0..FLY_DESTINATIONS).filter_map(Map::from_repr).any(|m| town_map_coords(mmu, m) == packed)
}

/// One agent tick of the Fly driver. Called from `agent.rs` via a single delegating match arm.
///
/// # The chain
///
/// ```text
/// overworld (outside map only)                      START
///   → START menu                                    cursor → 1 (POKéMON), A
///   → party menu                                    cursor → the Fly mon, A
///   → field-move menu                               cursor → FLY's index, A
///   → the town map                                  Up until wTownMapCoords is the target, then A
///   → bird animation + warp                         no input; wait for the map to change
/// ```
///
/// # Why the town map is not driven like a menu
///
/// §6-B5 warns that the town map is "a bespoke screen, not a `HandleMenuInput` list", and it is worse
/// than that: `LoadTownMap_Fly` holds the cursor in `hl` and never writes it to RAM, so
/// `wCurrentMenuItem` is whatever the *party* menu left behind. Three consequences shape this driver:
///
/// 1. **The screen is identified by its *broken font*.** Nothing else works. `wTownMapSpriteBlinkingEnabled`
///    looks like the flag for it and is a trap — it shares its byte with `wPartyMenuAnimMonEnabled`
///    (`ram/wram.asm:1444-1447`), which the party menu sets, so a driver keyed on it starts "driving
///    the town map" while still in the party list. Reading the screen does not work either:
///    `LoadTownMap_Fly` copies its up-arrow glyph to `vChars1 tile $6d`, which is **inside** the font
///    block at `vFont` ($8800, $80 tiles), so `pokemon_font_loaded()` is false and `on_screen_text`
///    returns `None` for the whole screen. That failure is deterministic and happens before the input
///    loop opens, so it *is* the signature: no font + not the overworld + `wTownMapCoords` holding a
///    real town coordinate means the fly screen, and nothing else does.
/// 2. **Only Up is pressed.** `.pressedUp` walks forward through `wFlyLocationsList`, skipping
///    unvisited towns and wrapping at the end, so a single button reaches every destination and the
///    walk is self-correcting: an overshoot just costs another lap.
/// 3. **Presses are dropped on purpose.** Each cursor move ends in `ld c, 15 / call DelayFrames`, a
///    quarter-second in which the joypad is not read at all. So the driver re-reads
///    `wTownMapCoords` every tick and decides afresh rather than counting presses — and because A is
///    idempotent while the cursor sits on the target, a swallowed A costs one tick, not the flight.
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: FlyState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    // `wCurMap`, not `game_state()`: this runs every tick of the flight and the whole of that decode —
    // party, map, bag, PC box, both dex bitfields — would be thrown away except for the map id.
    let on_target = api.mmu().read_pointer(&pokered_symbols::wCurMap) == s.to as u8;
    // How the fly screen is recognised — see the "Why the town map is not driven like a menu" note.
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

    // ── Landed ────────────────────────────────────────────────────────────────────────────────────
    if s.entered_menu && on_target && !in_flight && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("Flew to {}", s.to) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // The budget is checked before anything that waits, including the in-flight branch below: a state
    // restored mid-animation has `BIT_USED_FLY` still set with no flight in progress to clear it, and a
    // wait with no ceiling in front of it never returns.
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

    // ── In the air: the animation and the warp own the next second or so; keep hands off ─────────
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

    // ── The town map. Checked before every geometry test below, because it is not a menu and leaves
    //    the party menu's `wTopMenuItem*` in place, so each of those tests would misread it. ────────
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
    //    "Can't use FLY here!"). Drop to Idle so the policy re-issues it and the chain restarts. ──
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }
    s.entered_menu |= game_mode != GameMode::Overworld;

    // ── FLY has been chosen and the town map is loading: hands off the joypad ─────────────────────
    if let Some(chosen_at) = s.chose_fly_at {
        if s.ticks.saturating_sub(chosen_at) < TOWN_MAP_LOAD_TICKS {
            wait(agent, api, s);
            return Ok(());
        }
        // Long enough that the selection cannot still be loading, and the town map is not up (that is
        // checked above) — so the A press was swallowed by a field-move menu that was still being
        // drawn, which happens routinely. Choose FLY again.
        s.chose_fly_at = None;
    }

    if !s.press {
        wait(agent, api, s);
        return Ok(());
    }

    // ── The menu chain: START → POKéMON → the Fly mon → FLY, shared with the other field-move
    //    drivers (`crate::pokemon::agent::field_move_menu_button`). ─────────────────────────────────
    let (slot, move_index) = fly_menu_indices(api);
    let button = if game_mode == GameMode::Overworld {
        JoypadButton::Start
    } else {
        crate::pokemon::agent::field_move_menu_button(api, slot, move_index)
    };
    // Pressing A on the field-move box *is* choosing FLY — the last input until the town map is up.
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
    /// Viridian City → Vermilion City the short way: **Diglett's Cave**, not the Pewter/Mt Moon loop.
    ///
    /// This is `saffron_to_cinnabar_steps`' Route-2 crossing run backwards. Route 2's east side is
    /// walled by cuttable trees on *both* sides of `Route2Gate` — the gate's south door is Route 2
    /// (15,39) and its north door (16,35) — so the lead must know Cut: the `CuttingTree` executor
    /// always uses party **slot 0** and field-move index 0, so Venusaur is rotated to the front first.
    fn viridian_to_vermilion() -> Vec<Self> {
        vec![
            Self::enter(Map::ViridianCity),
            // Venusaur is the only Cut holder; the Cut executor only ever asks slot 0.
            Self::MovePokemonToFront { slot: 1 },
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

    /// **B1** — the **Bike Voucher**, from `postgame-phase0.bin` (the Viridian Pokémon Center).
    ///
    /// The chairman of the Vermilion **Pokémon Fan Club** hands it over after a YES/NO ("would you
    /// like to hear about my Rapidash?") — `YesNoChoice` opens with the cursor on YES, so the agent's
    /// generic A-mash answers it without a driver (`scripts/PokemonFanClub.asm:98-130`). Talking again
    /// afterwards is inert: `PokemonFanClub_CheckBikeInBag` short-circuits to a closing line once the
    /// voucher, or the Bicycle, is in the bag — so the `Interact` is repeated in case the first one is
    /// issued mid-script.
    pub fn bike_voucher_steps() -> Vec<Self> {
        let mut s = Self::viridian_to_vermilion();
        s.push(Self::enter(Map::PokemonFanClub));
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::POKEMONFANCLUB_CHAIRMAN), 3));
        s
    }

    /// **B2** — the **Bicycle**, from `postgame-bike-voucher.bin` (inside the Pokémon Fan Club).
    ///
    /// Vermilion → Cerulean is the Underground Path (`back_to_cerulean_steps`' crossing), then the
    /// Bike Shop at Cerulean (13,25). The clerk needs no menu once the voucher is held: `IsItemInBag`
    /// finds it, `GiveItem BICYCLE` runs, and the voucher is removed
    /// (`scripts/BikeShop.asm:10-31`) — so this is one `Interact`, not a `BuyFromMart`. Without the
    /// voucher the same NPC opens a BICYCLE/CANCEL menu at ¥1,000,000 instead, which is the branch this
    /// step must *not* land in.
    ///
    /// ⚠️ Cerulean is split by one-way ledges (see the `cerulean-route5-terraces` finding). Entering
    /// from Route 5 lands in the north-east pocket and the ledges drop *south* into the main terrace,
    /// which is where the Bike Shop is — so this direction is fine, and only the reverse needs the
    /// trashed-house bridge.
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

    /// **B3** — **HM02 Fly**, from `postgame-bicycle.bin` (inside the Cerulean Bike Shop).
    ///
    /// Cerulean → Saffron → Celadon → Route 16, then the girl in `Route16FlyHouse` hands HM02 over on
    /// the first talk (`scripts/Route16FlyHouse.asm:9-22`).
    ///
    /// Two crossings need spelling out:
    ///
    /// - **Out of Cerulean** the one-way ledges mean the Route-5 exit is only reachable through the
    ///   trashed house: front door (27,11) in the main terrace, back door → (27,9) in the Route-5
    ///   pocket (the `cerulean-route5-terraces` finding, and `cerulean_to_vermilion_steps`).
    /// - **Route16Gate1F is two corridors, not one.** Its west/east doors come in pairs at Route 16
    ///   y=10/11 (lower, the Cycling Road road) and y=4/5 (upper, where the Fly house is), and the
    ///   only way between them is the middle column past the guard — who blocks it unless the
    ///   **Bicycle** is in the bag (`scripts/Route16Gate1F.asm:16-46`, `.StopsPlayerCoords` (4,7)…(4,10)).
    ///   So B3 depends on B2, which §6-B does not say.
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
            // ledge-sealed pocket at (20,2) with no path to the gate (`eevee_vaporeon_surf_steps`).
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

    /// **B4 + B5** — teach Fly to the one compatible party member, then fly out of Route 16.
    ///
    /// Fly only works on an **outside** map (`CheckIfInOutsideMap`: tileset `OVERWORLD` or `PLATEAU`),
    /// so the step list has to leave the Fly house first — the driver refuses indoors rather than
    /// mashing at "Can't use FLY here!".
    pub fn teach_and_use_fly_steps(to: Map) -> Vec<Self> {
        vec![
            Self::TeachMove { item: crate::pokemon::item::ItemId::Hm02Fly, target_slot: 1 },
            Self::enter(Map::Route16), // outside, so the town map will open
            Self::Fly { to },
        ]
    }

    /// **B7 + B6** — wake the **Route 16 Snorlax**, then ride **Cycling Road** to Fuchsia.
    ///
    /// From anywhere flyable: Fly to Celadon, cross onto Route 16, wake the second Snorlax with the
    /// Poké Flute (`UseFieldItem` starts the lv30 wild battle, which the normal battle handler wins),
    /// then take the gate's **lower** corridor west. Stepping out at Route 16 (17,10) is what puts the
    /// player on the bike: `ForcedBikeOrSurfMaps` lists exactly (17,10) and (17,11) for Route 16
    /// (`data/maps/force_bike_surf.asm:7-8`), so nothing has to *use* the Bicycle — owning it is what
    /// gets you past the gate guard, and the tiles beyond mount it for you.
    ///
    /// Then it is one long downhill: Route 16 → 17 → 18 → Fuchsia, all connections, all southbound so
    /// the one-way ledges are with us. The ten Cycling Road bikers engage by line of sight en route.
    ///
    /// The two `CutTree`s bracket the Snorlax deliberately: the tree at Route 16 (34,9) walls off the
    /// road west of the Celadon entrance, and **the Snorlax battle reloads the map, so it grows back**
    /// (§10 of the plan). Each pops harmlessly if nothing is cuttable.
    pub fn cycling_road_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeladonCity },
            // Heal before the ride. Cycling Road is 144 tiles of one-way ledges with ten bikers on it
            // and no Pokémon Center anywhere on Routes 16/17/18, and the party arrives with the trek's
            // PP already spent — a lead down to its last Solarbeam turns every biker into a long fight.
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
            // ⚠️ Route 18's top edge is **water on both flanks** — the connection strip reads
            // `~~~~~CCCCCCCC~~~~~~`, i.e. `ConnectionWater` at x=1–5 and x=14–19 — and a plain
            // `enter(Route18)` picks one of those, at which point the agent stops on the last dry tile
            // and tries to mount Surf on Cycling Road for ever. Land on the dry middle explicitly.
            Self::enter_at(Map::Route18, 13, 0),
            // Route 18 has a gate too, and unlike Route 16's it is a plain east-west corridor: west
            // doors at (33,8)/(33,9) — also Route 18's force-bike tiles — and east doors at (40,8)/(40,9),
            // beyond which the Fuchsia connection sits (`data/maps/objects/Route18.asm:9-13`).
            Self::enter(Map::Route18Gate1F),
            Self::enter_at(Map::Route18, 40, 8),
            Self::enter(Map::FuchsiaCity),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ram::RAM;

    /// The town-map coordinate table the Fly driver steers by, read straight out of ROM bank `$1c`.
    ///
    /// This is the one fact the whole driver rests on — the fly screen keeps its cursor in a register,
    /// so `wTownMapCoords` is the only way to know which town is highlighted — and it is read through a
    /// banked ROM pointer, which is easy to get silently wrong: a mis-banked read returns *plausible*
    /// bytes, and the driver would then confirm the wrong town. The expected values are
    /// `data/maps/town_map_entries.asm`'s first eleven `external_map` rows.
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

    /// `wTownVisitedFlag` decodes to the towns Fly can reach. A fresh ROM has visited nothing, which is
    /// also the case the driver's pre-flight guard exists for.
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
