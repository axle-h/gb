//! Workstream E — Safari Zone proper.

use gb::geometry::Point8;
use gb::mmu::MMU;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::map::Map;
use crate::pokemon::policy::{DeterministicPolicy, PolicyStep};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::tile::MetaTile;
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::GameState;
use gb::ram::ROM;

// ── E1: the step budget
// ──────────────────────────────────────────────────────────────────────────

/// The live state of a Safari trip — `None` in [`GameState::safari`] when the player is not on
/// the clock, which is every map in the game bar the five Safari ones (and the gate, after
/// ejection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafariState {
    /// `wSafariSteps`, a big-endian word (`ld a, HIGH(502)` into the low address).
    pub steps_left: u16,
    /// `wNumSafariBalls`.
    pub balls_left: u8,
    /// `EVENT_SAFARI_GAME_OVER` — set by `SafariZoneGameOver` the instant the budget runs out,
    /// and consumed by the gate's `CheckAndResetEvent` on arrival.
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

// ── E2: the odds, and why BAIT and ROCK are never thrown
// ─────────────────────────────────────────

/// Rejection-sampling ceiling on `Rand1` for a Safari Ball — it shares the Ultra Ball's range
/// (`engine/items/item_effects.asm:200-208`), so the first check passes with probability
/// `(catch_rate + 1) / 151` rather than `/ 256`.
const SAFARI_BALL_RAND1_MAX: u16 = 150;

/// `BallFactor` for every ball but the Great Ball (`item_effects.asm:238-244`).
const SAFARI_BALL_FACTOR: u32 = 12;

/// Probability that one Safari Ball catches a target with `catch_rate`, at `current_hp` of
/// `max_hp`.
pub fn ball_catch_chance(catch_rate: u8, max_hp: u16, current_hp: u16) -> f64 {
    let first = (catch_rate as u16 + 1).min(SAFARI_BALL_RAND1_MAX + 1) as f64 / (SAFARI_BALL_RAND1_MAX + 1) as f64;
    let hp_term = (current_hp / 4).max(1) as u32;
    let x = ((max_hp as u32 * 255) / SAFARI_BALL_FACTOR) / hp_term;
    if x > 255 {
        return first;
    }
    first * (x + 1) as f64 / 256.0
}

/// Probability that the target flees at the end of a turn, given its live Speed *stat*.
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

/// Chance of eventually catching a target that is thrown at every turn until it is caught or
/// flees: `p / (p + (1 - p) · f)`.
pub fn encounter_catch_chance(per_ball: f64, per_turn_flee: f64) -> f64 {
    let denominator = per_ball + (1.0 - per_ball) * per_turn_flee;
    if denominator <= 0.0 { 0.0 } else { per_ball / denominator }
}

/// The battle half of a [`PolicyStep::SafariHunt`]: throw a ball at anything still wanted, run
/// from everything else.
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

// ── E3: the hunt
// ─────────────────────────────────────────────────────────────────────────────────

/// What [`pick`] wants the policy to do this tick.
pub enum Hunt {
    /// Issue this overworld action (a walk toward the zone, or a pace through grass).
    Walk(OverworldAction),
    /// Nothing to do this tick — a script is running, or the tile grid has not settled yet.
    Wait,
    /// The step is over; the policy pops it.
    Done,
}

/// Trip bookkeeping for the current [`PolicyStep::SafariHunt`], held by `DeterministicPolicy`.
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

const MONS_PER_BOX: usize = 20;

/// Species in `targets` that are not owned yet — the only reason to still be in the zone.
pub fn wanted(state: &GameState, targets: &[PokemonSpecies]) -> Vec<PokemonSpecies> {
    targets.iter().copied().filter(|s| !state.pokedex_owned.contains(s)).collect()
}

