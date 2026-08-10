//! Workstream **E — Safari Zone proper**. See `docs/postgame-coverage-plan.md` §6-E.
//!
//! Before this, the Safari Zone was entered only to grab HM03 and the Gold Teeth, and
//! `pick_battle_action` hard-coded RUN on every Safari encounter — the whole catching half of the
//! mechanic was unexercised.
//!
//! Sub-steps: E1 model the 500-step budget · E2 replace the blanket RUN with a real catch policy ·
//! E3 catch a Safari-exclusive · E4 exit cleanly both ways, then `postgame-safari.bin`.
//!
//! # What a Safari trip is
//!
//! Paying the ¥500 at the gate (`scripts/SafariZoneGate.asm:176-195`) writes **30** into
//! `wNumSafariBalls` and **502** — not 500 — into `wSafariSteps`, sets `EVENT_IN_SAFARI_ZONE`, and
//! auto-walks the player north into `SafariZoneCenter`. Every overworld step inside the zone then runs
//! `SafariZoneCheckSteps`, which decrements the counter and, at zero (or when the last ball is spent),
//! warps the player back to the gate and ends the game
//! (`engine/events/hidden_events/safari_game.asm`). Both of those are the same code path, which is
//! why E4's "walk out" and "be ejected" cases converge on the same tile.
//!
//! # The battle is not a battle
//!
//! A Safari encounter has no FIGHT and no bag: the menu is BALL / BAIT / ROCK / RUN and the enemy
//! decides every turn whether to flee. So there is no weakening pass, no status, and the only levers
//! are the ball itself and the two throwables — whose arithmetic is in [`ball_catch_chance`] /
//! [`flee_chance`], and which turn out **not to be worth using**. See the module tests and §11.

use crate::geometry::Point8;
use crate::mmu::MMU;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::map::Map;
use crate::pokemon::policy::{DeterministicPolicy, PolicyStep};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::tile::MetaTile;
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::GameState;
use crate::ram::ROM;

// ── E1: the step budget ──────────────────────────────────────────────────────────────────────────

/// The live state of a Safari trip — `None` in [`GameState::safari`] when the player is not on the
/// clock, which is every map in the game bar the five Safari ones (and the gate, after ejection).
///
/// Both numbers are hard budgets the game enforces itself: at `steps_left == 0` **or**
/// `balls_left == 0` the trip ends and the player is warped to the gate. Without them in `GameState`
/// a hunt reports nothing until it is already over — the plan's E1 in one sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafariState {
    /// `wSafariSteps`, a **big-endian** word (`ld a, HIGH(502)` into the low address). Starts at 502.
    pub steps_left: u16,
    /// `wNumSafariBalls`. Starts at 30 and is decremented by `ItemUseBall`'s `.safariZone` branch.
    pub balls_left: u8,
    /// `EVENT_SAFARI_GAME_OVER` — set by `SafariZoneGameOver` the instant the budget runs out, and
    /// consumed by the gate's `CheckAndResetEvent` on arrival. True for the handful of ticks between
    /// the two, which is exactly the window E4's ejection test waits on.
    pub game_over: bool,
}

/// `wEventFlags` byte holding both Safari events: `EVENT_SAFARI_GAME_OVER` is $24E and
/// `EVENT_IN_SAFARI_ZONE` is $24F (`constants/event_constants.asm:151`, counting from the
/// `const_next $238` that opens the Fuchsia block), so both live in byte $24F / 8 = 73.
const SAFARI_EVENT_BYTE: u16 = 73;
/// $24F % 8 = 7.
const IN_SAFARI_ZONE: u8 = 1 << 7;
/// $24E % 8 = 6.
const SAFARI_GAME_OVER: u8 = 1 << 6;

/// Read [`SafariState`], or `None` when `EVENT_IN_SAFARI_ZONE` is clear.
///
/// The event — not the map — is the discriminator, because the two disagree exactly where it matters:
/// the ejection warp puts the player on `SafariZoneGate` with the event still set, and the gate script
/// only clears it a few ticks later when it prints "good haul". A map-based reader would report the
/// trip over before the game agrees.
pub fn read_state(mmu: &MMU) -> Option<SafariState> {
    let flags = mmu.read(pokered_symbols::wEventFlags.address + SAFARI_EVENT_BYTE);
    if flags & IN_SAFARI_ZONE == 0 {
        return None;
    }
    Some(SafariState {
        steps_left: mmu.read_pointer_u16_be(&pokered_symbols::wSafariSteps),
        balls_left: mmu.read_pointer(&pokered_symbols::wNumSafariBalls),
        game_over: flags & SAFARI_GAME_OVER != 0,
    })
}

// ── E2: the odds, and why BAIT and ROCK are never thrown ─────────────────────────────────────────

