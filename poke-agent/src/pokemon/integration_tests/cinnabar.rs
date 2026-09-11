//! Surf to Cinnabar, the Mansion's Secret Key, the Volcano Badge and the Seafoam Islands.

use super::*;

/// From the ROM alone, Seafoam's warp graph with Surf reaches a tile beside Articuno at B4F (6,1).
#[test]
fn seafoam_articuno_is_reachable_offline() {
    use crate::pokemon::map_metadata::MapMetadataReader;
    use crate::pokemon::tile_map::MetaTileMap;
    use std::collections::{HashSet, VecDeque};
    use std::sync::Arc;

    let mmu = gb::mmu::MMU::from_rom(roms::POKERED).unwrap();
    let build = |map: Map, at: Point8| {
        let metadata = Arc::new(mmu.read_map_metadata(map).unwrap());
        let current = crate::pokemon::map_metadata::CurrentMap {
            player_position: at,
            player_direction: crate::pokemon::map_metadata::PlayerFacingDirection::Down,
            sprites: vec![], metadata, closed_doors: vec![], card_key_locked: false,
            grass_encounter_rate: 0,
            header_loaded: true,
            surfing: true,
            sprites_loaded: true,
            script_cancelled_warps: Vec::new(),
            standing_on_warp: true,
        };
        let mut tm = MetaTileMap::new(&current);
        tm.can_surf = true;
        tm
    };

    let start = (Map::SeafoamIslands1F, Point8 { x: 26, y: 17 });
    let mut seen: HashSet<(Map, Point8)> = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    let mut articuno_entry = None;

    while let Some((map, at)) = queue.pop_front() {
        let tm = build(map, at);
        let reach = tm.reachable_tiles();
        if map == Map::SeafoamIslandsB4F && articuno_entry.is_none() {
            let adj = [Point8 { x: 6, y: 0 }, Point8 { x: 6, y: 2 },
                       Point8 { x: 5, y: 1 }, Point8 { x: 7, y: 1 }];
            if adj.iter().any(|p| reach.contains(p)) { articuno_entry = Some((map, at)); }
        }
        for (_, tile) in tm.all_reachable_warps_and_connections() {
            if let MetaTile::Warp { to_map, to_position } = tile {
                if !matches!(to_map, Map::SeafoamIslands1F | Map::SeafoamIslandsB1F
                    | Map::SeafoamIslandsB2F | Map::SeafoamIslandsB3F | Map::SeafoamIslandsB4F) { continue; }
                // Enqueue each node once, or two floors that warp to each other loop for ever.
                if seen.insert((to_map, to_position)) { queue.push_back((to_map, to_position)); }
            }
        }
    }

    let entry = articuno_entry.expect("Articuno should be reachable from the Route-20 east entrance");
    println!("Articuno reachable from {} @ {}", entry.0, entry.1);
}

/// Saffron to Cinnabar by Route 6, Diglett's Cave, Route 2's Cut trees and Pallet, then Surf.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_surf_to_cinnabar() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-marsh-badge.bin"),
        Duration::from_mins(120),
        PolicyStep::saffron_to_cinnabar_steps(),
    );
    let s = fixture.run_until(|s| s.map.map == Map::CinnabarIsland);
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    fixture.save_state_named("src/pokemon/data/at-cinnabar.bin").unwrap();
}

/// The Pokémon Mansion switch-gate maze to the Secret Key.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_secret_key() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-cinnabar.bin"),
        Duration::from_mins(20),
        PolicyStep::mansion_secret_key_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("on {} @ {}", s.map.map, s.map.player_position);
    assert!(s.bag.contains(&ItemId::SecretKey), "should have collected the Secret Key");
    fixture.save_state_named("src/pokemon/data/post-secret-key.bin").unwrap();
}

