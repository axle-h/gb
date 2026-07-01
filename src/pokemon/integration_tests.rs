
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::pokemon::policy::{DeterministicPolicy, PolicyStep};
use crate::pokemon::*;
use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason, PokemonAgent, AGENT_RESOLUTION};
use crate::pokemon::battle::BattleType;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::tile::JumpDirection;
use crate::pokemon::tile::MetaTile;
use crate::ram::{RAM, ROM};

pub const PALLET_TOWN_STATE: &[u8] = include_bytes!("data/pallet-town-state.bin");
pub const ROUTE1_STATE: &[u8] = include_bytes!("data/route1-state.bin");
pub const BATTLE_STATE: &[u8] = include_bytes!("data/battle-state.bin");


#[test]
fn test_ledge_jump_does_not_abort_overworld_movement() {
    let mut fixture = TestFixture::new(
        ROUTE1_STATE,
        Duration::from_secs(200),
        vec![
            PolicyStep::GrindUntilLevel { target_level: 100, on_map: Map::Route1 },
        ]
    );

    let mut battle_in_grass = false;

    loop {
        fixture.step();
        for event in fixture.agent.drain_events() {
            match &event {
                AgentEvent::OverworldActionAborted { destination, reason }
                    if *destination == MetaTile::Grass
                    && *reason == OverworldActionAbortedReason::Script =>
                {
                    panic!(
                        "Script abort on Grass navigation — ledge jump is being \
                         mistaken for a frozen script (bug not fixed)"
                    );
                }
                AgentEvent::BattleStarted=> {
                    battle_in_grass = true;
                }
                _ => {}
            }
        }
        if battle_in_grass { break; }
    }

    assert!(battle_in_grass, "agent should have successfully navigated into the grass and triggered a battle (if we got here, the ledge jump did not cause a Script abort)")
}

#[test]
fn test_debouncing() {
    pub const STATE: &[u8] = include_bytes!("data/oaks-lab-just-got-squirtle.bin");

    let mut fixture = TestFixture::new(
        STATE,
        Duration::from_secs(200),
        vec![PolicyStep::goto(Map::PalletTown)]
    );

    fixture.step_until_exhausted();
}


/// Route 1 has tall grass — a WalkInGrass action must be present and route to a Grass tile.
/// Indoor maps (no wGrassTile) must produce no WalkInGrass action.
#[test]
fn test_walk_in_grass_action() {
    {
        let mut fixture = TestFixture::new(ROUTE1_STATE, Duration::from_secs(10), vec![]);
        let state = fixture.game_state();

        assert!(
            state.map.meta_tiles.iter().any(|t| *t == MetaTile::Grass),
            "Route 1 map should contain Grass tiles"
        );

        let grass_action = state.map.actions().into_iter()
            .find(|a| a.tile == MetaTile::Grass);
        assert!(grass_action.is_some(), "Route 1 should have a WalkInGrass action");

        let action = grass_action.unwrap();
        assert_eq!(
            state.map.meta_tiles[action.destination.x as usize + action.destination.y as usize * state.map.width],
            MetaTile::Grass,
            "WalkInGrass destination tile must be Grass"
        );
    }

    // Indoor map (Red's House 1F): different tileset, no grass tile
    {
        const REDS_HOUSE_1F_STATE: &[u8] = include_bytes!("data/reds-house-1f-state.bin");
        let mut fixture = TestFixture::new(REDS_HOUSE_1F_STATE, Duration::from_secs(10), vec![]);
        let state = fixture.game_state();

        assert!(
            !state.map.meta_tiles.iter().any(|t| *t == MetaTile::Grass),
            "Red's House 1F should have no Grass tiles"
        );
        assert!(
            !state.map.actions().iter().any(|a| a.tile == MetaTile::Grass),
            "Red's House 1F should have no WalkInGrass action"
        );
    }
}