/// Rejection-sampling ceiling on `Rand1` for a Safari Ball — it shares the Ultra Ball's range
/// (`engine/items/item_effects.asm:200-208`), so the first check passes with probability
/// `(catch_rate + 1) / 151` rather than `/ 256`.
const SAFARI_BALL_RAND1_MAX: u16 = 150;

/// `BallFactor` for every ball but the Great Ball (`item_effects.asm:238-244`).
const SAFARI_BALL_FACTOR: u32 = 12;

/// Probability that **one** Safari Ball catches a target with `catch_rate`, at `current_hp` of
/// `max_hp`.
///
/// This is `ItemUseBall` with the Safari branch's constants substituted, and with the status term
/// dropped — a Safari target can never *be* statused, since the player has no moves. Two rolls:
///
/// 1. `Rand1 ∈ [0, 150]`; the ball fails outright if `Rand1 > catch_rate`.
/// 2. `X = min(255, ((MaxHP * 255) / 12) / max(HP / 4, 1))`; captured if `X > 255` (impossible at
///    BallFactor 12 — the ceiling is 255 exactly at 1 HP) or if `Rand2 ≤ X`.
///
/// At **full HP** the second roll collapses to a constant: `X = 255 * 4 / 12 = 85`, i.e. 86/256 ≈
/// 33.6 %, whatever the species. That constant is why the Safari's catch rates matter so much more
/// than its HP bars, and why there is no weakening pass to write here even in principle.
pub fn ball_catch_chance(catch_rate: u8, max_hp: u16, current_hp: u16) -> f64 {
    let first = (catch_rate as u16 + 1).min(SAFARI_BALL_RAND1_MAX + 1) as f64 / (SAFARI_BALL_RAND1_MAX + 1) as f64;
    let hp_term = (current_hp / 4).max(1) as u32;
    let x = ((max_hp as u32 * 255) / SAFARI_BALL_FACTOR) / hp_term;
    if x > 255 {
        return first;
    }
    first * (x + 1) as f64 / 256.0
}

/// Probability that the target **flees** at the end of a turn, given its live Speed *stat*.
///
/// `engine/battle/core.asm:181-207`: `b = (speed & 0xFF) * 2`, and the doubling's carry is not a
/// wrap — it is an immediate `jp c, EnemyRan`, so anything whose speed byte exceeds 127 flees on the
/// spot. Otherwise the enemy runs when `Random < b`, i.e. with probability `b / 256`.
///
/// Note this is the **stat**, not the base stat: a lv23 Chansey's 50 base speed computes to ~35, so
/// it flees about 27 % of the time rather than the ~39 % base speed would suggest.
pub fn flee_chance(enemy_speed: u16, bait_active: bool, rock_active: bool) -> f64 {
    let low = (enemy_speed & 0xFF) as u16;
    if low > 127 {
        return 1.0; // the carry out of `add a` — flees unconditionally
    }
    let mut b = low * 2;
    if bait_active {
        b >>= 2;
    }
    if rock_active {
        b = (b * 2).min(255);
    }
    b as f64 / 256.0
}

/// Chance of eventually catching a target that is thrown at every turn until it is caught or flees:
/// `p / (p + (1 - p) · f)`.
pub fn encounter_catch_chance(per_ball: f64, per_turn_flee: f64) -> f64 {
    let denominator = per_ball + (1.0 - per_ball) * per_turn_flee;
    if denominator <= 0.0 { 0.0 } else { per_ball / denominator }
}

/// The battle half of a [`PolicyStep::SafariHunt`]: throw a ball at anything still wanted, run from
/// everything else.
///
/// **Balls only — never BAIT, never ROCK.** The plan (§6-E2) reads them as a real trade-off ("Rock
/// raises catch rate *and* flee rate; Bait does the inverse"), and they are, but the arithmetic in
/// `engine/battle/safari_zone.asm` is lopsided in a way the description hides:
///
/// - Both effects **decay**: `PrintSafariZoneBattleText` decrements the factor once per turn *before*
///   the flee check, so a freshly-thrown Bait with the minimum roll of 1 protects for zero turns, and
///   the expectation is only ~2.
/// - Bait's catch-rate halving is **permanent for the encounter** — only the *escape* counter's
///   expiry restores the base rate, so the protection runs out and the penalty does not.
/// - Rock spends a whole turn buying a doubled catch rate that arrives alongside a doubled flee
///   chance, and the flee check runs immediately.
///
/// Worked through exactly — every branch of the ROM's turn, not a simulation — in
/// `bait_and_rock_are_never_worth_throwing`: a lv23 Chansey, the target with the most to gain from
/// either throwable, is **21.3 %** per encounter on balls alone against **13.1 %** baited first and
/// **11.9 %** rocked first. Encounters are cheap (30 balls and 502 steps buy dozens); the throwables
/// are not.
pub fn pick_battle_action(state: &GameState, targets: &[PokemonSpecies], actions: &[BattleAction])
    -> Option<BattleAction>
{
    let battle = state.battle.as_ref()?;
    if battle.battle_type != BattleType::Safari {
        return None;
    }
    let run = actions.iter().find(|a| matches!(a, BattleAction::Run)).cloned();
    let enemy = battle.enemy.species;

    if !targets.contains(&enemy) || state.pokedex_owned.contains(&enemy) {
        return run;
    }
    if state.safari.is_some_and(|s| s.balls_left == 0) {
        println!("[safari] {enemy} is wanted but the last ball is spent — running");
        return run;
    }
    let per_ball = ball_catch_chance(battle.enemy_catch_rate, battle.enemy.stats.hp, battle.enemy.current_hp);
    let per_flee = flee_chance(battle.enemy.stats.speed, false, false);
    println!("[safari] {enemy} lv{} — catch rate {}, {:.1}%/ball, {:.0}% flee → {:.0}%/encounter ({} balls left)",
        battle.enemy.level, battle.enemy_catch_rate, per_ball * 100.0, per_flee * 100.0,
        encounter_catch_chance(per_ball, per_flee) * 100.0,
        state.safari.map_or(0, |s| s.balls_left));

    actions.iter().find(|a| matches!(a, BattleAction::SafariBall)).cloned().or(run)
}

