//! Surf to Cinnabar → Pokémon Mansion (Secret Key) → Volcano Badge → the Seafoam Islands (Articuno).

use super::*;

/// Offline proof that the Articuno route exists at all, straight from the ROM: BFS the Seafoam warp
/// graph out of the Route-20 east entrance with Surf enabled, and check that some floor/entry pair
/// puts a walkable tile next to Articuno at B4F (6,1).
///
/// No emulator, so it belongs to the fast tier — it is the cheap guard on the map decoding that
/// `can_catch_articuno` then spends 45 minutes of game time depending on.
#[test]
fn seafoam_articuno_is_reachable_offline() {
    use crate::pokemon::map_metadata::MapMetadataReader;
    use crate::pokemon::tile_map::MetaTileMap;
    use std::collections::{HashSet, VecDeque};
    use std::sync::Arc;

    let mmu = crate::mmu::MMU::from_rom(roms::POKERED).unwrap();
    let build = |map: Map, at: Point8| {
        let metadata = Arc::new(mmu.read_map_metadata(map).unwrap());
        let current = crate::pokemon::map_metadata::CurrentMap {
            player_position: at,
            player_direction: crate::pokemon::map_metadata::PlayerFacingDirection::Down,
            sprites: vec![], metadata, closed_doors: vec![], card_key_locked: false,
            // Seafoam is surf routing; grass never enters into it.
            grass_encounter_rate: 0,
            header_loaded: true,
            surfing: true,
            sprites_loaded: true,
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
            // Articuno is at (6,1); reaching it means standing on one of its four neighbours.
            let adj = [Point8 { x: 6, y: 0 }, Point8 { x: 6, y: 2 },
                       Point8 { x: 5, y: 1 }, Point8 { x: 7, y: 1 }];
            if adj.iter().any(|p| reach.contains(p)) { articuno_entry = Some((map, at)); }
        }
        for (_, tile) in tm.all_reachable_warps_and_connections() {
            if let MetaTile::Warp { to_map, to_position } = tile {
                if !matches!(to_map, Map::SeafoamIslands1F | Map::SeafoamIslandsB1F
                    | Map::SeafoamIslandsB2F | Map::SeafoamIslandsB3F | Map::SeafoamIslandsB4F) { continue; }
                // Only ever enqueue a node once: two floors that warp to each other otherwise keep
                // re-queueing one another and the walk never terminates.
                if seen.insert((to_map, to_position)) { queue.push_back((to_map, to_position)); }
            }
        }
    }

    let entry = articuno_entry.expect("Articuno should be reachable from the Route-20 east entrance");
    println!("Articuno reachable from {} @ {}", entry.0, entry.1);
}

/// Saffron → Cinnabar Island: Route 6 (threading its gate) → Vermilion → Diglett's Cave → Route 2
/// (two Cut trees either side of its mid-route gate) → Viridian → Route 1 → Pallet, then **Surf**
/// across Route 21. The first leg that mounts Surf, so it is also the test of water connections
/// being crossable at all.
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

/// Navigate the Pokémon Mansion switch-gate maze and collect the **Secret Key** that unlocks the
/// Cinnabar Gym. One global switch toggles every floor's sliding doors, and the only way to the B1F
/// key is to fall through a 3F hole to 1F's right side.
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

/// Exit the mansion, heal, and clear the Cinnabar Gym's quiz-gate snake maze — `DefeatGymLeader` beats
/// each fire trainer via line of sight to unlock the gate ahead — then beat Blaine for the **Volcano
/// Badge**.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_volcano_badge() {
    // ⚠️ Pinned to the pre-**J** battle timing — see `TestFixture::with_original_battle_timing`.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-secret-key.bin"),
        Duration::from_mins(40),
        PolicyStep::volcano_badge_steps(),
    ).with_original_battle_timing();
    let s = fixture.run_until(|s| s.badges.contains(Badge::VolcanoBadge));
    println!("on {} @ {} — badges = {:?}", s.map.map, s.map.player_position, s.badges);
    fixture.save_state_named("src/pokemon/data/post-volcano-badge.bin").unwrap();
}

/// The Seafoam Islands detour, off Cinnabar and back: Sokoban-push the B3F boulders into the two floor
/// holes to kill the B4F current, fall through to B4F, and take **Articuno** with the Master Ball. Adds
/// the Ice sweeper the Elite Four needs. (It used to add a Slowpoke HM-slave for Strength and Dig too;
/// Blastoise carries both.)
///
/// Seeded from `post-volcano-badge.bin`, which is where the mainline is when it takes this detour and
/// already carries TM14 Blizzard out of Mansion B1F — so the leg's `TeachMove` puts **Blizzard on
/// Articuno** rather than skipping, and that STAB is what makes Lance's room winnable. (It used to be
/// a hand-cut `at-mansion-blizzard.bin`, which was a second root nothing produced and which pinned
/// the old party.)
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

/// **The fixture the boulder test below stands on**: B3F with Strength armed and all four boulders
/// untouched. Cut here rather than replayed, because the Seafoam leg is 60 game-minutes end to end
/// and that is no way to debug a puzzle that lives in its last two steps.
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