#[test]
fn test_route_1_ledge_routing() {
    let mut fixture = TestFixture::new(ROUTE1_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();
    let map = state.map;
    assert_eq!(map.map, Map::Route1);

    // Find a south-facing ledge tile that has walkable ground on both sides.
    let (lx, ly) = (0..map.height)
        .flat_map(|y| (0..map.width).map(move |x| (x, y)))
        .find(|&(x, y)| {
            y > 0 && y + 1 < map.height
                && matches!(map.meta_tiles[x + y * map.width], MetaTile::Jump(JumpDirection::South))
                && matches!(map.meta_tiles[x + (y - 1) * map.width], MetaTile::Empty)
                && matches!(map.meta_tiles[x + (y + 1) * map.width], MetaTile::Empty)
        })
        .expect("Route 1 should have south-facing ledges");

    let north_of_ledge = Point8 { x: lx as u8, y: (ly - 1) as u8 };
    let south_of_ledge = Point8 { x: lx as u8, y: (ly + 1) as u8 };

    // Returns the shortest route length to a connection at the given edge y-row.
    let shortest_to_connection = |pos: Point8, target_y: u8| -> Option<usize> {
        let mut m = map.clone();
        m.player_position = pos;
        m.actions().into_iter()
            .filter(|a| matches!(a.tile, MetaTile::Connection { .. }) && a.destination.y == target_y)
            .map(|a| a.route.len())
            .min()
    };

    let south_row = (map.height - 1) as u8; // Pallet Town connection row
    let north_row = 0u8;                     // Viridian City connection row

    // ── Southward: jump is used ──────────────────────────────────────────────
    // Pressing Down once from north-of-ledge jumps two tiles (over the ledge),
    // landing at south-of-ledge. So the south route is exactly 1 step longer
    // than starting from south-of-ledge.
    let north_to_south = shortest_to_connection(north_of_ledge, south_row)
        .expect("Pallet Town reachable from north of ledge via jump");
    let south_to_south = shortest_to_connection(south_of_ledge, south_row)
        .expect("Pallet Town reachable from south of ledge");

    assert!(
        north_to_south <= south_to_south + 1,
        "jumping south over ledge ({north_to_south} steps) should cost at most \
         1 more than starting just south of it ({south_to_south} steps)"
    );

    // ── Northward: ledge forces a detour ────────────────────────────────────
    // Ledges block northward movement. A player south of the ledge must navigate
    // around the entire ledge row, adding many more steps than from north of it.
    let north_to_north = shortest_to_connection(north_of_ledge, north_row)
        .expect("Viridian City reachable from north of ledge");
    let south_to_north = shortest_to_connection(south_of_ledge, north_row)
        .expect("Viridian City reachable from south of ledge via detour");

    assert!(
        south_to_north > north_to_north,
        "going north from south-of-ledge ({south_to_north} steps) must take more \
         steps than from north-of-ledge ({north_to_north} steps) — ledge forces detour"
    );

}

#[test]
fn test_pallet_town_actions() {
    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();
    let map = &state.map;

    let actions = map.actions();

    // Sanity-check that the test is actually exercising something.
    assert!(!map.warp_targets.is_empty(), "expected warp targets in Pallet Town");
    assert!(map.sprites.iter().any(|s| !s.hidden), "expected visible sprites in Pallet Town");

    // Every warp target must produce an action with a non-empty route.
    for &(warp_map, warp_pos) in &map.warp_targets {
        let action = actions.iter()
            .find(|a| matches!(a.tile, MetaTile::Warp { to_map, to_position } if to_map == warp_map && to_position == warp_pos));
        assert!(action.is_some(), "no action for warp to {warp_map} at {warp_pos:?}");
        assert!(!action.unwrap().route.is_empty(), "empty route to warp {warp_map} at {warp_pos:?}");
    }

    // Every visible (non-hidden) sprite must produce an action with a non-empty route.
    for sprite in map.sprites.iter().filter(|s| !s.hidden) {
        let action = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name));
        assert!(action.is_some(), "no action for sprite '{}'", sprite.name);
        assert!(!action.unwrap().route.is_empty(), "empty route to sprite '{}'", sprite.name);
    }

    // Route 1 is walkable from Pallet Town — must have a connection action.
    assert!(
        actions.iter().any(|a| matches!(a.tile, MetaTile::Connection { to_map: Map::Route1, .. })),
        "missing connection action for Route1"
    );

    // Route 21 is water-only from Pallet Town — no walkable Connection action expected.
    assert!(
        !actions.iter().any(|a| matches!(a.tile, MetaTile::Connection { to_map: Map::Route21, .. })),
        "unexpected walkable connection to Route21 (should be water-only)"
    );
}

/// When the ViridianCity PokeMART clerk's intro script is running the agent must
/// advance the conversation by pressing A — not try to navigate around the map.
#[test]
fn test_viridian_pokemart_script_advances_dialogue() {
    const POKEMART_STATE: &[u8] = include_bytes!("data/viridian-city-pokemart-during-script.bin");

    let mut fixture = TestFixture::new(
        POKEMART_STATE,
        Duration::from_secs(60),
        vec![PolicyStep::goto(Map::ViridianCity)],
    );

    // The save state has the clerk's text box already open.
    // The game mode must be TextBox or Script — never plain Overworld.
    {
        let mode = fixture.game_state().mode;
        assert!(
            matches!(mode, GameMode::TextBox | GameMode::Script),
            "Expected TextBox or Script mode in PokeMART save state, got {:?}", mode
        );
    }

    // Run the agent: it should press A to advance the dialogue and eventually
    // return to Overworld mode.
    fixture.step_until_exhausted();

    assert_eq!(
        fixture.game_state().mode,
        GameMode::Overworld,
        "agent should advance the PokeMART dialogue and return to Overworld"
    );
}

#[test]
fn test_pokemart_shopping() {
    const STATE: &[u8] = include_bytes!("data/viridian-city-pokemart-shopping.bin");
    let mut fixture = TestFixture::new(
        STATE,
        Duration::from_secs(60),
        vec![
            PolicyStep::BuyFromMart { map: Map::ViridianMart, item: BagItem::new(ItemId::PokeBall, 5) },
            PolicyStep::goto(Map::ViridianCity),
        ]
    );

    fixture.step_until_exhausted();

    let state = fixture.game_state();
    let pokeballs = state.bag.iter()
        .find(|i| i.id == ItemId::PokeBall)
        .expect("expected pokeballs to be in bag");
    assert_eq!(*pokeballs, BagItem::new(ItemId::PokeBall, 5), "expected to have bought 5 Poké Balls from the Mart");
}

/// A Cut bush on the path north of Viridian City blocks access to the Fisher.
/// Without the Cascade Badge and a Pokémon that knows Cut, the bush is an
/// impassable obstacle and no action to talk to the Fisher should be generated.
#[test]
fn test_cut_bush_blocks_fisher_without_cut() {
    const BUSH_STATE: &[u8] = include_bytes!("data/viridian-city-north-of-bush.bin");

    let mut fixture = TestFixture::new(BUSH_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();

    assert!(!state.can_use_cut, "player should not have Cut available at this point");

    let actions = state.map.actions();
    let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

    assert!(
        !actions.iter().any(|a| a.tile == MetaTile::Sprite("Fisher")),
        "Fisher must not be accessible without Cut; actions: {tiles:?}"
    );
}


/// Before the player receives the Pokédex from Oak, `has_pokedex` must be false
/// and both seen/owned sets must be empty.
#[test]
fn test_pokedex_empty_before_receiving_pokedex() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/start-of-game-state.bin"),
        Duration::from_secs(10),
        vec![],
    );
    let state = fixture.game_state();

    assert!(!state.has_pokedex, "player should not have the Pokédex before Oak's script");
    assert!(state.pokedex_owned.is_empty(), "wPokedexOwned should be all-zero at game start");
    assert!(state.pokedex_seen.is_empty(), "wPokedexSeen should be all-zero at game start");
}

