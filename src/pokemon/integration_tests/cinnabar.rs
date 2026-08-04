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
/// the two party members the Elite Four needs — a Slowpoke HM-slave (Strength for Victory Road, Dig for
/// the way out) and the Ice sweeper itself.
///
/// Seeded from `at-mansion-blizzard.bin` (TM14 Blizzard already in the bag from Mansion B1F) so the
/// leg's `TeachMove` puts **Blizzard on Articuno** rather than skipping — that STAB is what makes
/// Lance's room winnable.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_articuno() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-mansion-blizzard.bin"),
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