/// **Seafoam B3F's two holes, and it is the floor that catches what Victory Road's cannot.**
///
/// Victory Road's goals are all *switches*, and a switch keeps its boulder — so a switch goal is
/// done when one is standing on the target, and `a_strength_puzzle_is_one_decision_…` proves that
/// path. A **hole swallows the boulder**, so nothing is ever standing on one and that same test is
/// structurally unreachable for it. Two separate completion tests, one in the agent and one in the
/// policy step, were written against the switch shape and silently never fired here; between them
/// they pushed for the whole budget, spent the *other* hole's only capable boulder in passing, and
/// then stalled on a floor that was already solved. See `AgentState::SolvingBoulderPuzzle` and
/// `PolicyStep::DropBoulderInHole`.
///
/// This floor is also the reason a goal row names its boulder: of the four here, only (3,15) can
/// reach (3,16) and only (8,14) can reach (6,16).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn both_seafoam_holes_are_filled_by_the_only_boulders_that_can_reach_them() {
    use crate::geometry::Point8;
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

    // ⚠️ **Which two went matters as much as how many.** The first goal shoves a third boulder
    // west to clear the corridor, and a driver that kept pushing after its own boulder dropped
    // sent (8,14) down hole B in passing — leaving the count right, both holes full, and the
    // second step wanting a boulder that no longer existed.
    assert!(!after.contains(&ONLY_A), "(3,15) should be in hole A: {after:?}");
    assert!(!after.contains(&ONLY_B), "(8,14) should be in hole B: {after:?}");
    assert_eq!(after.len(), 2, "exactly the two named boulders should have left: {after:?}");
}

/// **A boulder floor's menu must not cost more than a handful of ordinary ones.**
///
/// ⚠️ **`actions()` is called on every 20 ms agent tick**, and a goal row's existence is a capped
/// BFS over boulder layouts, one per (boulder, target) pair. Emitting the rows without caching the
/// searches measured **11.4 ms per call on Seafoam B3F and 3.1 ms on Victory Road 1F against 82 us
/// on a map with no boulders** — and a tick is 20 ms of game time that costs about 0.4 ms of wall
/// clock to emulate, so the menu alone dropped the agent from ~48x real time to **1.9x**. The
/// coverage walk of 2026-09-07 spent 73 minutes of wall clock to buy 2.3 of the 24 game-hours it
/// was given and stopped with the frontier wide open. `PlanKey` is the fix and this is its bound.
///
/// The ceiling is deliberately loose — twenty times a boulder-free map, where the fix measured
/// three — because this is guarding against a regression of two orders of magnitude, not policing
/// microseconds on whatever machine happens to run it.
#[test]
fn a_boulder_floors_action_menu_is_not_a_search_per_tick() {
    /// Calls to average over. Small: the first call on a fresh `MetaTileMap` is the cold one that
    /// fills `PLAN_CACHE`, and it is the *steady state* that runs fifty times a second.
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

/// ⭐ **A warp on the water is taken by stepping onto it, and the agent used to lean on it.**
///
/// `SeafoamIslandsB3F:21,17` is the bottom-right hole down to B4F: water, on the map's last row,
/// and reachable only by surfing. `home/overworld.asm`'s `.noDirectionChange` tests
/// `wWalkBikeSurfState` for `$02` before it looks at anything else. On foot, walking into the wall
/// in front while standing on a warp entry runs `ExtraWarpCheck` and then `CheckWarpsCollision`, and
/// the warp fires — that is how every map-edge ladder in the game is taken. Surfing, the branch goes
/// to `CollisionCheckOnWater` and the next instruction is `jp c, OverworldLoop`;
/// `CheckWarpsCollision` is not on that path at all. What is left is `CheckWarpsNoCollision`, which
/// runs on a completed **step**, so the entry has to be arrived at rather than pressed against.
///
/// The coverage walk of 2026-09-09 hit it twice a sweep — here and at the identical `B4F:21,17`
/// going back up — holding Down for 60 s of game time each time and then reporting that it "did not
/// arrive" while standing exactly there. Measured on this state: 120 ticks of Down move nothing, Up
/// and then Down warps.
///
/// ⚠️ **Two places had to learn it, and fixing the first alone changed nothing.**
/// [`MetaTileMap::actions`](crate::pokemon::tile_map::MetaTileMap::actions) builds
/// `[opposite(dir), dir]` for a surfing player standing on the entry — but `OverworldMovement` tests
/// for a border warp *before* it consults the route, and pressed the outward direction itself. Both
/// carry the `surfing` condition now and this test fails if either is taken away.
///
/// The fixture is the save state the walk dropped at the moment the verdict turned
/// (`TestFixture::observe_coverage`), which is the only moment it exists: the square is on the far
/// side of a current-swept channel and cannot be stood on again by replaying anything.
// Default tier, unlike everything else in this file that drives the agent: the fixture is dropped
// two ticks from the answer, so the whole test is 30 ms and there is no reason to make anyone opt in
// to a routing regression this narrow.
#[test]
fn a_seafoam_warp_on_the_water_is_stepped_onto_rather_than_leant_on() {
    use crate::geometry::Point8;
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