/// Directly writes the EVENT_GOT_POKEDEX bit (bit 37 of wEventFlags) and checks
/// that `has_pokedex` flips accordingly — tests the bit-address logic independently
/// of any particular save state's history.
#[test]
fn test_has_pokedex_bit_toggling() {
    let mut fixture = TestFixture::new(ROUTE1_STATE, Duration::from_secs(10), vec![]);

    // EVENT_GOT_POKEDEX = bit 37 → byte 4, bit 5, mask 0x20 of wEventFlags
    let flag_addr = pokered_symbols::wEventFlags.address + 4;

    {
        let mmu = fixture.gb.core_mut().mmu_mut();
        let current = mmu.read(flag_addr);
        mmu.write(flag_addr, current & !0x20); // clear the bit
    }
    assert!(!fixture.game_state().has_pokedex, "should be false when bit 5 is clear");

    {
        let mmu = fixture.gb.core_mut().mmu_mut();
        let current = mmu.read(flag_addr);
        mmu.write(flag_addr, current | 0x20); // set the bit
    }
    assert!(fixture.game_state().has_pokedex, "should be true when bit 5 is set");
}

/// Directly writes known species bits into wPokedexOwned/wPokedexSeen and verifies
/// `read_pokedex_flags` decodes them to the correct `PokemonSpecies` values.
/// Bulbasaur=dex#1 (byte 0, bit 0), Charmander=dex#4 (byte 0, bit 3),
/// Squirtle=dex#7 (byte 0, bit 6).
#[test]
fn test_pokedex_flag_bit_decoding() {
    let mut fixture = TestFixture::new(ROUTE1_STATE, Duration::from_secs(10), vec![]);

    let owned_base = pokered_symbols::wPokedexOwned.address;
    // Write byte 0 with bits for Bulbasaur (#1=bit0), Charmander (#4=bit3), Squirtle (#7=bit6)
    // Bit index = dex_number - 1, LSB first.
    // Bulbasaur: bit 0 → mask 0x01
    // Charmander: bit 3 → mask 0x08
    // Squirtle: bit 6 → mask 0x40
    let test_byte: u8 = 0x01 | 0x08 | 0x40; // = 0x49

    fixture.gb.core_mut().mmu_mut().write(owned_base, test_byte);

    let state = fixture.game_state();

    assert!(state.pokedex_owned.contains(&PokemonSpecies::Bulbasaur), "Bulbasaur should be owned");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Charmander), "Charmander should be owned");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Squirtle), "Squirtle should be owned");
    assert_eq!(state.pokedex_owned.species().len(), 3, "exactly 3 species should be owned");
}

/// When a wild battle is in progress, the enemy species must appear in `pokedex_seen`.
/// The game sets the seen flag when the wild encounter begins.
#[test]
fn test_pokedex_seen_contains_battle_enemy() {
    let mut fixture = TestFixture::new(BATTLE_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();

    let battle = state.battle.expect("save state should be in a wild battle");
    assert_eq!(battle.battle_type, BattleType::Wild);

    assert!(
        state.pokedex_seen.contains(&battle.enemy.species),
        "wPokedexSeen should contain {:?} (the current battle enemy)",
        battle.enemy.species
    );
}

#[test]
fn test_caught_pokemon_nickname() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/viridian-forest.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::CatchPokemon { species: PokemonSpecies::Weedle, on_map: Map::ViridianForest },
            PolicyStep::goto(Map::ViridianForest),
        ]
    );
    fixture.step_until_exhausted();
    let state = fixture.game_state();
    let weedle = &state.pokemon[2];
    assert_eq!(weedle.species, PokemonSpecies::Weedle);
    assert_ne!(weedle.nickname.to_default_string(), "AAAAAAAAAA");
}


#[test]
fn can_navigate_to_pewter_city() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/viridian-city-pokemart-shopping.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::goto(Map::ViridianCity),
            PolicyStep::goto(Map::PewterCity),
        ]
    );

    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::PewterCity, "agent should have navigated to Pewter City");
}

/// Check every Mt Moon B1F/B2F/1F warp_event against how the tile is classified — a warp whose
/// position is classified Obstacle is DROPPED from the graph (map_metadata.rs), which would
/// silently disconnect sections.
#[test]
#[ignore]
fn check_mt_moon_warp_classification() {
    use crate::pokemon::map_metadata::MapMetadataReader;
    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(include_bytes!("data/mt-moon.bin")).unwrap();
    use crate::pokemon::map_metadata::{CurrentMap, PlayerFacingDirection};
    use std::sync::Arc;
    for map in [Map::MtMoon1F, Map::MtMoonB1F, Map::MtMoonB2F] {
        let meta = Arc::new(gb.core().mmu().read_map_metadata(map).unwrap());
        let dims = meta.dimensions();
        let w = dims.full_width();
        println!("=== {map} warp_events ({} total) ===", meta.warp_events.len());
        for warp in &meta.warp_events {
            let mx = warp.position.x as usize + dims.west_extra;
            let my = warp.position.y as usize + dims.north_extra;
            let tile = meta.meta_tiles_base.get(mx + my * w).copied();
            print!("  warp@{} -> {:?}  classified={:?}  {}",
                warp.position, warp.destination_map, tile,
                if matches!(tile, Some(MetaTile::Warp { .. })) { "OK" } else { "DROPPED!" });
            // Pure-maze reachability (no sprites) starting from this warp tile:
            let cm = CurrentMap { player_position: warp.position, player_direction: PlayerFacingDirection::Down, sprites: vec![], metadata: Arc::clone(&meta) };
            let mt = crate::pokemon::tile_map::MetaTileMap::new(&cm);
            let reach: Vec<_> = mt.all_reachable_warps_and_connections().into_iter()
                .filter_map(|(_, t)| match t {
                    MetaTile::Warp { to_map, to_position } => Some(format!("{to_map}@{to_position}")),
                    MetaTile::Connection { to_map, .. } => Some(format!("~{to_map}")),
                    _ => None,
                }).collect();
            println!("   reaches: {reach:?}");
        }
    }
}