// ── E3: the hunt ─────────────────────────────────────────────────────────────────────────────────

/// What [`pick`] wants the policy to do this tick. Three outcomes because a hunt has three:
/// keep walking, wait for the map to settle, or the step is finished.
pub enum Hunt {
    /// Issue this overworld action (a walk toward the zone, or a pace through grass).
    Walk(OverworldAction),
    /// Nothing to do this tick — a script is running, or the tile grid has not settled yet.
    Wait,
    /// The step is over; the policy pops it.
    Done,
}

/// Trip bookkeeping for the current [`PolicyStep::SafariHunt`], held by `DeterministicPolicy`.
///
/// A trip is not observable from any single frame: `EVENT_IN_SAFARI_ZONE` is a level, not an edge, so
/// counting entries means remembering whether we were inside last tick. Reset when the step pops.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HuntProgress {
    /// Paid entries so far, including the one in progress.
    pub trips: u32,
    /// Whether `EVENT_IN_SAFARI_ZONE` was set last time [`pick`] ran.
    was_inside: bool,
    /// Consecutive ticks with no route to the hunting ground — see [`ROUTE_PATIENCE`].
    route_stuck: u32,
}

impl HuntProgress {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The ¥500 the gate charges per trip (`SafariZoneGate.asm:159-163`).
pub const ENTRY_FEE: u32 = 500;

/// One box is 20 Pokémon; `ItemUseBall` refuses outright ("the BOX is full!") when the party is full
/// **and** the open box is (`item_effects.asm:118-123`).
const MONS_PER_BOX: usize = 20;

/// Species in `targets` that are not owned yet — the only reason to still be in the zone.
pub fn wanted(state: &GameState, targets: &[PokemonSpecies]) -> Vec<PokemonSpecies> {
    targets.iter().copied().filter(|s| !state.pokedex_owned.contains(s)).collect()
}

/// The overworld half of a [`PolicyStep::SafariHunt`] — pay, pace, get ejected, pay again.
///
/// The shape is deliberately the same as `CatchPokemon`'s wander branch (walk to the nearest grass and
/// let `AgentState::PacingForEncounters` do the rest), with the Safari's three extra facts layered on:
///
/// - **Trips are re-entrant.** Running out of steps or balls is not a failure, it is the ordinary end
///   of a trip: the game warps the player to the gate, and walking back up pays another ¥500 for a
///   fresh 502/30. So being outside the zone is a routing problem, not a stop condition.
/// - **The stop conditions are all resource ones** — every target owned, the trip budget spent, the
///   wallet short of the fee, or nowhere left to put a catch.
/// - **`route_toward` only knows maps this run has walked**, so the caller's step list must have
///   entered the zone once explicitly ([`PolicyStep::safari_hunt_steps`]); after that the graph holds
///   the gate → centre edge and every re-entry can route itself.
pub fn pick(
    progress: &mut HuntProgress,
    state: &GameState,
    world_graph: &WorldGraph,
    actions: &[OverworldAction],
    targets: &[PokemonSpecies],
    map: Map,
    max_trips: u32,
) -> Hunt {
    // ⚠️ Ejection is not instantaneous, and the gap is a trap. `SafariZoneGameOver` warps the player
    // to the gate and sets `EVENT_SAFARI_GAME_OVER`, but leaves `EVENT_IN_SAFARI_ZONE` set until the
    // gate script's `CheckAndResetEvent` runs a few ticks later. In that window the player is standing
    // on the gate mat with the trip already over while `safari` still reads `Some` — and routing back
    // toward the zone there walks straight into the join prompt and pays another ¥500, blowing
    // `max_trips`. So an ejected trip counts as outside, and nothing is issued until the script lands.
    let ejected = state.safari.is_some_and(|s| s.game_over);
    if ejected {
        return Hunt::Wait;
    }
    let inside = state.safari.is_some();
    if inside && !progress.was_inside {
        progress.trips += 1;
        println!("[safari] trip {}/{max_trips} begins ({} steps, {} balls, ¥{})",
            progress.trips, state.safari.map_or(0, |s| s.steps_left),
            state.safari.map_or(0, |s| s.balls_left), state.money);
    }
    progress.was_inside = inside;

    let outstanding = wanted(state, targets);
    if outstanding.is_empty() {
        println!("[safari] every target owned after {} trip(s) — done", progress.trips);
        return Hunt::Done;
    }
    // Room for the next catch. A full party *and* a full open box makes `ItemUseBall` refuse with a
    // text box, which from the policy's side is indistinguishable from a miss — it would throw the
    // rest of the trip's balls at nothing.
    if state.pokemon.len() >= 6 && state.boxed_pokemon.len() >= MONS_PER_BOX {
        println!("[safari] party and box {} are both full — no room for a catch, stopping",
            state.current_box + 1);
        return Hunt::Done;
    }
    if !inside {
        // Between trips (or before the first one): the budget and the wallet decide whether there is
        // another. Both checks belong here rather than at the top, because inside the zone the fee is
        // already paid and a low balance is not a reason to walk out.
        if progress.trips >= max_trips {
            println!("[safari] {max_trips} trip(s) spent, still wanting {outstanding:?} — stopping");
            return Hunt::Done;
        }
        if state.money < ENTRY_FEE {
            println!("[safari] ¥{} is not the ¥{ENTRY_FEE} entry fee — stopping", state.money);
            return Hunt::Done;
        }
    }

    if state.map.map != map {
        // Walk (back) in. From the gate this crosses the join prompt, which `YesNoChoice` opens on
        // YES, so the agent's generic A-mash pays without a driver — the same shape as B1's chairman.
        return match step_toward(world_graph, actions, state.map.map, map) {
            Some(action) => { progress.route_stuck = 0; Hunt::Walk(action) }
            // ⚠️ **Not a failure yet.** For several ticks after a warp — and after every battle — the
            // tile grid and the sprite list are still settling and `actions()` is short or empty, so a
            // route that exists reads as absent. Giving up on the first miss cost a sweep four species
            // (`no route from SafariZoneCenter to SafariZoneEast`, on a map whose east warp is 30 steps
            // away). Same bound and same reasoning as `CatchPokemon`'s `catch_wander_stuck`.
            None if progress.route_stuck < ROUTE_PATIENCE => {
                progress.route_stuck += 1;
                Hunt::Wait
            }
            None => {
                println!("[safari] no route from {} to {map} in {ROUTE_PATIENCE} ticks — giving up",
                    state.map.map);
                Hunt::Done
            }
        };
    }

    match actions.iter().find(|a| a.tile == MetaTile::Grass) {
        Some(action) => { progress.route_stuck = 0; Hunt::Walk(action.clone()) }
        // ⚠️ **A wait with no grass has to be bounded, and for a reason that is easy to miss.** For the
        // first few ticks after a warp this is just the tile grid settling. But if the area we landed
        // in has no *reachable* grass — the west area's two warp pairs land on shelves that one-way
        // ledges seal off from each other — then waiting is not patience, it is a hang: the trip's step
        // counter only moves when the player *walks*, so the budget that bounds every other case never
        // runs down, and the leg burns its whole cycle cap standing still. Give up on the area instead
        // and let the sweep carry on to the next one.
        None if progress.route_stuck < ROUTE_PATIENCE => {
            progress.route_stuck += 1;
            Hunt::Wait
        }
        None => {
            println!("[safari] no reachable grass on {map} at {} — giving up on this area",
                state.map.player_position);
            Hunt::Done
        }
    }
}

/// Ticks (20 ms each, so ~8 s of game time) a hunt waits for a route to the hunting ground before
/// concluding there is not one. Matches `DeterministicPolicy::catch_wander_stuck`'s bound, which
/// exists for the same reason.
const ROUTE_PATIENCE: u32 = 400;

/// The best area to hunt each of the twelve species the zone adds to this save's dex, with its
/// encounter-slot share there.
///
/// Every species below appears in two or three of the four areas, and *which* one matters more than
/// anything else in this workstream: the slot table
/// (`data/wild/probabilities.asm` — 19.9 / 19.9 / 15.2 / 9.8 / 9.8 / 9.8 / 5.1 / 5.1 / 4.3 / 1.2 %)
/// puts the same species in slot 8 on one map and slot 9 on another, which is a **3.6×** difference in
/// how long it takes to meet one. Chansey is 4.3 % in the north and 1.2 % in the centre; Tauros 4.3 %
/// in the west and 1.2 % in the north; Kangaskhan 4.3 % in the east and 1.2 % in the west. Hunting
/// each where it is common is the difference between this sweep costing minutes and costing an hour.
pub mod grounds {
    use super::*;