/// Heal, clear the Cinnabar Gym's quiz-gate maze, and beat Blaine for the Volcano Badge.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_volcano_badge() {
    // Animations on: `TestFixture::with_original_battle_timing`.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-secret-key.bin"),
        Duration::from_mins(40),
        PolicyStep::volcano_badge_steps(),
    ).with_original_battle_timing();
    let s = fixture.run_until(|s| s.badges.contains(Badge::VolcanoBadge));
    println!("on {} @ {} — badges = {:?}", s.map.map, s.map.player_position, s.badges);
    fixture.save_state_named("src/pokemon/data/post-volcano-badge.bin").unwrap();
}

/// Seafoam: fill both B3F holes to stop the B4F current, fall through, and catch Articuno.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_articuno() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-volcano-badge.bin"),
        Duration::from_mins(60),
        PolicyStep::seafoam_articuno_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("final: map {} @ {}", s.map.map, s.map.player_position);
    for (i, p) in s.pokemon.iter().enumerate() {
        let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}", m.name)).collect();
        println!("  slot{i}: {:?} lv{} {}/{}hp — {}", p.species, p.level, p.current_hp, p.stats.hp, moves.join("/"));
    }
    assert!(s.pokemon.iter().any(|p| p.species == PokemonSpecies::Articuno),
        "Articuno should be in the party after the Seafoam leg");
    assert_eq!(s.map.map, Map::CinnabarIsland, "the leg should end back on Cinnabar Island");
    fixture.save_state_named("src/pokemon/data/post-articuno.bin").unwrap();
}

/// Cuts the fixture the boulder test stands on: B3F, Strength armed, all four boulders untouched.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn cut_seafoam_b3f_fixture() {
    let all = PolicyStep::seafoam_articuno_steps();
    let upto = all.iter().position(|s| matches!(s, PolicyStep::DropBoulderInHole { .. }))
        .expect("the leg has a hole step");
    let steps: Vec<PolicyStep> = all.into_iter().take(upto).collect();
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-volcano-badge.bin"), Duration::from_mins(60), steps);
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("stopped on {} @ {} boulders {:?}", s.map.map, s.map.player_position, s.map.boulders());
    assert_eq!(s.map.map, Map::SeafoamIslandsB3F);
    assert!(s.map.can_strength, "the fixture must carry Strength and the badge");
    fixture.save_state_named("src/pokemon/data/seafoam-b3f.bin").unwrap();
}

/// Seafoam B3F's two holes are each filled by the one boulder that can reach it.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn both_seafoam_holes_are_filled_by_the_only_boulders_that_can_reach_them() {
    use gb::geometry::Point8;
    const HOLE_A: Point8 = Point8 { x: 3, y: 16 };
    const HOLE_B: Point8 = Point8 { x: 6, y: 16 };
    const ONLY_A: Point8 = Point8 { x: 3, y: 15 };
    const ONLY_B: Point8 = Point8 { x: 8, y: 14 };

    let mut fixture = TestFixture::new(
        include_bytes!("../data/seafoam-b3f.bin"), Duration::from_mins(20),
        vec![PolicyStep::DropBoulderInHole { hole: HOLE_A, boulder: Some(ONLY_A) },
             PolicyStep::DropBoulderInHole { hole: HOLE_B, boulder: Some(ONLY_B) }]);
    let before = fixture.game_state().map.boulders();
    assert_eq!(before.len(), 4, "the fixture starts with all four: {before:?}");

    fixture.step_until_exhausted();
    let after = fixture.game_state().map.boulders();
    println!("boulders {before:?} -> {after:?}");

    assert!(!after.contains(&ONLY_A), "(3,15) should be in hole A: {after:?}");
    assert!(!after.contains(&ONLY_B), "(8,14) should be in hole B: {after:?}");
    assert_eq!(after.len(), 2, "exactly the two named boulders should have left: {after:?}");
}

