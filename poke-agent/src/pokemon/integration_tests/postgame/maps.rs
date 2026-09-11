use super::super::*;
use crate::pokemon::postgame::maps;

const AIDES: &[u8] = include_bytes!("../../data/postgame-aides.bin");

/// How many doors deep a hub tour goes.
const TOUR_DEPTH: u8 = 2;

/// Drive one hub's tour and return `(entered, missed)`.
fn tour(hub: Map, budget: Duration) -> (Vec<Map>, Vec<Map>) {
    use std::collections::HashMap;
    let mut planned = maps::rooms_off(hub, TOUR_DEPTH);
    planned.extend(maps::connected_routes(hub));
    let mut fixture = TestFixture::new(AIDES, budget, PolicyStep::tour_hub_steps(hub, TOUR_DEPTH));

    let mut entered: Vec<Map> = Vec::new();
    // Per-room health, the best reading over every tick in the room.
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

/// Pallet Town, Viridian and Pewter.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_southern_hubs() {
    for hub in [Map::PalletTown, Map::ViridianCity, Map::PewterCity] {
        assert_toured(hub, Duration::from_mins(120));
    }
}

/// Cerulean, Lavender and Vermilion.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_central_hubs() {
    for hub in [Map::CeruleanCity, Map::LavenderTown, Map::VermilionCity] {
        assert_toured(hub, Duration::from_mins(120));
    }
}

/// Celadon and Saffron, with the department stores and Silph Co.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_western_hubs() {
    for hub in [Map::CeladonCity, Map::SaffronCity] {
        assert_toured(hub, Duration::from_mins(150));
    }
}

/// Fuchsia, Cinnabar and Indigo Plateau.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_tour_the_southern_islands() {
    for hub in [Map::FuchsiaCity, Map::CinnabarIsland, Map::IndigoPlateau] {
        assert_toured(hub, Duration::from_mins(120));
    }
}
