//! Tests for workstream `maps` — see `docs/postgame-coverage-plan.md` §8-L and
//! [`crate::pokemon::postgame::maps`].
//!
//! **L1 lives with the module**, in `postgame/maps.rs`'s own `#[cfg(test)]` block, because it needs
//! no emulator at all: it reads the ROM. This file is L2 (the emulated tour, one test per Fly hub)
//! and L3 (the awkward set), plus the L4 report.
//!
//! Rooted on **H's output**, the chain head — a save with every badge, every HM and Fly, which is
//! the only state from which a tour of Kanto is one step per room rather than a playthrough.

use super::super::*;
use crate::pokemon::postgame::maps;

/// H's output and the chain head (§9): Route 15, all eight badges, Fly on Articuno.
const AIDES: &[u8] = include_bytes!("../../data/postgame-aides.bin");

/// How many doors deep a hub tour goes. Two covers a mart's upper floor and a gym's interior, which
/// is what §8-L means by "every building in the cluster"; deeper turns Silph Co and the Rocket
/// Hideout into their own afternoon.
const TOUR_DEPTH: u8 = 2;

/// Drive one hub's tour and return `(entered, missed)`.
///
/// The tour is deliberately *reporting* rather than asserting: `EnterMapIfReachable` pops with a
/// reason instead of stalling, so the run reaches the end of the town and the caller decides what
/// counts as a failure. Which is the whole L4 deliverable — the list of rooms that could not be
/// entered is worth more than a red test that stopped at the first one.
fn tour(hub: Map, budget: Duration) -> (Vec<Map>, Vec<Map>) {
    use std::collections::HashMap;
    let mut planned = maps::rooms_off(hub, TOUR_DEPTH);
    planned.extend(maps::connected_routes(hub));
    let mut fixture = TestFixture::new(AIDES, budget, PolicyStep::tour_hub_steps(hub, TOUR_DEPTH));

    let mut entered: Vec<Map> = Vec::new();
    // Per-room health, taken as the **best** reading over every tick spent in the room rather than
    // the first. ⚠️ The first is worthless: the meta-tile grid is briefly unsettled on arrival (§10 —
    // it is the same instability the encounter pacer's `stalled` counter exists for), so a check
    // performed on the tick the warp lands sees an empty action list in a perfectly ordinary Pokémon
    // Center. Asserting on that failed Viridian's tour before it had walked a step.
    let mut health: HashMap<Map, (usize, bool)> = HashMap::new();
    while !fixture.agent.policy_exhausted() {
        fixture.step();
        let state = fixture.game_state();
        let here = state.map.map;
        if !planned.contains(&here) { continue }
        if !entered.contains(&here) { entered.push(here); }
        let actions = state.map.actions();
        let has_exit = actions.iter().any(|a| matches!(a.tile,
            MetaTile::Warp { .. } | MetaTile::Connection { .. }));
        let entry = health.entry(here).or_insert((0, false));
        entry.0 = entry.0.max(actions.len());
        entry.1 |= has_exit;
    }

    for room in &entered {
        let (actions, has_exit) = health[room];
        assert!(actions > 0, "{room}: the agent never saw a single action here");
        assert!(has_exit, "{room}: {actions} things to do and none of them a way out — a room the \
            agent can enter and not leave is exactly what this tour is looking for");
    }

    let missed: Vec<Map> = planned.iter().copied().filter(|m| !entered.contains(m)).collect();
    println!("== {hub}: entered {}/{} rooms", entered.len(), planned.len());
    if !missed.is_empty() {
        for map in &missed {
            println!("   MISSED {map}{}", match maps::known_unreachable(*map) {
                Some(why) => format!(" — expected: {}", why.why()),
                None => " — NOT on the known-unreachable list".into(),
            });
        }
    }
    (entered, missed)
}

/// Assert a hub tour entered everything it was not already known to be unable to.
fn assert_toured(hub: Map, budget: Duration) {
    let (entered, missed) = tour(hub, budget);
    assert!(!entered.is_empty(), "{hub}: the tour entered nothing at all — did the Fly land?");
    let unexpected: Vec<Map> = missed.into_iter()
        .filter(|m| maps::known_unreachable(*m).is_none()).collect();
    assert!(unexpected.is_empty(),
        "{hub}: {} rooms could not be entered and none of them is a known one-way door: {unexpected:?}\n\
         If these are genuinely unreachable, add them to `postgame::maps::known_unreachable` with a \
         reason — that list IS L4's deliverable.", unexpected.len());
}

/// **Task L2a** — Pallet Town, Viridian and Pewter. Emulates ≤180 min (≈8 min wall).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_southern_hubs() {
    for hub in [Map::PalletTown, Map::ViridianCity, Map::PewterCity] {
        assert_toured(hub, Duration::from_mins(120));
    }
}

/// **Task L2b** — Cerulean, Lavender and Vermilion. Emulates ≤180 min (≈8 min wall).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_central_hubs() {
    for hub in [Map::CeruleanCity, Map::LavenderTown, Map::VermilionCity] {
        assert_toured(hub, Duration::from_mins(120));
    }
}

/// **Task L2c** — Celadon and Saffron, the two with department stores and Silph Co behind them.
/// Emulates ≤180 min (≈8 min wall).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_western_hubs() {
    for hub in [Map::CeladonCity, Map::SaffronCity] {
        assert_toured(hub, Duration::from_mins(150));
    }
}

/// **Task L2d** — Fuchsia, Cinnabar and Indigo Plateau. Emulates ≤180 min (≈8 min wall).
///
/// ⚠️ Fuchsia is where the **Safari Zone gate** sits, and stepping into it starts a paid trip: the
/// warden's script asks for ¥500 and the 502-step budget begins. The tour enters the gate building
/// like any other room; what it must not do is walk *through* it, which is why `TOUR_DEPTH` stops
/// where it does. L3 covers the zone itself.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_southern_islands() {
    for hub in [Map::FuchsiaCity, Map::CinnabarIsland, Map::IndigoPlateau] {
        assert_toured(hub, Duration::from_mins(120));
    }
}