    /// Centre — Rhyhorn 19.9 %, Exeggcute 9.8+9.8 %, Nidorino 9.8 %, Nidorina 5.1 %, Parasect 5.1 %,
    /// **Scyther 4.3 %** (1.2 % in the east, its only other home).
    pub const CENTRE: &[PokemonSpecies] = &[
        PokemonSpecies::Rhyhorn, PokemonSpecies::Exeggcute, PokemonSpecies::Nidorino,
        PokemonSpecies::Nidorina, PokemonSpecies::Parasect, PokemonSpecies::Scyther,
    ];
    /// East — Doduo 19.9 %, **Kangaskhan 4.3 %** (1.2 % in the west).
    pub const EAST: &[PokemonSpecies] = &[PokemonSpecies::Doduo, PokemonSpecies::Kangaskhan];
    /// North — Paras 15.2 %, Venomoth 5.1 %, **Chansey 4.3 %** (1.2 % in the centre).
    pub const NORTH: &[PokemonSpecies] = &[
        PokemonSpecies::Paras, PokemonSpecies::Venomoth, PokemonSpecies::Chansey,
    ];
    /// West — **Tauros 4.3 %** (1.2 % in the north). Everything else the west offers is caught by then.
    pub const WEST: &[PokemonSpecies] = &[PokemonSpecies::Tauros];
}

/// The zone's land topology: a **chain**, not a hub.
///
/// The centre looks like a hub — its action list offers all three areas — but only while the BFS is
/// allowed to swim. On foot the pond cuts it in two, and from the entrance side only the **east** warp
/// is reachable (measured: `probe_safari_centre_from_the_entrance` with `can_surf` false lists the
/// gate, east and the rest house, and neither north nor west). That is the same conclusion the
/// pre-existing `safari_zone_surf_steps` reached from the other direction — *"the item-bearing West
/// area is reached the long way round: Center → East → North → West (the only land route)"*.
const LAND_CHAIN: [Map; 5] = [
    Map::SafariZoneGate, Map::SafariZoneCenter, Map::SafariZoneEast,
    Map::SafariZoneNorth, Map::SafariZoneWest,
];

/// North → West has **four** warps in two pairs, and which one a leg wants depends on what it is for.
///
/// `safari_zone_surf_steps` pins the *western* pair (landing (20,0)/(21,0)) because that is the Gold
/// Teeth and Secret House plateau. A hunt wants the **eastern** pair, landing (26,0)/(27,0), for the
/// exact reason that leg avoided it: one-way ledges seal the two shelves off from each other, and all
/// of the west's **grass is on the eastern one**. Measured by `probe_safari_areas`, which reports
/// `grass: None` from (21,0) and `grass: Some(((6,20), 44 steps))` from (26,0) — a Tauros hunt on the
/// plateau would stand still on a bare shelf for its whole budget. The eastern shelf also has its own
/// warps straight back to the centre, which the plateau does not.
const WEST_LANDING: Point8 = Point8 { x: 26, y: 0 };

/// One hop toward `map` from wherever the player is standing.
///
/// ⚠️ **`route_toward` alone is not enough here, and the reason is the ejection.** The incremental
/// world graph is keyed by *(map, entry position)* and its nodes are the ones the agent has *walked*
/// into. Being ejected at 0 steps is a **warp** onto the gate's third mat — a node no walk ever
/// created — so from there the graph offers no path anywhere, and a hunt that had been ejected
/// mid-sweep gave up with "no route from SafariZoneGate to SafariZoneEast" even though the two are two
/// warps apart and both had been walked minutes earlier. Measured: the first full sweep lost four
/// species that way.
///
/// [`LAND_CHAIN`] does not need the graph. It says which map is *next*, and the crossing to an
/// adjacent map is always in the current map's own action list.
fn step_toward(world_graph: &WorldGraph, actions: &[OverworldAction], from: Map, to: Map)
    -> Option<OverworldAction>
{
    let crossing_to = |target: Map, landing: Option<Point8>| {
        let matches_landing = move |a: &&OverworldAction| match a.tile {
            MetaTile::Warp { to_map, to_position } =>
                to_map == target && landing.is_none_or(|l| to_position == l),
            MetaTile::Connection { to_map, .. } => to_map == target && landing.is_none(),
            _ => false,
        };
        // The pinned landing first, then any crossing to the same map — a hunt that cannot reach the
        // preferred warp should still get *somewhere* rather than stand still.
        actions.iter().find(matches_landing).or_else(|| actions.iter().find(|a| match a.tile {
            MetaTile::Warp { to_map, .. } | MetaTile::Connection { to_map, .. } => to_map == target,
            _ => false,
        })).cloned()
    };
    // The next map along the chain, which for an adjacent target is the target itself.
    let next = match (LAND_CHAIN.iter().position(|&m| m == from), LAND_CHAIN.iter().position(|&m| m == to)) {
        (Some(i), Some(j)) if i < j => LAND_CHAIN[i + 1],
        (Some(i), Some(j)) if i > j => LAND_CHAIN[i - 1],
        _ => to,
    };
    let landing = (next == Map::SafariZoneWest).then_some(WEST_LANDING);
    crossing_to(next, landing)
        .or_else(|| DeterministicPolicy::route_toward(world_graph, actions, to))
}

/// The overworld half of [`PolicyStep::SafariExit`] — walk out of the zone from wherever a hunt left
/// us, and pop once we are standing on the gate mat.
///
/// A plain `enter(SafariZoneGate)` cannot do this job, and the west is why: it is four warps from the
/// gate, `enter` only knows crossings on the *current* map plus the world graph, and the graph has no
/// usable node after an ejection. Worse, the west's own shortcut warp back to the centre lands at
/// (0,10) — the far side of the pond, a region that on foot **cannot reach the gate at all** (the
/// entrance probe's action list is the proof: it never mentions the west warps). So the way out is the
/// way in, reversed, along [`LAND_CHAIN`] — which is exactly what the pre-existing
/// `safari_zone_strength_steps` does by hand.
pub fn exit(progress: &mut HuntProgress, state: &GameState, world_graph: &WorldGraph,
            actions: &[OverworldAction]) -> Hunt
{
    if !LAND_CHAIN.contains(&state.map.map) || state.map.map == Map::SafariZoneGate {
        return Hunt::Done; // on the mat, or already outside — either way the zone is behind us
    }
    match step_toward(world_graph, actions, state.map.map, Map::SafariZoneGate) {
        Some(action) => { progress.route_stuck = 0; Hunt::Walk(action) }
        None if progress.route_stuck < ROUTE_PATIENCE => { progress.route_stuck += 1; Hunt::Wait }
        None => {
            println!("[safari] no way out of {} — giving up", state.map.map);
            Hunt::Done
        }
    }
}

impl PolicyStep {
    /// **E3/E4** — hunt `targets` in `SafariZoneCenter`, then walk out through the gate.
    ///
    /// The centre is where the entrance auto-walk drops the player, so this is the cheapest possible
    /// hunt: no walking beyond the pacing itself.
    ///
    /// The `enter` steps at the front are load-bearing: the world graph is built as the agent walks,
    /// so the first entry cannot be routed, only scripted. Everything after that — including re-entry
    /// after an ejection — [`pick`] routes for itself.
    ///
    /// The two at the back are E4's *deliberate* exit: from inside, the gate answers with "leaving
    /// early?" (`YesNoChoice`, opens on YES); after an ejection the player is already standing there
    /// and the step pops on arrival. Both paths end outdoors in Fuchsia, which is what the next leg's
    /// `Fly` needs.
    pub fn safari_hunt_steps(targets: &'static [PokemonSpecies], max_trips: u32) -> Vec<Self> {
        let mut steps = vec![
            Self::Fly { to: Map::FuchsiaCity },
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::SafariZoneCenter), // pays ¥500 via the join prompt, auto-walks in
            Self::SafariHunt { targets, map: Map::SafariZoneCenter, max_trips },
        ];
        steps.extend(Self::safari_exit_steps());
        steps
    }