/// Dump a Mt Moon B2F room's tilemap + reachable warps to understand the maze connectivity.
#[test]
#[ignore]
fn dump_b2f_room() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/mt-moon.bin"),
        Duration::from_mins(5),
        vec![
            PolicyStep::EnterMap { to_map: Map::MtMoonB1F, to_position: Some(Point8 { x: 5, y: 5 }) },
            PolicyStep::EnterMap { to_map: Map::MtMoonB2F, to_position: Some(Point8 { x: 21, y: 17 }) },
        ],
    );
    fixture.pimp_pokemon();
    fixture.step_until_exhausted();
    let state = fixture.game_state();
    println!("on {} @ {}", state.map.map, state.map.player_position);
    println!("visible sprites:");
    for s in state.map.sprites.iter().filter(|s| !s.hidden) {
        println!("   {} @ {}", s.name, s.position);
    }
    println!("reachable warps/connections (WITH live sprites):");
    for (p, t) in state.map.all_reachable_warps_and_connections() {
        println!("   tile@{p} -> {t:?}");
    }
    // Decisive: rebuild the same section with NO sprites and compare — if the far warps become
    // reachable, the divide is sprite-blocking (BFS over-block); if not, it's genuine walls.
    {
        use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
        use std::sync::Arc;
        let meta = fixture.gb.core().mmu().read_map_metadata(state.map.map).unwrap();
        let raw = Point8 {
            x: fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wXCoord),
            y: fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wYCoord),
        };
        let _ = raw;
        let meta = Arc::new(meta);
        let live_sprites: Vec<_> = state.map.sprites.iter().filter(|s| !s.hidden).cloned().collect();
        // Component structure: place the player at EACH B2F warp position and see which warps
        // share its walkable component (WITH live sprites). This reveals the true B2F graph.
        for entry in [Point8{x:25,y:9}, Point8{x:21,y:17}, Point8{x:15,y:27}, Point8{x:5,y:7}] {
            for (lbl, sp) in [("live", live_sprites.clone()), ("nosprite", vec![])] {
                let cm = CurrentMap { player_position: entry, player_direction: PlayerFacingDirection::Down, sprites: sp, metadata: Arc::clone(&meta) };
                let ms = crate::pokemon::tile_map::MetaTileMap::new(&cm);
                let reach: Vec<_> = ms.all_reachable_warps_and_connections().into_iter()
                    .filter_map(|(p, t)| if matches!(t, MetaTile::Warp { .. }) { Some(format!("{p}")) } else { None }).collect();
                println!("B2F entry {entry} [{lbl}] reaches warp-tiles: {reach:?}");
            }
        }
        // B1F no-sprite topology (read metadata directly; sprite blockers on B1F are only
        // Rockets we can defeat — no-sprite reveals the walkable component structure).
        let b1f_meta = Arc::new(fixture.gb.core().mmu().read_map_metadata(Map::MtMoonB1F).unwrap());
        for entry in [Point8{x:5,y:5}, Point8{x:17,y:11}, Point8{x:25,y:9}, Point8{x:25,y:15}, Point8{x:21,y:17}, Point8{x:13,y:27}, Point8{x:23,y:3}, Point8{x:27,y:3}] {
            let cm = CurrentMap { player_position: entry, player_direction: PlayerFacingDirection::Down, sprites: vec![], metadata: Arc::clone(&b1f_meta) };
            let ms = crate::pokemon::tile_map::MetaTileMap::new(&cm);
            let reach: Vec<_> = ms.all_reachable_warps_and_connections().into_iter()
                .filter_map(|(p, t)| if matches!(t, MetaTile::Warp { .. } | MetaTile::Connection { .. }) { Some(format!("{p}")) } else { None }).collect();
            println!("B1F entry {entry} [nosprite] reaches warp-tiles: {reach:?}");
        }
    }
    println!("{}", fixture.game_state().map);
}

/// Fast check that a single explicit `EnterMap` transition works: from the Mt Moon 1F start,
/// take the warp that lands at MtMoonB1F (5,5).
#[test]
#[ignore]
fn test_enter_map_single() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/mt-moon.bin"),
        Duration::from_mins(3),
        vec![PolicyStep::EnterMap { to_map: Map::MtMoonB1F, to_position: Some(Point8 { x: 5, y: 5 }) }],
    );
    fixture.pimp_pokemon();
    fixture.step_until_exhausted();
    let state = fixture.game_state();
    println!("ended on {} @ {}", state.map.map, state.map.player_position);
    assert_eq!(state.map.map, Map::MtMoonB1F, "should have taken the warp to B1F(5,5)");
    assert_eq!(state.map.player_position, Point8 { x: 5, y: 5 }, "should land at (5,5)");
}

#[test]
fn can_navigate_mt_moon() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/mt-moon.bin"),
        Duration::from_mins(40),
        mt_moon_traversal_steps(),
    );

    fixture.pimp_pokemon();
    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::CeruleanCity, "agent should have navigated to Cerulean City");
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
fn mt_moon_traversal_steps() -> Vec<PolicyStep> {
    vec![
        PolicyStep::EnterMap { to_map: Map::MtMoonB1F, to_position: Some(Point8 { x: 5, y: 5 }) },
        PolicyStep::EnterMap { to_map: Map::MtMoonB2F, to_position: Some(Point8 { x: 21, y: 17 }) },
        PolicyStep::CollectItem(crate::pokemon::map::MapSprite::MTMOONB2F_HELIX_FOSSIL),
        PolicyStep::EnterMap { to_map: Map::MtMoonB1F, to_position: Some(Point8 { x: 23, y: 3 }) },
        PolicyStep::EnterMap { to_map: Map::Route4, to_position: None },
        PolicyStep::EnterMap { to_map: Map::CeruleanCity, to_position: None },
    ]
}