/// The overworld half of a [`PolicyStep::SafariHunt`] — pay, pace, get ejected, pay again.
pub fn pick(
    progress: &mut HuntProgress,
    state: &GameState,
    world_graph: &WorldGraph,
    actions: &[OverworldAction],
    targets: &[PokemonSpecies],
    map: Map,
    max_trips: u32,
) -> Hunt {
    // Ejection is not instantaneous, and the gap is a trap.
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
    // Room for the next catch. A full party *and* a full open box makes `ItemUseBall` refuse with
    // a text box, which from the policy's side is indistinguishable from a miss — it would throw
    // the rest of the trip's balls at nothing.
    if state.pokemon.len() >= 6 && state.boxed_pokemon.len() >= MONS_PER_BOX {
        println!("[safari] party and box {} are both full — no room for a catch, stopping",
            state.current_box + 1);
        return Hunt::Done;
    }
    if !inside {
        // Between trips (or before the first one): the budget and the wallet decide whether there
        // is another.
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
        // Walk (back) in.
        return match step_toward(world_graph, actions, state.map.map, map) {
            Some(action) => { progress.route_stuck = 0; Hunt::Walk(action) }
            // Not a failure yet.
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
        // A wait with no grass has to be bounded, and for a reason that is easy to miss.
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
/// concluding there is not one.
const ROUTE_PATIENCE: u32 = 400;

/// The best area to hunt each of the twelve species the zone adds to this save's dex, with its
/// encounter-slot share there.
pub mod grounds {
    use super::*;

    /// Centre — Rhyhorn 19.9 %, Exeggcute 9.8+9.8 %, Nidorino 9.8 %, Nidorina 5.1 %, Parasect 5.1
    /// %, Scyther 4.3 % (1.2 % in the east, its only other home).
    pub const CENTRE: &[PokemonSpecies] = &[
        PokemonSpecies::Rhyhorn, PokemonSpecies::Exeggcute, PokemonSpecies::Nidorino,
        PokemonSpecies::Nidorina, PokemonSpecies::Parasect, PokemonSpecies::Scyther,
    ];
    /// East — Doduo 19.9 %, Kangaskhan 4.3 % (1.2 % in the west).
    pub const EAST: &[PokemonSpecies] = &[PokemonSpecies::Doduo, PokemonSpecies::Kangaskhan];
    /// North — Paras 15.2 %, Venomoth 5.1 %, Chansey 4.3 % (1.2 % in the centre).
    pub const NORTH: &[PokemonSpecies] = &[
        PokemonSpecies::Paras, PokemonSpecies::Venomoth, PokemonSpecies::Chansey,
    ];
    /// West — Tauros 4.3 % (1.2 % in the north).
    pub const WEST: &[PokemonSpecies] = &[PokemonSpecies::Tauros];
}

/// The zone's land topology: a chain, not a hub.
const LAND_CHAIN: [Map; 5] = [
    Map::SafariZoneGate, Map::SafariZoneCenter, Map::SafariZoneEast,
    Map::SafariZoneNorth, Map::SafariZoneWest,
];

/// North → West has four warps in two pairs, and which one a leg wants depends on what it is for.
const WEST_LANDING: Point8 = Point8 { x: 26, y: 0 };

/// One hop toward `map` from wherever the player is standing.
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
        // The pinned landing first, then any crossing to the same map — a hunt that cannot reach
        // the preferred warp should still get *somewhere* rather than stand still.
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

/// The overworld half of [`PolicyStep::SafariExit`] — walk out of the zone from wherever a hunt
/// left us, and pop once we are standing on the gate mat.
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
    /// E3/E4 — hunt `targets` in `SafariZoneCenter`, then walk out through the gate.
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

    /// E3 — sweep all four areas, hunting each species where its encounter slot is fattest (see
    /// [`grounds`]).
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
            // No `enter` steps between hunts: the hunt walks itself in along [`LAND_CHAIN`],
            // which is the *only* thing that works from both places a hunt can end — deep inside
            // the previous area, or standing on the gate mat after an ejection.
            steps.push(Self::SafariHunt { targets, map: area, max_trips });
        }
        steps.extend(Self::safari_exit_steps());
        steps
    }

    /// Out of the zone and back onto an outdoor Fuchsia tile, from wherever the last hunt ended —
    /// deep in an area, or standing on the gate mat after an ejection.
    fn safari_exit_steps() -> Vec<Self> {
        vec![Self::SafariExit, Self::enter(Map::FuchsiaCity)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Safari Ball's own constants, pinned: the `[0,150]` rejection range and BallFactor 12
    /// are what make a full-HP throw a flat 33.6 % on its second roll, and getting either wrong
    /// would silently change every number this module prints.
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

    /// `add a` on the speed byte is a *carry*, not a wrap: over 127 and the target is gone
    /// whatever else happens.
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

    /// Exact probability of catching a full-HP target under `opening`, over a `turns`-turn
    /// horizon.
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

    /// Why [`pick_battle_action`] only ever throws balls.
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
        // The closed form the driver's log line prints agrees with the 30-turn expansion, which
        // is what licenses `encounter_catch_chance` as the number to reason with.
        let closed_form = encounter_catch_chance(
            ball_catch_chance(30, HP, HP), flee_chance(SPEED, false, false));
        assert!((balls - closed_form).abs() < 0.005,
            "30 balls is effectively the limit: {balls:.3} vs {closed_form:.3}");
    }

    /// The same comparison for a target ROCK should suit best — Exeggcute is slow (speed stat
    /// ~27) and already catchable (rate 90), so doubling it saturates the ball's first roll
    /// outright.
    #[test]
    fn rock_loses_even_where_it_looks_strongest() {
        let balls = catch_probability(Opening::Balls, 90, 200, 27, 30);
        let rock = catch_probability(Opening::RockFirst, 90, 200, 27, 30);
        println!("Exeggcute per encounter: balls {balls:.3} · rock-first {rock:.3}");
        assert!(balls > rock, "balls {balls:.3} should beat rock-first {rock:.3}");
    }

    /// The two event bits share a byte and are one apart, so an off-by-one here would read "in
    /// the zone" as "game over" — and the ejection test would pass for the wrong reason.
    #[test]
    fn the_safari_event_bits_are_adjacent() {
        assert_eq!(SAFARI_EVENT_BYTE, 0x24F / 8);
        assert_eq!(IN_SAFARI_ZONE, 1 << (0x24F % 8));
        assert_eq!(SAFARI_GAME_OVER, 1 << (0x24E % 8));
    }
}
