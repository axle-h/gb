//! Pallet/Viridian → Pewter → Mt Moon → Cerulean → Vermilion.
//!
//! The two navigation tests here are in the fast tier: between them they cover forward `EnterMap`
//! chaining across connections and the warp-graph traversal of a fragmented dungeon, which is the
//! machinery every later leg is built on. If they pass, a failure further down the chain is about
//! that leg rather than about routing.

use super::*;

#[test]
fn can_navigate_to_pewter_city() {
    // Explicit forward navigation (Viridian City → Viridian Forest → Pewter City), the same
    // single-hop `EnterMap` chain `complete_game_steps` uses. The abstract `goto` form this test
    // used previously needed the deleted pre-built world graph.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/viridian-city-pokemart-shopping.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::enter(Map::ViridianCity),   // exit the Mart (the save state is inside it)
            PolicyStep::enter(Map::Route2),
            PolicyStep::enter(Map::ViridianForestSouthGate),
            PolicyStep::enter(Map::ViridianForest),
            PolicyStep::enter(Map::ViridianForestNorthGate),
            PolicyStep::enter(Map::Route2),
            PolicyStep::enter(Map::PewterCity),
        ]
    );

    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::PewterCity, "agent should have navigated to Pewter City");
}

/// Explicit Mt Moon traversal, discovered from the ROM warp graph + live sprite-resolved
/// reachability. Mt Moon's floors are fragmented into disjoint walkable components joined only by
/// warps; the sole route to the Route 4 east exit crosses B2F between the (21,17) and (5,7) warps,
/// which is plugged by the two fossil item-sprites. Collecting one fossil (which also triggers the
/// mandatory Super Nerd battle and makes him grab the other fossil) opens the 1-wide passage.
///
///   1F(5,5)→B1F(5,5) [comp A] → walk → B1F(21,17)→B2F(21,17)
///     → collect Helix Fossil (beat Super Nerd, corridor opens)
///     → walk → B2F(5,7)→B1F(23,3) [comp D] → walk → B1F(27,3)→Route4 → Cerulean
#[test]
fn can_navigate_mt_moon() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/mt-moon.bin"),
        Duration::from_mins(40),
        PolicyStep::mt_moon_traversal(),
    );

    fixture.pimp_pokemon();
    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::CeruleanCity, "agent should have navigated to Cerulean City");
}

/// From a post-Cascade save state, do the full Bill → SS Ticket → Route 5 → Vermilion leg.
///
/// Route 5 is unreachable from the Cerulean Pokécenter terrace directly (one-way south ledges split
/// the city; verified ROM-faithful). The real path is the **trashed-house bridge**, which only opens
/// after meeting Bill (the `CERULEANCITY_GUARD2` guard at raw (27,12) clears): enter the trashed
/// house from the main terrace, take its back door to land at Cerulean (27,9) — which IS in the
/// Route-5-reaching terrace — then walk onto Route 5. So: Nugget Bridge → Bill (SS Ticket) → return →
/// trashed-house bridge → Route 5 → Underground Path → Route 6 → Vermilion.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_vermilion() {
    // Exactly the leg folded into `complete_game_steps` (Bill/SS-Ticket → trashed-house bridge →
    // Vermilion), so this test and the full playthrough stay in lockstep. It subsumes the SS Ticket
    // hand-off, which is why there is no separate test for it.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-cascade.bin"),
        Duration::from_mins(40),
        PolicyStep::cerulean_to_vermilion_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("bag: {:?}", s.bag.iter().collect::<Vec<_>>());
    assert!(s.bag.contains(&ItemId::SSTicket), "should have obtained the SS Ticket from Bill");
    assert_eq!(s.map.map, Map::VermilionCity, "should reach Vermilion City via the trashed-house bridge");
    fixture.save_state_named("src/pokemon/data/at-vermilion.bin").unwrap();
}