#[test]
fn can_start_game() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/start-of-game-state.bin"),
        Duration::from_mins(90),
        PolicyStep::complete_game_steps(),
    );

    {
        let state = fixture.game_state();
        assert_eq!(state.map.map, Map::RedsHouse2F, "save state should be in RedsHouse2F");
        assert_eq!(state.pokemon.len(), 0, "player should have no pokemon before Oak's script");
    }

    fixture.step_until_exhausted();

    let state = fixture.game_state();
    for pokemon in state.pokemon.iter() {
        println!("{}: {} lv.{}", pokemon.species, pokemon.nickname, pokemon.level);
    }
    println!("badges: {:?}", state.badges);
    println!("map: {:?}", state.map.map);

    assert!(state.badges.contains(Badge::BoulderBadge), "should have the Boulder Badge");
    assert!(state.badges.contains(Badge::CascadeBadge), "should have the Cascade Badge");

    fixture.save_state_to_file().unwrap();
}

struct TestFixture {
    pub gb: GameBoy,
    map_cache: MapMetadataCache,
    pub agent: PokemonAgent,
    pub total_cycles: MachineCycles,
    pub max_cycles: MachineCycles,
    /// Cycles since the policy queue length last changed (stall detection).
    stall_cycles: MachineCycles,
    last_steps_remaining: Option<usize>,
    /// How long without queue progress before we declare a stall.
    stall_threshold: MachineCycles,
}

impl TestFixture {
    pub fn new(save_state: &[u8], max_game_time: Duration, policy_steps: Vec<PolicyStep>) -> Self {
        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(save_state).expect("failed to load save state");

        // The agent builds its world graph incrementally as it traverses.
        let policy = DeterministicPolicy::new(42, policy_steps);

        PokemonApi::new(&mut gb)
            .write_game_options(&GameOptions::default())
            .expect("failed to write game options");

        Self {
            gb,
            map_cache: MapMetadataCache::default(),
            total_cycles: MachineCycles::ZERO,
            max_cycles: MachineCycles::from_duration(max_game_time),
            stall_cycles: MachineCycles::ZERO,
            last_steps_remaining: None,
            // 10 minutes of game time without a queue step change → stall
            stall_threshold: MachineCycles::from_duration(Duration::from_secs(10 * 60)),
            agent: PokemonAgent::new(Box::new(policy)),
        }
    }

    pub fn step(&mut self) {
        let cycles = self.gb.run(AGENT_RESOLUTION);

        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        self.agent.update(&mut api, cycles).ok();

        self.total_cycles += cycles;

        // Stall detection: GrindUntilLevel and CatchPokemon legitimately sit on the
        // same step for long stretches — exempt them regardless of queue length.
        let steps = self.agent.policy_steps_remaining();
        let long_running = self.agent.policy_current_step_is_long_running();
        if steps != self.last_steps_remaining {
            self.last_steps_remaining = steps;
            self.stall_cycles = MachineCycles::ZERO;
        } else if !long_running && steps.map_or(false, |n| n > 1) {
            self.stall_cycles += cycles;
            if self.stall_cycles >= self.stall_threshold {
                self.save_failure_artifacts("test_stall");
                panic!("policy stalled — queue unchanged for {:?} of game time", self.stall_threshold);
            }
        }

        if self.total_cycles >= self.max_cycles {
            self.save_failure_artifacts("test_timeout");
            panic!("exceeded max cycles ({:?} game time)", self.max_cycles);
        }
    }

    fn save_failure_artifacts(&self, name: &str) {
        self.gb.save_state_to_file(&format!("{name}_state.bin")).ok();
        self.gb.save_screenshot_to_file(&format!("{name}_screenshot.png")).ok();
        println!("saved failure artifacts: {name}_state.bin, {name}_screenshot.png");
    }

    pub fn step_until_exhausted(&mut self) {
        while !self.agent.policy_exhausted() {
            self.step();
        }
    }


    pub fn pimp_pokemon(&mut self) {
        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        api.pimp_out_pokemon().expect("cannot pimp pokemon");
    }

    pub fn api(&mut self) -> PokemonApi<'_> {
        PokemonApi::new(&mut self.gb)
    }

    pub fn game_state(&mut self) -> GameState {
        self.api().game_state().unwrap()
    }

    pub fn save_state_to_file(&mut self) -> Result<(), String> {
        self.gb.save_state_to_file("pokemon-red.bin")
    }

    pub fn save_state_named(&mut self, path: &str) -> Result<(), String> {
        self.gb.save_state_to_file(path)
    }
}