    /// **E3** — sweep all four areas, hunting each species where its encounter slot is fattest
    /// (see [`grounds`]). `max_trips` bounds the ¥500 entries **per area**, not for the sweep.
    ///
    /// All twelve species the Safari Zone adds to this save's dex, which is what makes E worth taking
    /// past its own §6: it is the cheapest route to the **30 owned** the Itemfinder wants (H3), and
    /// the only source of Chansey, Scyther, Kangaskhan and Tauros on this cartridge.
    ///
    /// Every area is one warp from the centre — the centre's own action list offers all three
    /// (30 / 37 / 41 steps) — so each hop is `enter(centre)` then `enter(area)`. The first of those
    /// looks redundant and is not: if the previous hunt ended by *ejection* the player is standing at
    /// the gate, and `enter(area)` from there has no route at all (the area is not adjacent, and the
    /// world graph is only built as the agent walks). From the gate `enter(centre)` pays a fresh ¥500
    /// and walks in; from inside the centre it pops on arrival. One step, both cases.
    pub fn safari_sweep_steps(max_trips: u32) -> Vec<Self> {
        let mut steps = vec![
            Self::Fly { to: Map::FuchsiaCity },
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::SafariZoneCenter),
            Self::SafariHunt { targets: grounds::CENTRE, map: Map::SafariZoneCenter, max_trips },
        ];
        for (area, targets) in [
            (Map::SafariZoneEast,  grounds::EAST),
            (Map::SafariZoneNorth, grounds::NORTH),
            (Map::SafariZoneWest,  grounds::WEST),
        ] {
            // No `enter` steps between hunts: the hunt walks itself in along [`LAND_CHAIN`], which is
            // the *only* thing that works from both places a hunt can end — deep inside the previous
            // area, or standing on the gate mat after an ejection.
            steps.push(Self::SafariHunt { targets, map: area, max_trips });
        }
        steps.extend(Self::safari_exit_steps());
        steps
    }

    /// Out of the zone and back onto an outdoor Fuchsia tile, from wherever the last hunt ended —
    /// deep in an area, or standing on the gate mat after an ejection.
    ///
    /// [`SafariExit`](Self::SafariExit) rather than `enter(gate)` because the two endings need
    /// different journeys and only one step can be in the list: see [`exit`].
    fn safari_exit_steps() -> Vec<Self> {
        vec![Self::SafariExit, Self::enter(Map::FuchsiaCity)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Safari Ball's own constants, pinned: the `[0,150]` rejection range and BallFactor 12 are
    /// what make a full-HP throw a flat 33.6 % on its second roll, and getting either wrong would
    /// silently change every number this module prints.
    #[test]
    fn a_full_hp_throw_collapses_to_the_ball_range() {
        // Second roll at full HP: X = ((MaxHP*255)/12) / (MaxHP/4) = 85 → 86/256.
        let second = 86.0 / 256.0;
        // Chansey, catch rate 30 (`data/pokemon/base_stats/chansey.asm`).
        assert!((ball_catch_chance(30, 200, 200) - (31.0 / 151.0) * second).abs() < 1e-6);
        // Exeggcute, catch rate 90 — the same second roll, a much better first.
        assert!((ball_catch_chance(90, 200, 200) - (91.0 / 151.0) * second).abs() < 1e-6);
        // A catch rate above the rejection ceiling cannot fail the *first* roll at all.
        assert!((ball_catch_chance(255, 200, 200) - second).abs() < 1e-6);
        // Weakening does move the second roll — it is just never available in the Safari.
        assert!(ball_catch_chance(30, 200, 20) > ball_catch_chance(30, 200, 200));
    }

    /// `add a` on the speed byte is a *carry*, not a wrap: over 127 and the target is gone whatever
    /// else happens. Worth pinning because it is the one branch no policy can play around.
    #[test]
    fn a_fast_enough_target_always_flees() {
        assert_eq!(flee_chance(128, false, false), 1.0);
        assert_eq!(flee_chance(127, false, false), 254.0 / 256.0);
        // Bait quarters the roll, Rock doubles it (capped at 255/256).
        assert_eq!(flee_chance(40, true, false), 20.0 / 256.0);
        assert_eq!(flee_chance(40, false, true), 160.0 / 256.0);
    }

    /// The turn's three openings, in the order `DisplayBattleMenu` offers them.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Opening { Balls, BaitFirst, RockFirst }

    /// Exact probability of catching a full-HP target under `opening`, over a `turns`-turn horizon.
    ///
    /// Not a simulation — an expectation, evaluated over every branch of the ROM's turn:
    ///
    /// 1. the throw resolves (`ItemUseBall` / `ItemUseBait` / `ItemUseRock`), where BAIT halves the
    ///    live catch rate and BOTH throwables roll their counter uniformly on 1..=5;
    /// 2. `PrintSafariZoneBattleText` decrements whichever counter is live — **before** the flee
    ///    check, which is why a counter that rolls 1 protects for exactly zero turns — and restores
    ///    the base catch rate if it was the *escape* counter that just hit zero;
    /// 3. the enemy flees with the probability [`flee_chance`] gives for the decremented counters.
    ///
    /// The asymmetry the plan's one-line description hides is in step 2: BAIT's halving is never
    /// restored (only the escape branch reloads `wMonHCatchRate`), so its protection expires and its
    /// penalty does not.
    fn catch_probability(opening: Opening, base_rate: u8, max_hp: u16, speed: u16, turns: u32) -> f64 {
        fn go(opening: Opening, base: u8, max_hp: u16, speed: u16, turns: u32,
              rate: u8, bait: u8, escape: u8, turn: u32) -> f64
        {
            if turn >= turns {
                return 0.0;
            }
            let throwable = match (opening, turn) {
                (Opening::BaitFirst, 0) | (Opening::RockFirst, 0) => opening,
                _ => Opening::Balls,
            };
            // Average over the counter's 1..=5 roll for the two throwables.
            let rolled = |f: &dyn Fn(u8) -> f64| -> f64 { (1..=5).map(|r| f(r)).sum::<f64>() / 5.0 };
            match throwable {
                Opening::Balls => {
                    let p = ball_catch_chance(rate, max_hp, max_hp);
                    // `PrintSafariZoneBattleText` on a turn with no fresh throw: decay whichever
                    // counter is live, restoring the base rate when the escape counter expires.
                    let (rate, bait, escape) = if bait > 0 {
                        (rate, bait - 1, escape)
                    } else if escape > 0 {
                        (if escape == 1 { base } else { rate }, bait, escape - 1)
                    } else {
                        (rate, bait, escape)
                    };
                    let fled = flee_chance(speed, bait > 0, escape > 0);
                    p + (1.0 - p) * (1.0 - fled)
                        * go(opening, base, max_hp, speed, turns, rate, bait, escape, turn + 1)
                }
                Opening::BaitFirst => rolled(&|r| {
                    let (rate, bait) = (rate / 2, r - 1); // halve, roll, then the immediate decrement
                    let fled = flee_chance(speed, bait > 0, false);
                    (1.0 - fled) * go(opening, base, max_hp, speed, turns, rate, bait, 0, turn + 1)
                }),
                Opening::RockFirst => rolled(&|r| {
                    let doubled = (rate as u16 * 2).min(255) as u8;
                    let escape = r - 1;
                    let rate = if escape == 0 { base } else { doubled }; // expired counters reload the base rate
                    let fled = flee_chance(speed, false, escape > 0);
                    (1.0 - fled) * go(opening, base, max_hp, speed, turns, rate, 0, escape, turn + 1)
                }),
            }
        }
        go(opening, base_rate, max_hp, speed, turns, base_rate, 0, 0, 0)
    }

    /// **Why [`pick_battle_action`] only ever throws balls.** §6-E2 presents BAIT and ROCK as a live
    /// trade-off — "Rock raises catch rate *and* flee rate; Bait does the inverse" — and they are, but
    /// run through [`catch_probability`] both come out behind a plain ball on the species with the
    /// most to gain from either.
    ///
    /// A lv23 Chansey (catch rate 30, speed stat ~35) is that species: the zone's hardest catch and a
    /// middling runner, so it wants ROCK's doubling and BAIT's protection more than anything else in
    /// the table. The horizon is 30 turns, the trip's whole ball supply.
    #[test]
    fn bait_and_rock_are_never_worth_throwing() {
        const HP: u16 = 200;
        const SPEED: u16 = 35;
        const BALLS: u32 = 30;
        let of = |o| catch_probability(o, 30, HP, SPEED, BALLS);
        let (balls, bait, rock) = (of(Opening::Balls), of(Opening::BaitFirst), of(Opening::RockFirst));
        println!("Chansey per encounter: balls {balls:.3} · bait-first {bait:.3} · rock-first {rock:.3}");

        assert!(balls > bait, "balls {balls:.3} should beat bait-first {bait:.3}");
        assert!(balls > rock, "balls {balls:.3} should beat rock-first {rock:.3}");
        // The closed form the driver's log line prints agrees with the 30-turn expansion, which is
        // what licenses `encounter_catch_chance` as the number to reason with.
        let closed_form = encounter_catch_chance(
            ball_catch_chance(30, HP, HP), flee_chance(SPEED, false, false));
        assert!((balls - closed_form).abs() < 0.005,
            "30 balls is effectively the limit: {balls:.3} vs {closed_form:.3}");
    }

    /// The same comparison for a target ROCK should suit best — Exeggcute is slow (speed stat ~27) and
    /// already catchable (rate 90), so doubling it saturates the ball's first roll outright. It still
    /// loses, because the doubled *flee* rate is charged on the rock's own turn and the counter it
    /// bought expires back to the base rate.
    #[test]
    fn rock_loses_even_where_it_looks_strongest() {
        let balls = catch_probability(Opening::Balls, 90, 200, 27, 30);
        let rock = catch_probability(Opening::RockFirst, 90, 200, 27, 30);
        println!("Exeggcute per encounter: balls {balls:.3} · rock-first {rock:.3}");
        assert!(balls > rock, "balls {balls:.3} should beat rock-first {rock:.3}");
    }

    /// The two event bits share a byte and are one apart, so an off-by-one here would read "in the
    /// zone" as "game over" — and the ejection test would pass for the wrong reason.
    #[test]
    fn the_safari_event_bits_are_adjacent() {
        assert_eq!(SAFARI_EVENT_BYTE, 0x24F / 8);
        assert_eq!(IN_SAFARI_ZONE, 1 << (0x24F % 8));
        assert_eq!(SAFARI_GAME_OVER, 1 << (0x24E % 8));
    }
}