/// A boulder floor's menu must not cost more than a handful of ordinary ones.
#[test]
fn a_boulder_floors_action_menu_is_not_a_search_per_tick() {
    /// Calls to average over.
    const CALLS: u32 = 100;
    let cost = |bytes: &[u8]| {
        let mut fixture = TestFixture::new(bytes, Duration::from_secs(5), vec![]);
        let state = fixture.game_state();
        std::hint::black_box(state.map.actions());     // warm the cache, as a tick after the first is
        let started = std::time::Instant::now();
        for _ in 0..CALLS { std::hint::black_box(state.map.actions()); }
        (started.elapsed() / CALLS, state.map.actions().len(), state.map.boulders().len())
    };

    let (plain, plain_rows, plain_boulders) = cost(include_bytes!("../data/at-cinnabar.bin"));
    assert_eq!(plain_boulders, 0, "the baseline map must have no boulders to search over");
    println!("at-cinnabar    {plain_rows} rows, no boulders  {plain:?}/call");

    for (name, bytes) in [("seafoam-b3f", &include_bytes!("../data/seafoam-b3f.bin")[..]),
                          ("vr1f-strength", &include_bytes!("../data/vr1f-strength.bin")[..])] {
        let (cost, rows, boulders) = cost(bytes);
        println!("{name:14} {rows} rows, {boulders} boulders  {cost:?}/call");
        assert!(boulders > 0 && rows > 0, "{name} is a boulder floor with rows");
        assert!(cost < plain * 20,
            "{name}'s menu costs {cost:?} against {plain:?} on a map with nothing to search; \
             a goal row is re-running its layout search every tick");
    }
}

#[test]
fn a_seafoam_warp_on_the_water_is_stepped_onto_rather_than_leant_on() {
    use gb::geometry::Point8;
    const HOLE: Point8 = Point8 { x: 21, y: 17 };

    let mut fixture = TestFixture::new(
        include_bytes!("../data/seafoam-b3f-on-the-water-warp.bin"), Duration::from_mins(2),
        vec![PolicyStep::EnterMap { to_map: Map::SeafoamIslandsB4F, to_position: Some(HOLE) }]);
    let start = fixture.game_state();
    println!("from {} @ {} surfing={}", start.map.map, start.map.player_position, start.map.surfing);
    assert_eq!(start.map.map, Map::SeafoamIslandsB3F);
    assert_eq!(start.map.player_position, HOLE, "the state is dropped standing on the entry itself");
    assert!(start.map.surfing, "and on the water, which is the whole of it");

    fixture.step_until_exhausted();
    let end = fixture.game_state();
    println!("ended on {} @ {}", end.map.map, end.map.player_position);
    assert_eq!(end.map.map, Map::SeafoamIslandsB4F,
        "the entry has to fire, and holding the outward direction on it never will");
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn a_walk_the_surf_mount_itself_finishes_says_that_it_arrived() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-articuno.bin"), Duration::from_mins(4),
        vec![PolicyStep::enter(Map::Route20)],
    ).with_coverage();
    assert_eq!(fixture.game_state().map.map, Map::CinnabarIsland);

    fixture.step_until_exhausted();
    assert_eq!(fixture.game_state().map.map, Map::Route20, "the walk has to cross the seam");

    let log = fixture.coverage.as_ref().expect("coverage was asked for");
    println!("[coverage] {}", log.summary());
    let quiet: Vec<&str> = log.entries()
        .filter(|entry| entry.verdict == crate::pokemon::integration_tests::coverage::Verdict::Silent)
        .map(|entry| entry.id.as_str())
        .collect();
    assert!(quiet.is_empty(),
        "the mount crossed the seam and nothing reported the arrival: {quiet:?}\n{}", log.report());
    // A completion, not merely no silence: the walk arrived.
    assert!(
        log.entries().any(|entry| entry.id.contains("ConnectionWater")
            && entry.verdict == crate::pokemon::integration_tests::coverage::Verdict::Completed),
        "no water crossing completed at all, so this leg did not exercise the arm:\n{}",
        log.report(),
    );
}