/// Overlay a route (sequence of buttons starting from the player) onto the ASCII map.
/// Path tiles are marked with '*'; the player stays 'P'.
fn dump_map_with_route(map: &MetaTileMap, route: &[JoypadButton]) -> String {
    let mut grid: Vec<Vec<char>> = (0..map.height).map(|y| {
        (0..map.width).map(|x| {
            if y as u8 == map.player_position.y && x as u8 == map.player_position.x { 'P' }
            else {
                match map.meta_tiles[x + y * map.width] {
                    MetaTile::Empty => '_',
                    MetaTile::Obstacle => 'O',
                    MetaTile::Water => 'X',
                    MetaTile::Sprite(_) => 'S',
                    MetaTile::Warp { .. } => 'W',
                    MetaTile::Connection { .. } => 'C',
                    MetaTile::ConnectionWater(_) => '~',
                    MetaTile::Jump(JumpDirection::South) => 'v',
                    MetaTile::Jump(JumpDirection::West) => '<',
                    MetaTile::Jump(JumpDirection::East) => '>',
                    MetaTile::Counter => '=',
                    MetaTile::CutTree => 't',
                    MetaTile::Grass => 'g',
                }
            }
        }).collect()
    }).collect();

    let mut pos = map.player_position;
    for &btn in route {
        let (nx, ny) = match btn {
            JoypadButton::Up => (pos.x as i32, pos.y as i32 - 1),
            JoypadButton::Down => (pos.x as i32, pos.y as i32 + 1),
            JoypadButton::Left => (pos.x as i32 - 1, pos.y as i32),
            JoypadButton::Right => (pos.x as i32 + 1, pos.y as i32),
            _ => (pos.x as i32, pos.y as i32),
        };
        if nx < 0 || ny < 0 || nx as usize >= map.width || ny as usize >= map.height { break; }
        pos = Point8 { x: nx as u8, y: ny as u8 };
        if !(pos.x == map.player_position.x && pos.y == map.player_position.y) {
            if grid[pos.y as usize][pos.x as usize] == '_'
                || grid[pos.y as usize][pos.x as usize] == 'W' {
                grid[pos.y as usize][pos.x as usize] = '*';
            }
        }
    }

    let mut out = String::new();
    // column header
    out.push_str("   ");
    for x in 0..map.width { out.push_str(&format!("{}", x % 10)); }
    out.push('\n');
    for (y, row) in grid.iter().enumerate() {
        out.push_str(&format!("{y:2} "));
        for c in row { out.push(*c); }
        out.push('\n');
    }
    out
}

/// Print the raw bottom-left sub-tile id of each meta-tile in a window around `center`,
/// plus whether each id is in the tileset's passable (collision) set. This is the data
/// pokered's movement collision check actually uses.
fn dump_raw_tile_ids(fixture: &mut TestFixture, map: Map, center: Point8) {
    use crate::pokemon::map_metadata::{MapMetadata, MapMetadataReader};
    let meta = match fixture.gb.core().mmu().read_map_metadata(map) {
        Ok(m) => m,
        Err(e) => { println!("could not read metadata: {e}"); return; }
    };
    let dims = meta.dimensions();
    println!("--- raw bottom-left tile ids around {center} (P=passable per Cavern_Coll) ---");
    println!("Cavern_Coll passable set (hex): {:02x?}", {
        let mut v: Vec<u8> = meta.collision_tiles.iter().copied().collect(); v.sort(); v
    });
    let cx = center.x as i32;
    let cy = center.y as i32;
    // meta map width/height (no connection extras for caves, but account anyway)
    let mwidth = dims.meta_width;
    let mheight = dims.meta_height;
    print!("        ");
    for mx in (cx-4).max(0)..=(cx+4).min(mwidth as i32 - 1) { print!("  x{mx:<2} "); }
    println!();
    for my in (cy-6).max(0)..=(cy+6).min(mheight as i32 - 1) {
        print!("  y{my:<3}  ");
        for mx in (cx-4).max(0)..=(cx+4).min(mwidth as i32 - 1) {
            // bottom-left raw sub-tile of the meta-tile (the one pokered checks)
            let tx = mx as usize * MapMetadata::TILES_PER_META;
            let ty = my as usize * MapMetadata::TILES_PER_META + 1;
            let id = meta.tile_id(tx, ty);
            let pass = if meta.collision_tiles.contains(&id) { 'P' } else { '.' };
            let here = if mx == cx && my == cy { '@' } else { ' ' };
            print!("{here}{id:02x}{pass}  ");
        }
        println!();
    }
}

/// A discovery-only policy that physically explores a maze by taking warps/connections,
/// preferring destinations the incremental world graph has not observed yet. Used offline to
/// discover the correct `(to_map, to_position)` transition sequence through a maze (which is
/// then hard-coded as `EnterMap` steps). Wins battles with the pimped party.
///
/// `allowed` constrains exploration to a set of maps (e.g. the Mt Moon complex + the exit route)
/// so the frontier search stays inside the maze instead of wandering the whole overworld.
struct ExplorerPolicy {
    /// Recently *reached* landings, to rotate away from re-exploring the same visited section.
    recent: std::collections::VecDeque<(Map, Point8)>,
    /// The unvisited section the explorer has committed to reaching. Stays fixed across trainer-
    /// battle interruptions (like an `EnterMap` step) so navigation actually completes instead of
    /// thrashing between candidate warps.
    target: Option<(Map, Point8)>,
    allowed: Vec<Map>,
}

impl ExplorerPolicy {
    fn new(allowed: Vec<Map>) -> Self {
        Self { recent: std::collections::VecDeque::new(), target: None, allowed }
    }
}

impl crate::pokemon::policy::Policy for ExplorerPolicy {
    fn pick_overworld_action(&mut self, state: &GameState, wg: &crate::pokemon::world_graph::WorldGraph) -> Option<OverworldAction> {
        let actions = state.map.actions();
        let dest = |a: &OverworldAction| match a.tile {
            MetaTile::Warp { to_map, to_position } | MetaTile::Connection { to_map, to_position } => Some((to_map, to_position)),
            _ => None,
        };
        // Only consider transitions to whitelisted maps, to keep the search inside the maze.
        let transitions: Vec<OverworldAction> = actions.into_iter()
            .filter(|a| dest(a).is_some_and(|(m, _)| self.allowed.contains(&m)))
            .collect();
        if transitions.is_empty() { self.target = None; return None; }

        // Keep pursuing the committed target if it's still unobserved and reachable from here.
        if let Some(t) = self.target {
            if !wg.has_node(t.0, t.1) {
                if let Some(a) = transitions.iter().find(|a| dest(a) == Some(t)) {
                    return Some(a.clone());
                }
            }
            self.target = None; // reached, observed elsewhere, or no longer reachable here
            self.recent.push_back(t);
            while self.recent.len() > 12 { self.recent.pop_front(); }
        }

        // Commit to a new unobserved destination, preferring DEEPER maps so the search pushes
        // toward the far (east-exit) cluster instead of endlessly resurfacing to 1F. Among equal
        // depth prefer non-recent. Fall back to any non-recent transition, then anything.
        let depth = |m: Map| match m {
            Map::MtMoonB2F => 3, Map::MtMoonB1F => 2, Map::MtMoon1F => 1, _ => 0,
        };
        let key = |a: &OverworldAction| {
            let d = dest(a).unwrap();
            (depth(d.0), !self.recent.contains(&d)) // higher depth first, then non-recent
        };
        let choice = transitions.iter().filter(|a| { let d = dest(a).unwrap(); !wg.has_node(d.0, d.1) })
            .max_by_key(|a| key(a))
            .or_else(|| transitions.iter().filter(|a| !self.recent.contains(&dest(a).unwrap())).max_by_key(|a| depth(dest(a).unwrap().0)))
            .or_else(|| transitions.first())
            .cloned();
        self.target = choice.as_ref().and_then(|a| dest(a));
        choice
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<crate::pokemon::battle::BattleAction> {
        let actions = crate::pokemon::policy::battle_options(state)?;
        let bs = state.battle.as_ref()?;
        crate::pokemon::damage::pick_best_move(bs, &actions, false)
            .or_else(|| actions.iter().find(|a| matches!(a, crate::pokemon::battle::BattleAction::Fight { .. })).cloned())
            .or_else(|| actions.into_iter().next())
    }
}

/// Offline discovery: drive `ExplorerPolicy` through Mt Moon until the incrementally-observed
/// graph knows a path from the starting section to Cerulean, then print that path as ready-to-
/// paste `EnterMap` steps. Not bound by the 10-min gameplay budget.
#[test]
#[ignore]
fn discover_mt_moon_path() {
    use crate::pokemon::symbols::pokered_symbols as ps;
    let mut fixture = TestFixture::new(include_bytes!("data/mt-moon.bin"), Duration::from_mins(120), vec![]);
    fixture.pimp_pokemon();
    fixture.agent = PokemonAgent::new(Box::new(ExplorerPolicy::new(vec![
        Map::MtMoon1F, Map::MtMoonB1F, Map::MtMoonB2F, Map::Route4, Map::CeruleanCity,
    ])));

    let start_map = fixture.game_state().map.map;
    let start_entry = Point8 {
        x: fixture.gb.core().mmu().read_pointer(&ps::wXCoord),
        y: fixture.gb.core().mmu().read_pointer(&ps::wYCoord),
    };
    println!("start section: {start_map} @ {start_entry}");

    let mut last_node_count = 0usize;
    let mut steps_since_growth = 0u32;
    for i in 0..1_000_000u32 {
        fixture.step();
        if i % 100 != 0 { continue; }

        if let Some(path) = fixture.agent.world_graph().shortest_node_path_from(start_map, start_entry, Map::CeruleanCity) {
            println!("\n=== DISCOVERED path ({} hops) ===", path.len() - 1);
            for (m, p) in path.iter() { println!("  {m} @ {p}"); }
            println!("\n=== EnterMap steps ===");
            for (m, p) in path.iter().skip(1) {
                println!("Self::EnterMap {{ to_map: Map::{m:?}, to_position: Some(Point8 {{ x: {}, y: {} }}) }},", p.x, p.y);
            }
            return;
        }

        let nodes = fixture.agent.world_graph().nodes();
        if nodes.len() > last_node_count {
            last_node_count = nodes.len();
            steps_since_growth = 0;
            let gs = fixture.game_state();
            println!("[{i}] observed {} sections; now on {} @ {}", nodes.len(), gs.map.map, gs.map.player_position);
        } else {
            steps_since_growth += 100;
        }
        // No new section observed for a long while → exploration is stuck; dump the graph.
        if steps_since_growth >= 60_000 {
            println!("\n=== STUCK: no new sections observed. Observed graph ({} nodes): ===", nodes.len());
            let mut ns = nodes;
            ns.sort_by_key(|((m, p), _)| (*m as u16, p.x, p.y));
            for ((m, p), edges) in &ns {
                println!("  {m} @ {p}:");
                for e in edges { println!("      -> {} @ {} ({:?})", e.to.map, e.to.location, e.kind); }
            }
            let gs = fixture.game_state();
            println!("current: {} @ {} facing {:?}  player_tile={:?}",
                gs.map.map, gs.map.player_position, gs.map.player_direction, gs.map.player_tile());
            println!("actions from here:");
            for a in gs.map.actions() {
                println!("   dest={} tile={:?} route={:?}", a.destination, a.tile, a.route);
            }
            let pos = gs.map.player_position;
            dump_raw_tile_ids(&mut fixture, gs.map.map, pos);
            println!("{}", fixture.game_state().map);
            return;
        }
    }
    panic!("did not discover a path to Cerulean");
}

/// Instrumented navigation through Mt Moon: prints the ASCII tilemap, the chosen route,
/// and per-frame player position; detects the first frame where the player jams against a
/// tile and dumps the map + route overlay so the offending tile can be identified.
#[test]
#[ignore]
fn debug_mt_moon_navigation() {
    use std::collections::HashMap;
    let mut fixture = TestFixture::new(
        include_bytes!("data/mt-moon.bin"),
        Duration::from_mins(10),
        vec![PolicyStep::goto(Map::CeruleanCity)],
    );
    fixture.pimp_pokemon();

    {
        let state = fixture.game_state();
        println!("=== INITIAL MtMoon1F (map={}) player at {} facing {:?} ===",
            state.map.map, state.map.player_position, state.map.player_direction);
        println!("{}", state.map);
        println!("--- actions ---");
        for a in state.map.actions() {
            println!("  dest={:?} tile={:?} route_len={} route={:?}",
                a.destination, a.tile, a.route.len(), a.route);
        }
    }

    let mut last_pos = fixture.game_state().map.player_position;
    let mut last_map = fixture.game_state().map.map;
    let mut stuck_steps = 0u32;
    let mut total_steps = 0u32;
    let mut seen_maps: HashMap<Map, u32> = HashMap::new();
    let mut history: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    loop {
        fixture.step();
        total_steps += 1;
        {
            let s = fixture.game_state();
            let held = fixture.api().read_joypad_state();
            let wco = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wCurOpponent);
            let wib = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wIsInBattle);
            let l = held.is_button_pressed(JoypadButton::Left);
            let a = held.is_button_pressed(JoypadButton::A);
            history.push_back(format!("step {total_steps}: mode={:?} pos={} agent={} L={} A={} wCurOpp={:#04x} wIsInBattle={:#04x}",
                s.mode, s.map.player_position, fixture.agent.state_debug(), l, a, wco, wib));
            while history.len() > 400 { history.pop_front(); }
        }
        if fixture.agent.policy_exhausted() {
            println!("*** policy exhausted — reached target! map={}", fixture.game_state().map.map);
            break;
        }

        let state = fixture.game_state();
        let pos = state.map.player_position;
        let map = state.map.map;
        *seen_maps.entry(map).or_default() += 1;

        if map != last_map {
            println!("[step {total_steps}] MAP CHANGE {last_map} -> {map}, player now at {pos} facing {:?}", state.map.player_direction);
            println!("{}", state.map);
            println!("--- actions on {map} ---");
            for a in state.map.actions() {
                println!("  dest={:?} tile={:?} route_len={} route={:?}",
                    a.destination, a.tile, a.route.len(), a.route);
            }
            last_map = map;
            last_pos = pos;
            stuck_steps = 0;
            continue;
        }

        if pos == last_pos {
            stuck_steps += 1;
        } else {
            stuck_steps = 0;
            last_pos = pos;
        }

        // Pure-navigation stuck (no trainer engagement: wco==0) on B1F/B2F => dump and stop.
        let wco_now = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wCurOpponent);
        if stuck_steps == 800 && wco_now == 0 && map != Map::MtMoon1F {
            println!("\n*** STUCK at step {total_steps}: player jammed at {pos} facing {:?} on {map} for {stuck_steps} steps ***",
                state.map.player_direction);
            println!("agent committed state: {}", fixture.agent.state_debug());
            println!("game mode: {:?}", state.mode);
            println!("--- last {} frames history ---", history.len());
            for h in &history { println!("  {h}"); }
            fixture.gb.save_screenshot_to_file("debug_mt_moon_stuck.png").ok();
            fixture.save_state_named("debug_mtmoon_b1f_stuck.bin").ok();
            println!("player_tile = {:?}", state.map.player_tile());

            // Decisive experiment: run the emulator with NO agent input and watch whether the
            // trainer battle resolves on its own (i.e. whether the agent's input is the culprit).
            {
                let read = |fixture: &mut TestFixture, sym: &crate::pokemon::symbols::DmgPointer| -> u8 {
                    fixture.gb.core().mmu().read_pointer(sym)
                };
                println!("--- RAM flags at stuck ---");
                println!("  wCurOpponent={:#04x} wIsInBattle={:#04x} wJoyIgnore={:#04x} wStatusFlags5={:#04x}",
                    read(&mut fixture, &crate::pokemon::symbols::pokered_symbols::wCurOpponent),
                    read(&mut fixture, &crate::pokemon::symbols::pokered_symbols::wIsInBattle),
                    read(&mut fixture, &crate::pokemon::symbols::pokered_symbols::wJoyIgnore),
                    read(&mut fixture, &crate::pokemon::symbols::pokered_symbols::wStatusFlags5));
                println!("--- running 400 frames: release all ONCE, then NO input (pure wait) ---");
                fixture.api().release_all_buttons();
                for i in 0..400 {
                    fixture.gb.run(AGENT_RESOLUTION);
                    if i % 20 == 0 || i == 399 {
                        let m = fixture.api().game_mode();
                        let wib = read(&mut fixture, &crate::pokemon::symbols::pokered_symbols::wIsInBattle);
                        let wco = read(&mut fixture, &crate::pokemon::symbols::pokered_symbols::wCurOpponent);
                        println!("  +{i:3}: mode={m:?} wIsInBattle={wib:#04x} wCurOpponent={wco:#04x}");
                    }
                }
                fixture.gb.save_screenshot_to_file("debug_mt_moon_noinput.png").ok();
            }

            println!("nearby sprites:");
            for s in state.map.sprites.iter().filter(|s| !s.hidden) {
                println!("  {} at {} (hidden={})", s.name, s.position, s.hidden);
            }
            dump_raw_tile_ids(&mut fixture, map, pos);
            // find which action the agent is pursuing and overlay its route
            println!("{}", state.map);
            println!("--- actions (with route overlay for warp toward Cerulean) ---");
            for a in state.map.actions() {
                println!("  dest={:?} tile={:?} route={:?}", a.destination, a.tile, a.route);
                if matches!(a.tile, MetaTile::Warp { .. } | MetaTile::Connection { .. }) {
                    println!("{}", dump_map_with_route(&state.map, &a.route));
                }
            }
            break;
        }

        if total_steps > 30_000 {
            println!("*** gave up after {total_steps} steps, last map={map} pos={pos}");
            break;
        }
    }

    println!("maps visited: {seen_maps:?}");
}



