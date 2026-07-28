
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::pokemon::policy::{DeterministicPolicy, PolicyStep};
use crate::pokemon::*;
use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason, PokemonAgent, AGENT_RESOLUTION};
use crate::pokemon::battle::BattleType;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::tile::JumpDirection;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::map::MapSprite;
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
            PolicyStep::GrindUntilLevel { target_level: 100, on_map: Map::Route1, slot: 0 },
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

/// The `complete_game_steps` mart sequence — an explicit `Interact(Clerk)` opens the shop, THEN
/// `BuyFromMart`. Guards the verify-and-retry buy flow (the purchase must register in the bag).
#[test]
fn test_mart_interact_then_buy() {
    const STATE: &[u8] = include_bytes!("data/viridian-city-pokemart-shopping.bin");
    let mut fixture = TestFixture::new(
        STATE,
        Duration::from_secs(120),
        vec![
            PolicyStep::Interact(MapSprite::VIRIDIANMART_CLERK),
            PolicyStep::BuyFromMart { map: Map::ViridianMart, item: BagItem::new(ItemId::PokeBall, 7) },
            PolicyStep::goto(Map::ViridianCity),
        ]
    );
    fixture.step_until_exhausted();
    let state = fixture.game_state();
    let pokeballs = state.bag.iter().find(|i| i.id == ItemId::PokeBall);
    assert_eq!(pokeballs, Some(&BagItem::new(ItemId::PokeBall, 7)), "expected 7 Poké Balls after Interact+Buy");
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
    // Explicit forward navigation (Viridian City → Viridian Forest → Pewter City), the same
    // single-hop `EnterMap` chain `complete_game_steps` uses. The abstract `goto` form this test
    // used previously needed the deleted pre-built world graph.
    let mut fixture = TestFixture::new(
        include_bytes!("data/viridian-city-pokemart-shopping.bin"),
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

/// Diagnostic: for water maps (Pallet, Route 21, Cinnabar), dump the tile map and the
/// warps/connections reachable from a land start with `can_surf` OFF vs ON. Confirms the BFS now
/// routes across water (a water connection appears only when surf is enabled).
#[test]
#[ignore]
fn probe_surf_reachability() {
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use std::sync::Arc;
    let gb = GameBoy::dmg(roms::POKERED);
    for (map, start) in [
        (Map::PalletTown,     Point8 { x: 12, y: 11 }), // Oak's Lab doorstep (land, south of town)
        (Map::Route21,        Point8 { x: 8,  y: 3  }), // top land strip
        (Map::CinnabarIsland, Point8 { x: 18, y: 3  }), // island land near the gym
    ] {
        let meta = Arc::new(gb.core().mmu().read_map_metadata(map).unwrap());
        for can_surf in [false, true] {
            let cm = CurrentMap { player_position: start, player_direction: PlayerFacingDirection::Down, sprites: vec![], metadata: Arc::clone(&meta), closed_doors: vec![], card_key_locked: false };
            let mut mt = crate::pokemon::tile_map::MetaTileMap::new(&cm);
            mt.can_surf = can_surf;
            let reach: Vec<_> = mt.all_reachable_warps_and_connections().into_iter()
                .map(|(p, t)| match t {
                    MetaTile::Warp { to_map, .. } => format!("W:{to_map}@{p}"),
                    MetaTile::Connection { to_map, .. } => format!("C:{to_map}@{p}"),
                    MetaTile::ConnectionWater(to_map) => format!("~C:{to_map}@{p}"),
                    _ => format!("?@{p}"),
                }).collect();
            if !can_surf { println!("\n=== {map} start={start} ===\n{}", mt); }
            println!("  can_surf={can_surf}: reaches {reach:?}");
        }
    }
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
            let cm = CurrentMap { player_position: warp.position, player_direction: PlayerFacingDirection::Down, sprites: vec![], metadata: Arc::clone(&meta), closed_doors: vec![], card_key_locked: false };
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
                let cm = CurrentMap { player_position: entry, player_direction: PlayerFacingDirection::Down, sprites: sp, metadata: Arc::clone(&meta), closed_doors: vec![], card_key_locked: false };
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
            let cm = CurrentMap { player_position: entry, player_direction: PlayerFacingDirection::Down, sprites: vec![], metadata: Arc::clone(&b1f_meta), closed_doors: vec![], card_key_locked: false };
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
    PolicyStep::mt_moon_traversal()
}

/// The full end-to-end playthrough — the single source of truth for how far the agent can play. From a
/// fresh `RedsHouse2F` save it plays legitimately (button input only, starting from the **lone starter**)
/// and earns **all 8 gym badges**: Boulder → Cascade → Thunder → Rainbow → (Silph Scope → Poké Flute →
/// Snorlax) → Soul → (Safari Surf/Strength → Eevee→Vaporeon+Surf) → Silph Co (Card Key → rival → Giovanni
/// → liberation) → Marsh → surf to Cinnabar → Pokémon Mansion Secret Key → Volcano → back to the Viridian
/// Gym for **Earth** (Giovanni). The only Pokémon caught is the free Celadon **Eevee**, evolved to
/// **Vaporeon** (its Surf counters the Silph rival's Alakazam / Blaine's Fire / Giovanni's Ground, and
/// ferries the party across Route 21). It emulates every frame, so even in `--release` it takes ~15 min
/// of wall-clock; it is therefore **opt-in**, running only with the `slow-tests` feature
/// (`cargo test --release --features slow-tests full_playthrough`). The per-leg focused tests above (each
/// seeded from a saved fixture) cover the same ground quickly.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "full multi-hour playthrough; run with --features slow-tests")]
fn full_playthrough() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/start-of-game-state.bin"),
        Duration::from_mins(800),
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
    println!("money: {}  bag: {:?}", state.money, state.bag.iter().collect::<Vec<_>>());

    fixture.save_state_to_file().unwrap();
    // Snapshot the lone-starter post-Soul state (Fuchsia) for fast iteration on the later legs.
    fixture.gb.save_state_to_file("/tmp/claude-1000/-home-alex-projects-gb/ba4d63f0-ce24-4d5d-9463-5238001f3ce6/scratchpad/post-soul-lone.bin").ok();
    // NB: do NOT resave post-cascade.bin here — the run now continues through the Rainbow Badge, so
    // the final state is no longer post-Cascade. The committed post-cascade.bin fixture (a viable
    // 2-mon, Tackle-keeping party) is regenerated by `can_start_game` only implicitly via the
    // per-leg tests that snapshot their own fixtures.

    assert!(state.badges.contains(Badge::BoulderBadge), "should have the Boulder Badge");
    assert!(state.badges.contains(Badge::CascadeBadge), "should have the Cascade Badge");
    assert!(state.badges.contains(Badge::ThunderBadge), "should have the Thunder Badge");
    assert!(state.badges.contains(Badge::RainbowBadge), "should have the Rainbow Badge");
    // Post-Rainbow: Silph Scope (Rocket Hideout) → Poké Flute → Snorlax → Soul Badge (Koga).
    assert!(state.bag.iter().any(|b| b.id == ItemId::SilphScope), "should have the Silph Scope");
    assert!(state.bag.iter().any(|b| b.id == ItemId::PokeFlute), "should have the Poké Flute");
    assert!(state.badges.contains(Badge::SoulBadge), "should have the Soul Badge");
    // Post-Soul: Safari HMs → Vaporeon → Silph (Marsh) → Cinnabar Mansion → Volcano → Viridian (Earth).
    assert!(state.bag.iter().any(|b| b.id == ItemId::Hm03Surf), "should have HM03 Surf");
    assert!(state.badges.contains(Badge::MarshBadge), "should have the Marsh Badge");
    assert!(state.badges.contains(Badge::VolcanoBadge), "should have the Volcano Badge");
    assert!(state.badges.contains(Badge::EarthBadge), "should have the Earth Badge (all 8 gym badges)");

    // Lone starter + the one free Eevee (evolved to Vaporeon for Surf); nothing else is caught (a weak
    // extra mon blocks the black-out recovery that clears the early attrition dungeons).
    assert!(state.pokemon.len() >= 2, "party should have the starter + Vaporeon");
    assert!(state.pokemon.iter().any(|p| format!("{:?}", p.species) == "Vaporeon"), "should have a Vaporeon");

    // Post-Earth: Victory Road 1F — caught a Machop HM-slave, taught it Strength, solved the boulder puzzle
    // (real push onto the (17,13) switch) and climbed to VR2F. The full VR2F/VR3F puzzle is validated
    // separately by `can_solve_victory_road_2f_3f` (chaining it here is PP-marginal for this team).
    assert!(state.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| format!("{:?}", m.name) == "Strength")),
        "a party member should know Strength (the Machop HM-slave)");
    assert_eq!(state.map.map, Map::VictoryRoad2F, "should have solved VR1F and climbed to Victory Road 2F");
}

/// From a post-Cascade save state, do the full Bill → SS Ticket → Route 5 → Vermilion leg.
///
/// Route 5 is unreachable from the Cerulean Pokécenter terrace directly (one-way south ledges split
/// the city; verified ROM-faithful). The real path is the **trashed-house bridge**, which only opens
/// after meeting Bill (the `CERULEANCITY_GUARD2` guard at raw (27,12) clears): enter the trashed
/// house from the main terrace, take its back door to land at Cerulean (27,9) — which IS in the
/// Route-5-reaching terrace — then walk onto Route 5. So: Nugget Bridge → Bill (SS Ticket) → return →
/// trashed-house bridge → Route 5 → Underground Path → Route 6 → Vermilion. See the plan doc Stage 3.
#[test]
fn can_reach_vermilion() {
    // Exactly the leg folded into `complete_game_steps` (Bill/SS-Ticket → trashed-house bridge →
    // Vermilion), so this test and the full playthrough stay in lockstep.
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-cascade.bin"),
        Duration::from_mins(40),
        PolicyStep::cerulean_to_vermilion_steps(),
    );

    fixture.step_until_exhausted();
    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::VermilionCity, "agent should reach Vermilion City");
    assert!(state.bag.contains(&ItemId::SSTicket), "should still hold the SS Ticket at Vermilion");
    // Snapshot at Vermilion with the SS Ticket, so the S.S. Anne tests can start here without
    // replaying the whole Bill leg each iteration.
    fixture.save_state_named("src/pokemon/data/at-vermilion.bin").unwrap();
}

/// Navigate from the post-Cascade Cerulean Pokécenter to Bill's House (Sea Cottage) via the
/// Nugget Bridge (Route 24 — the Cerulean rival battle triggers en route) and Route 25, then run
/// the SS-Ticket sub-sequence: talk to Bill's Pokémon (YES → it enters the cell separator) → use
/// the PC (sets the used-separator event, Bill appears) → talk to Bill → receive the SS Ticket.
#[test]
fn can_get_ss_ticket() {
    let mut steps = vec![
        PolicyStep::enter(Map::CeruleanCity),
        PolicyStep::enter(Map::Route24),
        PolicyStep::enter(Map::Route25),
        PolicyStep::enter(Map::BillsHouse),
    ];
    steps.extend(PolicyStep::bill_ss_ticket_steps());
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-cascade.bin"),
        Duration::from_mins(30),
        steps,
    );
    fixture.step_until_exhausted();
    // `Interact` pops its step when the walk is *issued*, so when the queue empties the agent is
    // still mid-walk to Bill and hasn't talked to him yet. The in-flight OverworldMovement finishes
    // on its own (independent of the now-empty queue), so keep stepping until the ticket appears.
    for _ in 0..15_000 {
        if fixture.game_state().bag.contains(&ItemId::SSTicket) { break; }
        fixture.step();
    }
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    println!("bag: {:?}", s.bag.iter().collect::<Vec<_>>());
    assert!(s.bag.contains(&ItemId::SSTicket), "should have obtained the SS Ticket from Bill");
}

/// Board the S.S. Anne: from Vermilion City (holding the SS Ticket) → Vermilion Dock → S.S. Anne 1F.
/// The dock sailor checks the ticket; with it in the bag, boarding should succeed.
#[test]
fn can_board_ss_anne() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-vermilion.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::enter(Map::VermilionDock),
            PolicyStep::enter(Map::SSAnne1F),
        ],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::SSAnne1F, "agent should board the S.S. Anne (1F)");
}


/// Board the S.S. Anne, defeat all 16 cabin/bow trainers (leveling the party), beat the rival, get
/// HM01 Cut from the captain, and disembark back to Vermilion. Each floor is a heal → board → sweep
/// → disembark cycle (no Pokémon Center on the ship).
#[test]
fn can_clear_ss_anne() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-vermilion.bin"),
        Duration::from_mins(90),
        PolicyStep::ss_anne_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended {} @ {} party_lv={:?} bag={:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>(), s.bag.iter().collect::<Vec<_>>());
    assert!(s.bag.contains(&ItemId::Hm01Cut), "should have HM01 Cut after clearing the S.S. Anne");
    assert_eq!(s.map.map, Map::VermilionCity, "should have disembarked back to Vermilion City");
    // Snapshot post-S.S.-Anne (HM01 in bag, party ~lv32) for the next leg (teach Cut → Lt. Surge).
    fixture.save_state_named("src/pokemon/data/post-ss-anne.bin").unwrap();
}

/// Teach HM01 Cut to the starter via the bag (START → ITEM → HM01 → USE → choose Pokémon), from the
/// post-S.S.-Anne save. The starter knows 4 moves, so the move-replace menu is exercised too.
#[test]
fn can_teach_cut() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-ss-anne.bin"),
        Duration::from_mins(5),
        vec![PolicyStep::TeachMove { item: ItemId::Hm01Cut, target_slot: 0 }],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("party[0] moves: {:?}", s.pokemon.get(0).map(|p| p.moves));
    assert!(s.can_use_cut, "starter should know Cut (can_use_cut true) after TeachMove");
    // Snapshot with Cut taught (at Vermilion) for the next leg (cut the gym tree → Lt. Surge).
    fixture.save_state_named("src/pokemon/data/post-teach-cut.bin").unwrap();
}

/// With Cut taught, cut the tree blocking the Vermilion Gym and enter the gym — all via the real UI.
/// The `CuttingTree` state drives START→POKéMON→mon→CUT with plain button mashing (cursor-navigate to
/// each target index, then confirm with A). The agent's `MetaTileMap` is decoded from static ROM (so
/// it still shows the felled tree), so the agent records what it cut and treats it as `Empty` for
/// routing (`observe_state`).
#[test]
fn can_cut_gym_tree() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-teach-cut.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::CutTree { map: Map::VermilionCity },
            PolicyStep::enter(Map::VermilionGym),
        ],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::VermilionGym, "should cut the gym tree and enter the Vermilion Gym");
    fixture.save_state_named("src/pokemon/data/in-vermilion-gym.bin").unwrap();
}

/// Solve the Vermilion Gym two-switch trash-can puzzle via the real UI: the agent reads which cans
/// hold the switches (`GameState::trash_cans`, from RAM), walks to each and presses A, unlocking the
/// door to Lt. Surge. Junior trainers that engage en route are fought and beaten normally.
#[test]
fn can_solve_gym_trash_cans() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/in-vermilion-gym.bin"),
        Duration::from_mins(5),
        vec![PolicyStep::SolveTrashCans],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    let tc = s.trash_cans.clone().expect("trash-can puzzle state in the gym");
    println!("player@{} first_opened={} second_opened={}", s.map.player_position, tc.first_opened, tc.second_opened);
    assert!(tc.second_opened, "both trash-can switches should be flipped (door to Lt. Surge unlocked)");
    fixture.save_state_named("src/pokemon/data/gym-trash-solved.bin").unwrap();
}

/// After the Thunder Badge, the agent can leave the gym and head east toward Route 11 / Diglett's
/// Cave — the first leg of the route to Celadon. Verifies the post-badge fixture is clean and
/// navigable. (The Rock Tunnel path to Celadon needs no Flash: the agent routes from RAM tile
/// collision, not the darkened screen.)
#[test]
fn can_leave_vermilion_after_surge() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-thunder-badge.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::enter(Map::VermilionCity),
            // Exiting the gym drops the player into an enclosure sealed by the Cut tree (which regrew
            // on re-entering the map) — cut it again to reach the rest of the city and the east edge.
            PolicyStep::CutTree { map: Map::VermilionCity },
            PolicyStep::enter(Map::Route11),
        ],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::Route11, "should leave the gym and reach Route 11");
}

/// Full Vermilion Gym clear via the real UI: solve the trash-can puzzle to unlock the door, then walk
/// up to Lt. Surge and battle him for the Thunder Badge.
#[test]
fn can_beat_lt_surge() {
    let mut steps = vec![PolicyStep::SolveTrashCans];
    // `Interact` pops the moment it issues the walk to Surge; a junior trainer engaging by line of
    // sight en route interrupts that walk, and the popped step can't resume — so retry enough times
    // to survive the interrupts (each retry re-routes; a beaten trainer won't re-engage).
    steps.extend(std::iter::repeat(PolicyStep::Interact(MapSprite::VERMILIONGYM_LT_SURGE)).take(8));
    let mut fixture = TestFixture::new(
        include_bytes!("data/in-vermilion-gym.bin"),
        Duration::from_mins(20),
        steps,
    );
    // Step (step() enforces the cycle budget) until the badge is earned rather than to exhaustion,
    // since the retry Interacts keep talking to Surge after the win.
    while !fixture.game_state().badges.contains(Badge::ThunderBadge) {
        fixture.step();
    }
    let s = fixture.game_state();
    println!("badges={:?} @ {} on {}", s.badges, s.map.player_position, s.map.map);
    assert!(s.badges.contains(Badge::ThunderBadge), "should earn the Thunder Badge from Lt. Surge");
    fixture.save_state_named("src/pokemon/data/post-thunder-badge.bin").unwrap();
}

/// Integrated Thunder-Badge leg exactly as folded into `complete_game_steps`: from post-S.S.-Anne
/// (HM01 Cut in the bag, in Vermilion City) run `thunder_badge_steps()` — teach Cut, cut the gym
/// tree, solve the trash-can puzzle, beat Lt. Surge — and confirm the badge. Keeps the helper and the
/// full playthrough in lockstep.
#[test]
fn can_get_thunder_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-ss-anne.bin"),
        Duration::from_mins(20),
        PolicyStep::thunder_badge_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("badges={:?} on {}", s.badges, s.map.map);
    assert!(s.badges.contains(Badge::ThunderBadge), "thunder_badge_steps should earn the Thunder Badge");
}

/// Leg 1 of the Rainbow-Badge push: from the post-Thunder-Badge state (inside the Vermilion Gym),
/// exit the gym, re-cut the enclosure tree, heal, and trek back to Cerulean City via the Underground
/// Path (Route 6 → Route 5) — Saffron's Route 6 gate is guard-blocked so the tunnel is the only way
/// north. Snapshots `back-in-cerulean.bin` for the Rock Tunnel leg.
#[test]
fn can_return_to_cerulean() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-thunder-badge.bin"),
        Duration::from_mins(30),
        PolicyStep::back_to_cerulean_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::CeruleanCity, "should trek back to Cerulean City");
    fixture.save_state_named("src/pokemon/data/back-in-cerulean.bin").unwrap();
}

/// Probe: from the main Cerulean terrace (post-Thunder), can we reach Route 9 → Route 10 via the
/// trashed-house bridge? The main Pokécenter terrace only connects to Route 4 (west) and Route 24
/// (north); Route 9 (east) is on a separate terrace, reached — like Route 5 — through the trashed
/// house's back door at (27,9).
#[test]
fn can_reach_route10() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/back-in-cerulean.bin"),
        Duration::from_mins(20),
        vec![
            PolicyStep::enter(Map::CeruleanTrashedHouse),
            PolicyStep::enter_at(Map::CeruleanCity, 27, 9),
            PolicyStep::enter(Map::Route9),
            // Route 9 boxes the west-entry pocket behind a Cut tree at (5,8); cut it to cross east.
            PolicyStep::CutTree { map: Map::Route9 },
            PolicyStep::enter(Map::Route10),
        ],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::Route10, "should reach Route 10 via the trashed-house bridge → Route 9");
    fixture.save_state_named("src/pokemon/data/at-route10.bin").unwrap();
}

/// Incremental Rock Tunnel probe: drive a hand-built warp chain (edit `chain` below), then dump the
/// current region's reachable warps so the next hop can be chosen. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn probe_rock_tunnel() {
    let chain = vec![
        PolicyStep::enter_at(Map::RockTunnel1F, 15, 3),   // Route 10 north → 1F region {Route10, B1F(33,25)}
        PolicyStep::enter_at(Map::RockTunnelB1F, 33, 25), // → B1F region {1F(5,3), 1F(37,3)}
        PolicyStep::enter_at(Map::RockTunnel1F, 5, 3),    // → 1F region {B1F(27,3), B1F(23,11)}
        PolicyStep::enter_at(Map::RockTunnelB1F, 23, 11), // → B1F region {1F(17,11), 1F(37,17)}
        PolicyStep::enter_at(Map::RockTunnel1F, 37, 17),  // → 1F region ??? (probe this — south exit?)
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-route10.bin"), Duration::from_mins(30), chain);
    fixture.pimp_pokemon();
    // Step until the queue is issued and we've settled on the last map.
    for _ in 0..120_000 {
        fixture.step();
        if fixture.agent.policy_steps_remaining().map_or(true, |n| n == 0) { break; }
    }
    for _ in 0..3_000 { fixture.step(); }
    let s = fixture.game_state();
    println!("=== probe landed on {} @ {} facing {:?} ===", s.map.map, s.map.player_position, s.map.player_direction);
    println!("{}", s.map);
    for a in s.map.actions() {
        println!("   dest={} tile={:?}", a.destination, a.tile);
    }
}

/// Leg 2: from the main Cerulean terrace, cross to Lavender Town via the trashed-house bridge, the
/// Route 9 Cut tree, and the **Rock Tunnel** warp maze (Route 10 → RockTunnel1F/B1F → Route 10 south).
/// No Flash needed. Snapshots `at-lavender.bin` for the Underground-Path leg to Celadon.
#[test]
fn can_reach_lavender() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/back-in-cerulean.bin"),
        Duration::from_mins(60),
        PolicyStep::cerulean_to_lavender_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {} party_lv={:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>());
    assert_eq!(s.map.map, Map::LavenderTown, "should cross Rock Tunnel to Lavender Town");
    fixture.save_state_named("src/pokemon/data/at-lavender.bin").unwrap();
}

/// Leg 3: Lavender Town → Celadon City via the Route 7–8 Underground Path (bypassing the drink-gated
/// Saffron gates). Snapshots `at-celadon.bin` for the Rainbow-Badge leg.
#[test]
fn can_reach_celadon() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-lavender.bin"),
        Duration::from_mins(30),
        PolicyStep::lavender_to_celadon_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::CeladonCity, "should reach Celadon City via the Underground Path");
    fixture.save_state_named("src/pokemon/data/at-celadon.bin").unwrap();
}

/// Micro-benchmark: raw emulation throughput (game-time emulated per wall-second) from a mid-game
/// fixture, driving the real agent. Reports the speedup factor over realtime. Run with
/// `--ignored --nocapture`.
#[test]
#[ignore]
fn bench_emulation_throughput() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-celadon.bin"),
        Duration::from_mins(60),
        PolicyStep::celadon_rainbow_steps(),
    );
    // (a) Raw emulation only.
    {
        let game_secs = 30.0;
        let target = MachineCycles::from_duration(Duration::from_secs_f64(game_secs));
        let start = std::time::Instant::now();
        let mut emulated = MachineCycles::ZERO;
        while emulated < target {
            emulated += fixture.gb.run(crate::pokemon::agent::AGENT_RESOLUTION);
        }
        let wall = start.elapsed().as_secs_f64();
        println!("[raw run only]     {game_secs}s game in {wall:.3}s → {:.1}x realtime", game_secs / wall);
    }
    // (b) Full agent step (observe + policy + input synthesis) — the real playthrough cost.
    {
        let n = 3000u32;
        let before = fixture.total_cycles;
        let start = std::time::Instant::now();
        for _ in 0..n { fixture.step(); }
        let wall = start.elapsed().as_secs_f64();
        let game_secs = (fixture.total_cycles.m_cycles() - before.m_cycles()) as f64 / 1_048_576.0;
        println!("[full agent.step]  {game_secs:.1}s game in {wall:.3}s → {:.1}x realtime ({} steps)",
            game_secs / wall, n);
    }
}

#[test]
#[ignore]
fn probe_reach_elevator() {
    // From B4F with the Lift Key, go back up to B1F and into the elevator room (tests reverse spinner
    // nav + reaching the B1F elevator warp), then dump the elevator room so we can drive the floor menu.
    let mut fixture = TestFixture::new(include_bytes!("data/rocket-hideout-lift-key.bin"), Duration::from_mins(30), vec![
        PolicyStep::enter(Map::RocketHideoutB3F),
        PolicyStep::enter(Map::RocketHideoutB2F),
        PolicyStep::enter(Map::RocketHideoutB1F),
        PolicyStep::enter(Map::RocketHideoutElevator),
    ]);
    fixture.pimp_pokemon();
    let mut last = fixture.game_state().map.map;
    let mut last_pos = fixture.game_state().map.player_position;
    let mut stuck = 0;
    for _ in 0..400_000 {
        fixture.step();
        let m = fixture.game_state().map.map;
        if m != last { println!("--> {m} @ {}", fixture.game_state().map.player_position); last = m; }
        if m == Map::RocketHideoutElevator { break; }
        let p = fixture.game_state().map.player_position;
        if p == last_pos { stuck += 1; } else { stuck = 0; last_pos = p; }
        if stuck > 6000 { println!("STUCK at {m} @ {p} facing {:?}", fixture.game_state().map.player_direction); break; }
    }
    let s = fixture.game_state();
    println!("ended on {} @ {} facing {:?}", s.map.map, s.map.player_position, s.map.player_direction);
    println!("{}", s.map);
    println!("tile in front: {:?}", s.map.tile_in_front());
    for sp in s.map.sprites.iter() {
        println!("   sprite {} @ {} hidden={} on_screen={}", sp.name, sp.position, sp.hidden, sp.on_screen);
    }
    let w = s.map.width;
    for y in 13..20usize { for x in 22..28usize {
        print!("({x},{y})=0x{:02x}:{:?}  ", s.map.raw_tile_ids[x + y*w], s.map.meta_tiles[x + y*w]);
    } println!(); }
    for a in s.map.actions() {
        if matches!(a.tile, MetaTile::Warp { to_map, .. } if to_map == Map::RocketHideoutElevator) {
            println!("   ELEVATOR dest={} route={:?}", a.destination, a.route);
        }
    }
}

#[test]
#[ignore]
fn probe_route12() {
    // Enter Route 12 from Lavender and dump the map + reachable warps/connections to see whether the
    // Route-12 Gate blocks the road down to the Snorlax and how to route through it.
    let mut fixture = TestFixture::new(include_bytes!("data/post-poke-flute.bin"), Duration::from_mins(10), vec![
        PolicyStep::enter(Map::LavenderTown),
        PolicyStep::enter(Map::Route12),
    ]);
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("on {} @ {}", s.map.map, s.map.player_position);
    println!("{}", s.map);
    println!("reachable warps/connections:");
    for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..} | MetaTile::Connection{..})) {
        println!("   dest={} tile={:?}", a.destination, a.tile);
    }
    println!("snorlax sprite:");
    for sp in s.map.sprites.iter().filter(|sp| sp.name == "Snorlax") {
        println!("   Snorlax @ {} hidden={}", sp.position, sp.hidden);
    }
}

#[test]
#[ignore]
fn probe_6f_rare_candy() {
    // The 6F stall: the Rare Candy ball at (6,8) blocks the only chokepoint to the 7F-stairs region.
    // Verify that collecting it opens the path and lets the agent reach 7F (fighting the ghost Marowak).
    let bytes = std::fs::read("test_stall_state.bin").expect("stall state");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(15), vec![
        PolicyStep::CollectItem(MapSprite::POKEMONTOWER6F_RARE_CANDY),
        PolicyStep::enter(Map::PokemonTower7F),
    ]);
    let mut last = fixture.game_state().map.map;
    for _ in 0..600_000 {
        fixture.step();
        let m = fixture.game_state().map.map;
        if m != last { println!("--> {m} @ {}", fixture.game_state().map.player_position); last = m; }
        if m == Map::PokemonTower7F { println!("REACHED 7F"); break; }
    }
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
}

const SCRATCH: &str = "/tmp/claude-1000/-home-alex-projects-gb/ba4d63f0-ce24-4d5d-9463-5238001f3ce6/scratchpad";

/// Runs a chained step list from an input fixture, logging map transitions, and saves the result.
fn run_chain(input: &str, output: &str, minutes: u64, max_steps: usize, steps: Vec<PolicyStep>) {
    let bytes = std::fs::read(format!("{SCRATCH}/{input}")).unwrap_or_else(|_| panic!("no fixture {input}"));
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(minutes), steps);
    let mut last = (Map::PalletTown, Point8 { x: 255, y: 255 });
    let mut last_levels = String::new();
    for i in 0..max_steps {
        fixture.step();
        if i % 100 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  {i}: {} @ {}", s.map.map, s.map.player_position); }
            }
        }
        if i % 5000 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let lv: String = s.pokemon.iter().map(|p| format!("{:?}{}", p.species, p.level)).collect::<Vec<_>>().join(",");
                if lv != last_levels { last_levels = lv; println!("  {i}: party [{last_levels}]"); }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> done at {i}"); break; }
    }
    let s = fixture.game_state();
    print!("final: {} @ {} badges={:?} party=[", s.map.map, s.map.player_position, s.badges);
    for p in s.pokemon.iter() { print!("{:?} lv{}; ", p.species, p.level); }
    println!("] HM03={} CardKey={}", s.bag.iter().any(|b| b.id == ItemId::Hm03Surf), s.bag.iter().any(|b| b.id == ItemId::CardKey));
    fixture.gb.save_state_to_file(&format!("{SCRATCH}/{output}")).ok();
}

/// Segment 1: Safari (Surf+Strength HMs) + Saffron entry (buys Super Potions) from the lone post-Soul fixture.
#[test]
#[ignore]
fn probe_post_soul_chain() {
    let mut steps = vec![];
    steps.extend(PolicyStep::safari_zone_surf_steps());
    steps.extend(PolicyStep::safari_zone_strength_steps());
    steps.extend(PolicyStep::saffron_entry_steps());
    run_chain("post-soul-lone.bin", "post-saffron-lone.bin", 60, 1_500_000, steps);
}

/// Segment 1b: Eevee → Vaporeon → teach Surf → grind to lv32, from the post-Saffron fixture.
#[test]
#[ignore]
fn probe_eevee_grind() {
    run_chain("post-saffron-lone.bin", "post-vaporeon-lone.bin", 600, 15_000_000, PolicyStep::eevee_vaporeon_surf_steps());
}

/// Segment 2: from the post-Vaporeon fixture (at Saffron), Card Key + Giovanni + liberation + Marsh Badge.
#[test]
#[ignore]
fn probe_seg2() {
    let mut steps = vec![];
    steps.extend(PolicyStep::silph_co_card_key_steps());
    steps.extend(PolicyStep::silph_giovanni_steps());
    steps.extend(PolicyStep::marsh_badge_steps());
    run_chain("post-vaporeon-lone.bin", "post-marsh-lone.bin", 120, 3_000_000, steps);
}

/// Segment 3: from post-Marsh, Surf to Cinnabar + Pokémon Mansion Secret Key + Volcano Badge (Blaine).
#[test]
#[ignore]
fn probe_seg3() {
    let mut steps = vec![];
    steps.extend(PolicyStep::saffron_to_cinnabar_steps());
    steps.extend(PolicyStep::mansion_secret_key_steps());
    steps.extend(PolicyStep::volcano_badge_steps());
    run_chain("post-marsh-lone.bin", "post-volcano-lone.bin", 120, 3_000_000, steps);
}

/// Segment 4: from post-Volcano, Cinnabar → Viridian Gym → Earth Badge (Giovanni) — the 8th badge.
/// Exercises the new Viridian Gym spinner-tile table.
#[test]
#[ignore]
fn probe_seg4() {
    run_chain("post-volcano-lone.bin", "post-earth-lone.bin", 60, 2_000_000, PolicyStep::earth_badge_steps());
}

/// Probe: from post-Earth, teach Strength then navigate Viridian → Route 22 → Route 23 → Victory Road,
/// to find exactly where the boulder puzzles / navigation block. Logs party + every map transition.
#[test]
#[ignore]
fn probe_victory_road_route() {
    use crate::pokemon::map::MapSprite as MS;
    let bytes = std::fs::read(format!("{SCRATCH}/post-earth-lone.bin")).unwrap_or_else(|_|
        std::fs::read("src/pokemon/data/post-earth-badge.bin").expect("no post-earth fixture"));
    let steps = vec![
        PolicyStep::enter(Map::ViridianCity),          // out of the gym
        PolicyStep::enter(Map::ViridianPokecenter),    // heal before the rival
        PolicyStep::Interact(MS::VIRIDIANPOKECENTER_NURSE),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::MovePokemonToFront { slot: 1 },    // Venusaur leads the rival (Alakazam nemesis)
        PolicyStep::enter(Map::Route22),
        PolicyStep::enter(Map::Route22Gate),           // walk west → rival ambush → gate to Route 23
        // The gate warp is dynamic (→Route23 only when the player's Y<4). Interacting with the guard
        // walks to his front tile (5,2), which is both the badge-check trigger ("Go right ahead!" with
        // the Boulder Badge) and Y<4 — so the north warps then read as →Route23.
        PolicyStep::Interact(MS::ROUTE22GATE_GUARD),
        PolicyStep::enter(Map::Route23),
        PolicyStep::goto(Map::VictoryRoad1F),
        // NOTE: past VR1F needs the Strength boulder-push mechanic (unbuilt) — VR1F gates the 2F stairs
        // behind a cross-room Sokoban (boulders at (5,15)/(2,10), switch at (17,13)).
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(60), steps);
    { let s = fixture.game_state(); print!("start {} @ {} party=[", s.map.map, s.map.player_position);
      for p in s.pokemon.iter() { print!("{:?}{} ", p.species, p.level); } println!("]");
      print!("bag: "); for it in s.bag.iter() { print!("{:?}x{} ", it.id, it.quantity); } println!(); }
    let mut last = (Map::PalletTown, Point8 { x: 255, y: 255 });
    for i in 0..2_000_000 {
        fixture.step();
        if i % 100 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  {i}: {} @ {}", s.map.map, s.map.player_position); }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> done at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    // Milestone: the team beats the Route 22 rival (Venusaur-lead + Hyper Potions), crosses the dynamic
    // Route-22 badge gate, and reaches Victory Road 1F. (Progressing past VR1F needs the Strength
    // boulder-push mechanic, not yet built.)
    assert_eq!(s.map.map, Map::VictoryRoad1F, "should reach Victory Road 1F from post-Earth");
    let _ = MS::ROUTE23_GUARD1;
}

// Isolated validation of the `victory_road_1f_steps()` leg folded into `full_playthrough`: from post-Earth,
// catch the Machop HM-slave, teach Strength, solve the VR1F boulder puzzle, and climb to VR2F. (The full
// VR2F/VR3F puzzle is validated separately by `can_solve_victory_road_2f_3f`.)
#[test]
#[ignore]
fn can_solve_victory_road_1f() {
    let bytes = std::fs::read("src/pokemon/data/post-earth-badge.bin").expect("no post-earth fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(120), PolicyStep::victory_road_1f_steps(2));
    let mut last = (Map::PalletTown, Point8 { x: 255, y: 255 });
    for i in 0..8_000_000 {
        fixture.step();
        if i % 500 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  {i}: {} @ {}", s.map.map, s.map.player_position); }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> done at {i}"); break; }
    }
    let s = fixture.game_state();
    let has_strength = s.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| format!("{:?}", m.name) == "Strength"));
    println!("final: {} @ {}  strength={has_strength}", s.map.map, s.map.player_position);
    assert!(has_strength, "should have caught+taught a Strength HM-slave");
    assert_eq!(s.map.map, Map::VictoryRoad2F, "should solve VR1F and climb to VR2F");
}

/// From post-Earth, reach Victory Road 1F, catch a wild Machop (HM-slave) with the Master Ball, and teach
/// it Strength (HM04). Saves `vr1f-strength.bin` for iterating on the boulder-push mechanic.
#[test]
#[ignore]
fn probe_vr_catch_machop() {
    use crate::pokemon::map::MapSprite as MS;
    use crate::pokemon::species::PokemonSpecies;
    let bytes = std::fs::read("src/pokemon/data/post-earth-badge.bin").expect("no post-earth fixture");
    let steps = vec![
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::MovePokemonToFront { slot: 1 },
        PolicyStep::enter(Map::Route22),
        PolicyStep::enter(Map::Route22Gate),
        PolicyStep::Interact(MS::ROUTE22GATE_GUARD),
        PolicyStep::enter(Map::Route23),
        PolicyStep::goto(Map::VictoryRoad1F),
        // Catch a Machop (learns Strength) as the boulder HM-slave — Master Ball, thrown immediately.
        PolicyStep::CatchPokemon { species: PokemonSpecies::Machop, on_map: Map::VictoryRoad1F },
        // Machop appends at slot 2; teach it Strength.
        PolicyStep::TeachMove { item: ItemId::Hm04Strength, target_slot: 2 },
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(60), steps);
    let mut last = (Map::PalletTown, Point8 { x: 255, y: 255 });
    for i in 0..2_000_000 {
        fixture.step();
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  {i}: {} @ {}", s.map.map, s.map.player_position); }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> done at {i}"); break; }
    }
    let s = fixture.game_state();
    print!("final party: ");
    for p in s.pokemon.iter() {
        let strength = p.moves.iter().flatten().any(|m| format!("{:?}", m.name) == "Strength");
        print!("{:?}{}(str={strength}) ", p.species, p.level);
    }
    println!();
    let has_str = s.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| format!("{:?}", m.name) == "Strength"));
    if has_str {
        fixture.gb.save_state_to_file(&format!("{SCRATCH}/vr1f-strength.bin")).ok();
        println!(">> saved vr1f-strength.bin");
    }
    assert!(has_str, "a party member should know Strength after the catch+teach");
}

// Re-verify VR1F solvability from a GUARANTEED-fresh entry (not the vr1f-strength fixture), to rule out
// fixture corruption. Dumps boulders + solver result + (1,1) reachability.
#[test]
#[ignore]
fn probe_vr1f_fresh() {
    use crate::pokemon::map::MapSprite as MS;
    let bytes = std::fs::read("src/pokemon/data/post-earth-badge.bin").expect("no post-earth fixture");
    let steps = vec![
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::MovePokemonToFront { slot: 1 },
        PolicyStep::enter(Map::Route22),
        PolicyStep::enter(Map::Route22Gate),
        PolicyStep::Interact(MS::ROUTE22GATE_GUARD),
        PolicyStep::enter(Map::Route23),
        PolicyStep::goto(Map::VictoryRoad1F),
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(40), steps);
    let mut reached = 0;
    for _ in 0..3_000_000 {
        fixture.step();
        if fixture.game_state().map.map == Map::VictoryRoad1F { reached += 1; if reached > 300 { break; } }
        if fixture.agent.policy_exhausted() { break; }
    }
    let s = fixture.game_state();
    println!("map {} @ {}", s.map.map, s.map.player_position);
    if s.map.map != Map::VictoryRoad1F { println!("!! did not reach VR1F"); return; }
    for sp in s.map.sprites.iter().filter(|sp| sp.name.starts_with("Boulder")) {
        println!("  {} @ {}", sp.name, sp.position);
    }
    let reach = s.map.reachable_tiles();
    println!("(1,1) reachable = {}  region size = {}", reach.contains(&Point8 { x: 1, y: 1 }), reach.len());
    match s.map.solve_boulder_push(Point8 { x: 17, y: 13 }) {
        Some(p) => println!(">> FRESH VR1F SOLVABLE: {} pushes", p.len()),
        None => println!(">> FRESH VR1F: no boulder reaches switch (17,13) — same as fixture"),
    }
}

// Render the full VR1F collision map with region coloring so the whole puzzle is visible at once.
#[test]
#[ignore]
fn probe_vr1f_render() {
    use image::{ImageBuffer, Rgb};
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    let reach = s.map.reachable_tiles();
    let (w, h) = (s.map.width, s.map.height);
    let scale = 20u32;
    let mut img = ImageBuffer::new(w as u32 * scale, h as u32 * scale);
    for y in 0..h { for x in 0..w {
        let p = Point8 { x: x as u8, y: y as u8 };
        let boulder = s.map.sprites.iter().any(|sp| sp.name.starts_with("Boulder") && sp.position == p);
        let color = if p == (Point8 { x: 17, y: 13 }) { Rgb([0u8, 220, 0]) }        // switch = green
            else if boulder { Rgb([220, 40, 40]) }                                    // boulder = red
            else if p == (Point8 { x: 1, y: 1 }) { Rgb([40, 80, 255]) }               // 2F ladder = blue
            else { match s.map.tile_at(p) {
                MetaTile::Obstacle => Rgb([30, 30, 30]),                               // wall = dark
                MetaTile::Warp { .. } => Rgb([255, 200, 0]),                           // warp = yellow
                MetaTile::Empty | MetaTile::Grass if reach.contains(&p) => Rgb([210, 210, 210]), // reachable floor
                MetaTile::Empty | MetaTile::Grass => Rgb([120, 120, 170]),            // unreachable floor = purple
                _ => Rgb([90, 60, 60]),
            } };
        for dy in 0..scale { for dx in 0..scale {
            let (px, py) = (x as u32 * scale + dx, y as u32 * scale + dy);
            // grid lines
            let c = if dx == 0 || dy == 0 { Rgb([0, 0, 0]) } else { color };
            img.put_pixel(px, py, c);
        }}
    }}
    img.save(format!("{SCRATCH}/vr1f_map.png")).ok();
    println!("saved vr1f_map.png ({}x{} tiles)", w, h);
}

// Impact test: if the player were at Route23(14,31) [where VR1F's exit warp_id 3 points], can they reach
// VR2F / Indigo Plateau (i.e., is the boulder puzzle skippable via the exit leapfrog)?
#[test]
#[ignore]
fn probe_route23_from_14_31() {
    use crate::pokemon::symbols::pokered_symbols as ps;
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let steps = vec![PolicyStep::enter(Map::Route23)];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(4), steps);
    for _ in 0..400_000 { fixture.step(); if fixture.agent.policy_exhausted() { break; } }
    println!("exited to {}", fixture.game_state().map.player_position);
    // Force the player to Route23 raw (14,31) — where warp_id 3 points.
    { let mmu = fixture.gb.core_mut().mmu_mut();
      mmu.write(ps::wXCoord.address, 14); mmu.write(ps::wYCoord.address, 31); }
    fixture.gb.run(MachineCycles::from_m(200_000));
    let s = fixture.game_state();
    println!("forced to {} on {}", s.map.player_position, s.map.map);
    let reach = s.map.reachable_tiles();
    let miny = reach.iter().map(|p| p.y).min().unwrap_or(255);
    println!("reachable region {} tiles, northmost y = {miny} (0 = Indigo edge)", reach.len());
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        let p = Point8 { x: (i % s.map.width) as u8, y: (i / s.map.width) as u8 };
        match t {
            MetaTile::Connection { to_map, .. } => println!("  conn {p} -> {:?} reachable={}", to_map, reach.contains(&p)),
            MetaTile::Warp { to_map, .. } if format!("{:?}", to_map).contains("Victory") =>
                println!("  warp {p} -> {:?} reachable={}", to_map, reach.contains(&p)),
            _ => {}
        }
    }
}

// Where does exiting VR1F via (8,17)/(9,17) REALLY land on Route 23? The warp is LAST_MAP,3 and Route23
// warp #3 is (14,31) [past the barrier]. Test the real landing (manually, no agent warp logic).
#[test]
#[ignore]
fn probe_vr1f_exit_landing() {
    use crate::joypad::JoypadButton as JB;
    use crate::pokemon::symbols::pokered_symbols as ps;
    let _ = JB::Down;
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let steps = vec![PolicyStep::enter(Map::Route23)];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(4), steps);
    fixture.gb.run(MachineCycles::from_m(4_000));
    let last_map = fixture.gb.core().mmu().read_pointer(&ps::wLastMap);
    println!("wLastMap = {last_map} (Route23 = {})", Map::Route23 as u8);
    // Poll wDestinationWarpID + coords across the whole exit to catch the warp resolution.
    let mut prev = (255u8, 255u8, 255u8, Map::PalletTown as u8);
    for _ in 0..30_000 {
        fixture.step();
        let mmu = fixture.gb.core().mmu();
        let cur = (mmu.read_pointer(&ps::wCurMap), mmu.read(ps::wDestinationWarpID.address),
                   mmu.read_pointer(&ps::wXCoord), mmu.read_pointer(&ps::wYCoord));
        let key = (cur.1, cur.2, cur.3, cur.0);
        if key != prev {
            println!("  map={} destWarpID={} coords=({},{})", cur.0, cur.1, cur.2, cur.3);
            prev = key;
        }
        if cur.0 == Map::Route23 as u8 && fixture.agent.policy_exhausted() {
            println!("   (Route23 warp2=(4,31)=entry; warp3=(14,31)=VR2F side)");
            return;
        }
    }
    println!("did not exit");
}

// Definitive boulder-model test: flood-fill boulder(5,15)'s REAL reachable set in-game (push it every
// which way with save/restore) and compare to the solver's prediction. If the game moves it somewhere the
// solver says is impossible, the boulder model has a bug.
#[test]
#[ignore]
fn probe_vr1f_boulder_flood() {
    use crate::joypad::JoypadButton as JB;
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(30), vec![]);
    // Arm Strength.
    { let a = crate::pokemon::symbols::pokered_symbols::wStatusFlags1.address;
      let mmu = fixture.gb.core_mut().mmu_mut(); let v = mmu.read(a); mmu.write(a, v | 0x01); }
    fixture.gb.run(MachineCycles::from_m(4_000));
    let boulder_at = |fx: &mut TestFixture, b: Point8| -> bool {
        let s = { PokemonApi::with_cache(&mut fx.gb, &mut fx.map_cache).game_state().unwrap() };
        s.map.sprites.iter().any(|sp| sp.name.starts_with("Boulder") && sp.position == b)
    };
    let stra = crate::pokemon::symbols::pokered_symbols::wStatusFlags1.address;
    let set_str = |fx: &mut TestFixture, on: bool| {
        let mmu = fx.gb.core_mut().mmu_mut(); let v = mmu.read(stra);
        mmu.write(stra, if on { v | 0x01 } else { v & !0x01 });
    };
    // Navigate with Strength OFF so we never accidentally push a boulder while routing past it.
    let walk_to = |fx: &mut TestFixture, target: Point8| -> bool {
        for _ in 0..80 {
            set_str(fx, false);
            let s = fx.game_state();
            if s.map.player_position == target { return true; }
            let Some(route) = s.map.route_to(target) else { return false; };
            let Some(&dir) = route.first() else { return false; };
            fx.gb.core_mut().mmu_mut().joypad_mut().press_button(dir);
            fx.gb.run(MachineCycles::from_m(240_000));
            fx.gb.core_mut().mmu_mut().joypad_mut().release_button(dir);
            fx.gb.run(MachineCycles::from_m(50_000));
        }
        false
    };
    let dirs = [(0i32, -1i32, JB::Up), (0, 1, JB::Down), (-1, 0, JB::Left), (1, 0, JB::Right)];
    // BFS over boulder positions, saving the game state at each.
    let baseline = fixture.gb.save_state().unwrap();
    let mut states: std::collections::HashMap<Point8, Vec<u8>> = std::collections::HashMap::new();
    states.insert(Point8 { x: 5, y: 15 }, baseline);
    let mut visited: std::collections::HashSet<Point8> = states.keys().copied().collect();
    let mut queue = vec![Point8 { x: 5, y: 15 }];
    while let Some(b) = queue.pop() {
        for &(dx, dy, btn) in &dirs {
            let (fx2, fy) = (b.x as i32 - dx, b.y as i32 - dy); // push-from = b - dir
            let (nx, ny) = (b.x as i32 + dx, b.y as i32 + dy); // dest = b + dir
            if fx2 < 0 || fy < 0 || nx < 0 || ny < 0 { continue; }
            let from = Point8 { x: fx2 as u8, y: fy as u8 };
            let dest = Point8 { x: nx as u8, y: ny as u8 };
            if visited.contains(&dest) { continue; }
            fixture.gb.load_state(&states[&b]).unwrap();
            fixture.gb.run(MachineCycles::from_m(2_000));
            if !walk_to(&mut fixture, from) { continue; }
            set_str(&mut fixture, true); // arm Strength for the push
            for _ in 0..6 {
                fixture.gb.core_mut().mmu_mut().joypad_mut().press_button(btn);
                fixture.gb.run(MachineCycles::from_m(260_000));
                fixture.gb.core_mut().mmu_mut().joypad_mut().release_button(btn);
                fixture.gb.run(MachineCycles::from_m(60_000));
                if boulder_at(&mut fixture, dest) { break; }
            }
            if boulder_at(&mut fixture, dest) {
                visited.insert(dest);
                states.insert(dest, fixture.gb.save_state().unwrap());
                queue.push(dest);
            }
        }
    }
    let mut cells: Vec<_> = visited.iter().map(|p| (p.x, p.y)).collect();
    cells.sort_by_key(|&(x, y)| (y, x));
    println!(">> REAL boulder(5,15) reachable ({} cells): {:?}", cells.len(), cells);
    println!(">> reached switch (17,13)? {}", visited.contains(&Point8 { x: 17, y: 13 }));
}

// Route 23 barrier test: from vr1f-strength, exit VR1F to Route 23 (lands north of the water near the
// VR1F entrance), then MANUALLY drive around the y=32 barrier / toward the VR2F entrance (14,32) to see
// if the game lets the player reach it or go north — where my model blocks.
#[test]
#[ignore]
fn probe_route23_barrier() {
    use crate::joypad::JoypadButton as JB;
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let steps = vec![PolicyStep::enter(Map::Route23)];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(5), steps);
    for i in 0..400_000 { fixture.step(); if fixture.agent.policy_exhausted() { println!(">> on Route23 at {i}"); break; } }
    let s0 = fixture.game_state();
    println!("start @ {} map {}", s0.map.player_position, s0.map.map);
    fixture.gb.save_screenshot_to_file(&format!("{SCRATCH}/route23_start.png")).ok();
    let drive = |fx: &mut TestFixture, dir: JB, tag: &str| {
        fx.gb.core_mut().mmu_mut().joypad_mut().press_button(dir);
        fx.gb.run(MachineCycles::from_m(260_000));
        fx.gb.core_mut().mmu_mut().joypad_mut().release_button(dir);
        fx.gb.run(MachineCycles::from_m(60_000));
        let s = fx.game_state();
        println!("  {tag} {dir:?}: @ {} map {}", s.map.player_position, s.map.map);
    };
    // Go east first (around the VR1F warp that's directly north), then probe north from the east side.
    for _ in 0..16 { drive(&mut fixture, JB::Right, "E"); }
    for _ in 0..8 { drive(&mut fixture, JB::Up, "N"); }
    for _ in 0..6 { drive(&mut fixture, JB::Down, "S"); }
    for _ in 0..8 { drive(&mut fixture, JB::Left, "W"); }
    fixture.gb.save_screenshot_to_file(&format!("{SCRATCH}/route23_end.png")).ok();
}

// Exhaustive boundary test: for every reachable tile bordering an Empty tile my model calls UNreachable,
// manually try to cross in-game. If the game lets the player cross, my classification/tile-pair logic is
// wrong there — printing the exact tile pair that diverges.
#[test]
#[ignore]
fn probe_vr1f_boundary_audit() {
    use crate::joypad::JoypadButton as JB;
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(6), vec![]);
    fixture.gb.run(MachineCycles::from_m(4_000));
    let dir_of = |b: JB| match b { JB::Up => (0i32, -1i32), JB::Down => (0, 1), JB::Left => (-1, 0), JB::Right => (1, 0), _ => (0, 0) };
    let mut walk_to = |fx: &mut TestFixture, target: Point8| -> bool {
        for _ in 0..160 {
            let s = fx.game_state();
            if s.map.player_position == target { return true; }
            let Some(route) = s.map.route_to(target) else { return false; };
            let Some(&dir) = route.first() else { return false; };
            fx.gb.core_mut().mmu_mut().joypad_mut().press_button(dir);
            fx.gb.run(MachineCycles::from_m(260_000));
            fx.gb.core_mut().mmu_mut().joypad_mut().release_button(dir);
            fx.gb.run(MachineCycles::from_m(60_000));
        }
        false
    };
    // Enumerate boundary crossings from my model.
    let (reach, w, raw, metas) = {
        let s = fixture.game_state();
        (s.map.reachable_tiles(), s.map.width, s.map.raw_tile_ids.clone(), s.map.meta_tiles.clone())
    };
    let dirs = [(0i32, -1i32, JB::Up), (0, 1, JB::Down), (-1, 0, JB::Left), (1, 0, JB::Right)];
    let mut crossings = vec![];
    for &from in &reach {
        for &(dx, dy, btn) in &dirs {
            let (nx, ny) = (from.x as i32 + dx, from.y as i32 + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 { continue; }
            let to = Point8 { x: nx as u8, y: ny as u8 };
            let idx = to.x as usize + to.y as usize * w;
            if idx >= metas.len() { continue; }
            // Test ALL unreachable neighbours except warps/connections (stepping on those changes map).
            if !reach.contains(&to)
                && !matches!(metas[idx], MetaTile::Warp { .. } | MetaTile::Connection { .. } | MetaTile::ConnectionWater(_)) {
                crossings.push((from, btn, to));
            }
        }
    }
    println!("testing {} boundary crossings", crossings.len());
    let mut bugs = 0;
    for (from, btn, to) in crossings {
        if !walk_to(&mut fixture, from) { continue; }
        fixture.gb.core_mut().mmu_mut().joypad_mut().press_button(btn);
        fixture.gb.run(MachineCycles::from_m(260_000));
        fixture.gb.core_mut().mmu_mut().joypad_mut().release_button(btn);
        fixture.gb.run(MachineCycles::from_m(60_000));
        let now = fixture.game_state().map.player_position;
        if now == to {
            let fi = from.x as usize + from.y as usize * w;
            let ti = to.x as usize + to.y as usize * w;
            println!("  !! BUG: game ALLOWS {from}->{to} ({btn:?}) — my model blocks it. tiles ${:02x}->${:02x}", raw[fi], raw[ti]);
            bugs += 1;
            // walk back so subsequent tests start from a known region
        }
    }
    println!(">> {bugs} divergences found");
}

// Exploration: walk the player around VR1F and screenshot the real map at key positions, to compare
// the actual walls/floors against my MetaTileMap classification (hunt for a misclassified corridor).
#[test]
#[ignore]
fn probe_vr1f_screenshots() {
    use crate::joypad::JoypadButton as JB;
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(3), vec![]);
    fixture.gb.run(MachineCycles::from_m(4_000));
    let dir_of = |b: JB| match b { JB::Up => (0i32, -1i32), JB::Down => (0, 1), JB::Left => (-1, 0), JB::Right => (1, 0), _ => (0, 0) };
    // Walk to `target` by repeatedly taking the first step of route_to (recomputed each tile — robust to
    // the initial turn-in-place).
    let mut walk_to = |fx: &mut TestFixture, target: Point8| {
        for _ in 0..120 {
            let s = fx.game_state();
            if s.map.player_position == target { return true; }
            let Some(route) = s.map.route_to(target) else { return false; };
            let Some(&dir) = route.first() else { return false; };
            let before = s.map.player_position;
            let (dx, dy) = dir_of(dir);
            let _ = (dx, dy);
            fx.gb.core_mut().mmu_mut().joypad_mut().press_button(dir);
            fx.gb.run(MachineCycles::from_m(260_000));
            fx.gb.core_mut().mmu_mut().joypad_mut().release_button(dir);
            fx.gb.run(MachineCycles::from_m(60_000));
            let after = fx.game_state().map.player_position;
            let _ = before; let _ = after;
        }
        false
    };
    // Walk to (7,9) [reachable], then MANUALLY drive Up across the row-9→row-8 tile-pair boundary that
    // my BFS blocks — does the real game allow it?
    let ok = walk_to(&mut fixture, Point8 { x: 7, y: 9 });
    println!("reached (7,9) = {ok}, actual = {}", fixture.game_state().map.player_position);
    // Raw tile IDs of the boundary tiles.
    {
        let s = fixture.game_state();
        for p in [(7u8, 9u8), (7, 8), (7, 7), (5, 9), (5, 8)] {
            let idx = p.0 as usize + p.1 as usize * s.map.width;
            println!("  raw tile id {:?} = ${:02x} (meta {:?})", p, s.map.raw_tile_ids[idx], s.map.tile_at(Point8 { x: p.0, y: p.1 }));
        }
    }
    for n in 0..8 {
        fixture.gb.core_mut().mmu_mut().joypad_mut().press_button(JB::Up);
        fixture.gb.run(MachineCycles::from_m(260_000));
        fixture.gb.core_mut().mmu_mut().joypad_mut().release_button(JB::Up);
        fixture.gb.run(MachineCycles::from_m(60_000));
        println!("  up#{n}: player @ {}", fixture.game_state().map.player_position);
    }
}

// Is Route 23 a single vertical strip (VR optional) or segmented (VR mandatory)? Approach Route 23 the
// proper way — up from the Route 22 gate — and report the northmost reachable tile + whether the Indigo
// Plateau connection and both VR entrances are reachable, WITHOUT entering Victory Road.
#[test]
#[ignore]
fn probe_route23_from_gate() {
    use crate::pokemon::map::MapSprite as MS;
    let bytes = std::fs::read("src/pokemon/data/post-earth-badge.bin").expect("no post-earth fixture");
    let steps = vec![
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::MovePokemonToFront { slot: 1 },
        PolicyStep::enter(Map::Route22),
        PolicyStep::enter(Map::Route22Gate),
        PolicyStep::Interact(MS::ROUTE22GATE_GUARD),
        PolicyStep::enter(Map::Route23),
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(15), steps);
    for i in 0..2_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> on Route23 at {i}"); break; }
    }
    let s = fixture.game_state();
    let reach = s.map.reachable_tiles();
    println!("map {} @ {} size {}x{}", s.map.map, s.map.player_position, s.map.width, s.map.height);
    let min_y = reach.iter().map(|p| p.y).min().unwrap_or(255);
    let max_y = reach.iter().map(|p| p.y).max().unwrap_or(0);
    println!("reachable y range = {min_y}..={max_y} (0 = Indigo Plateau edge), region size {}", reach.len());
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        let p = Point8 { x: (i % s.map.width) as u8, y: (i / s.map.width) as u8 };
        match t {
            MetaTile::Connection { to_map, .. } => println!("  conn {p} -> {:?} reachable={}", to_map, reach.contains(&p)),
            MetaTile::Warp { to_map, .. } if format!("{:?}", to_map).contains("Victory") =>
                println!("  warp {p} -> {:?} reachable={}", to_map, reach.contains(&p)),
            _ => {}
        }
    }
}

// Minimal manual boulder-push test: set Strength active in RAM, then mash a direction into a boulder
// and watch whether the boulder sprite moves — validates the double-press mechanic AND cross-checks the
// tile-classification (whether a tile my model calls a wall is actually pushable).
#[test]
#[ignore]
fn probe_vr_manual_push() {
    use crate::joypad::JoypadButton as JB;
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(2), vec![]);
    // Arm Strength directly (bit0 of wStatusFlags1).
    {
        let a = crate::pokemon::symbols::pokered_symbols::wStatusFlags1.address;
        let mmu = fixture.gb.core_mut().mmu_mut();
        let v = mmu.read(a); mmu.write(a, v | 0x01);
    }
    fixture.gb.run(MachineCycles::from_m(4_000));
    let boulders = |fx: &mut TestFixture| -> Vec<(String, Point8)> {
        let s = { PokemonApi::with_cache(&mut fx.gb, &mut fx.map_cache).game_state().unwrap() };
        s.map.sprites.iter().filter(|sp| sp.name.starts_with("Boulder"))
            .map(|sp| (sp.name.to_string(), sp.position)).collect()
    };
    let s0 = fixture.game_state();
    println!("player @ {} facing {:?}", s0.map.player_position, s0.map.player_direction);
    println!("boulders before: {:?}", boulders(&mut fixture));
    let mut walk = |fx: &mut TestFixture, dir: JB, tag: &str| {
        fx.gb.core_mut().mmu_mut().joypad_mut().press_button(dir);
        fx.gb.run(MachineCycles::from_m(260_000)); // ~15 frames — enough for a full step/turn
        fx.gb.core_mut().mmu_mut().joypad_mut().release_button(dir);
        fx.gb.run(MachineCycles::from_m(70_000));
        let s = fx.game_state();
        println!("  {tag} {dir:?}: player @ {} facing {:?} boulders {:?}", s.map.player_position, s.map.player_direction, boulders(fx));
    };
    // Reposition below the boulder: (4,15) -> (4,16) -> (5,16). Then push Up: boulder(5,15) -> (5,14).
    walk(&mut fixture, JB::Down, "move");
    walk(&mut fixture, JB::Right, "move");
    for n in 0..6 { walk(&mut fixture, JB::Up, &format!("push{n}")); }
}

// Traversal experiment: force ALL VR switch events open every step, reload VR1F so the barrier opens,
// climb to VR2F, and report which warps/connections each floor can reach — to map the real route to
// Indigo Plateau (and settle whether VR1F's switch is on the critical path).
#[test]
#[ignore]
fn probe_vr_traverse() {
    let set_all = |fx: &mut TestFixture| {
        let base = crate::pokemon::symbols::pokered_symbols::wEventFlags.address;
        let mmu = fx.gb.core_mut().mmu_mut();
        for (byte, mask) in [(290u16, 0x80u8), (167, 0x03), (204, 0x03)] {
            let cur = mmu.read(base + byte);
            mmu.write(base + byte, cur | mask);
        }
    };
    let dump = |fx: &mut TestFixture, tag: &str| {
        let s = { PokemonApi::with_cache(&mut fx.gb, &mut fx.map_cache).game_state().unwrap() };
        let reach = s.map.reachable_tiles();
        println!("[{tag}] map {} @ {} — reachable warps/connections:", s.map.map, s.map.player_position);
        for (i, t) in s.map.meta_tiles.iter().enumerate() {
            let p = Point8 { x: (i % s.map.width) as u8, y: (i / s.map.width) as u8 };
            match t {
                MetaTile::Warp { to_map, to_position } if reach.contains(&p) =>
                    println!("    warp {p} -> {:?} @ {}", to_map, to_position),
                MetaTile::Connection { to_map, .. } if reach.contains(&p) =>
                    println!("    conn {p} -> {:?}", to_map),
                _ => {}
            }
        }
    };
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    // Reload VR1F (event set) so its barrier opens, then climb to VR2F.
    let steps = vec![
        PolicyStep::enter(Map::Route23),
        PolicyStep::enter(Map::VictoryRoad1F),
        PolicyStep::enter(Map::VictoryRoad2F),               // via 1,1 barrier (now open)
        PolicyStep::enter(Map::VictoryRoad3F),               // via 23,7 stairs
        PolicyStep::enter_at(Map::VictoryRoad2F, 22, 16),   // via the 23,15 hole → falls to 2F east side
        PolicyStep::enter(Map::Route23),                     // via 29,7/8 exit toward Indigo Plateau
        PolicyStep::soft_goto(Map::IndigoPlateauLobby),
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(18), steps);
    set_all(&mut fixture);
    let mut last_map = Map::VictoryRoad1F;
    for i in 0..1_000_000 {
        set_all(&mut fixture);
        fixture.step();
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let moved = s.map.map != last_map;
                if moved { last_map = s.map.map; }
                if moved || (i > 13000 && i % 2000 == 0) {
                    println!("  {i}: {} @ {} st={}", s.map.map, s.map.player_position, fixture.agent.state_debug());
                }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> steps done at {i}"); break; }
    }
    dump(&mut fixture, "final");
    // Also report whether the VR3F hole (23,15) and its switch (3,5) are reachable.
    let s = fixture.game_state();
    let reach = s.map.reachable_tiles();
    for (name, p) in [("hole(23,15)", Point8 { x: 23, y: 15 }), ("switch(3,5)", Point8 { x: 3, y: 5 })] {
        println!("    {name} reachable={} tile={:?}", reach.contains(&p), s.map.tile_at(p));
    }
}

// REAL Victory Road traversal — NO event forcing. Solve each floor's boulder puzzle with genuine
// Strength pushes (UseStrength + SolveBoulders) and navigate VR1F→VR2F→VR3F→(fall through hole)→VR2F
// east→Route23→Indigo Plateau. Logs every map transition + push plan so one run shows how far it gets.
#[test]
#[ignore]
fn probe_vr_traverse_real() {
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let steps = vec![
        // VR1F: push a boulder onto (17,13), then climb the (1,1) ladder to VR2F.
        PolicyStep::UseStrength { slot: 2 },
        PolicyStep::SolveBoulders { switch: Point8 { x: 17, y: 13 } },
        PolicyStep::enter(Map::VictoryRoad2F),
        // VR2F west: press the (1,16) switch, then head up the (23,7) stairs to VR3F.
        PolicyStep::UseStrength { slot: 2 },
        PolicyStep::SolveBoulders { switch: Point8 { x: 1, y: 16 } },
        PolicyStep::enter(Map::VictoryRoad3F),
        // VR3F: fall through the (23,15) hole to VR2F's east side (lands ~ (22,16)).
        PolicyStep::enter_at(Map::VictoryRoad2F, 22, 16),
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(20), steps);
    let mut last_map = Map::VictoryRoad1F;
    let mut last_state = String::new();
    for i in 0..3_000_000 {
        fixture.step();
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            let st = fixture.agent.state_debug();
            if s.map.map != last_map {
                println!("  {i}: -> {} @ {}", s.map.map, s.map.player_position);
                last_map = s.map.map;
            }
            if st != last_state { println!("       [{i}] st={st}"); last_state = st; }
        }
        if fixture.agent.policy_exhausted() { println!(">> traversal steps done at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("FINAL: map {} @ {}", s.map.map, s.map.player_position);
    let reach = s.map.reachable_tiles();
    for (name, p) in [("VR2F exit (29,7)", Point8 { x: 29, y: 7 }), ("VR2F exit (29,8)", Point8 { x: 29, y: 8 }),
                      ("switch2 (9,16)", Point8 { x: 9, y: 16 })] {
        println!("    {name}: reachable={} tile={:?}", reach.contains(&p), s.map.tile_at(p));
    }
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        let p = Point8 { x: (i % s.map.width) as u8, y: (i / s.map.width) as u8 };
        if let MetaTile::Warp { to_map, .. } = t {
            if reach.contains(&p) { println!("    reachable warp {p} -> {:?}", to_map); }
        }
    }
}

// Grind Vaporeon (the weak link) for the Elite Four in the CINNABAR POKÉMON MANSION (from post-volcano-badge,
// Vaporeon lv36). The Mansion beats Route 23 as a grind map: it's a building (per-step wild encounters, no
// grass needed — the cave-grind wander applies), it has NO one-way ledge traps (Route 23 stranded the grind
// in a ledge pocket), the Cinnabar Pokémon Center is a clean short heal-return on the same island (no gates),
// and Vaporeon's Surf is 2× on the Fire wilds (Ponyta/Magmar). Vaporeon leads (slot 0), participate-then-
// hand-off keeps it alive, target lv42 → Aurora Beam (Ice, 2× on Lance's dragons). Saves `at-mansion-grinded.bin`.
#[test]
#[ignore]
fn probe_e4_grind() {
    use crate::pokemon::map::MapSprite as MS;
    // Grind from `post-volcano-lone.bin` — it has all 7 non-Earth badges (Badge 127, so the Viridian Gym
    // will open for the Earth Badge) AND a clean 2-mon party [Vaporeon(0) lv29, Venusaur(1)] with no dead-
    // weight Pidgey, so a caught Machop lands at slot 2 and the DEFAULT VR steps apply. (post-volcano-badge,
    // used earlier, was missing the Rainbow badge → the Viridian Gym stayed locked.) Grind Vaporeon to lv54
    // (Hydro Pump). Saves `at-mansion-grinded.bin`.
    let bytes = std::fs::read("src/pokemon/data/post-volcano-lone.bin").expect("no post-volcano-lone fixture");
    let target = 54u8;  // Vaporeon learns Hydro Pump at lv54 (no Ice move exists on its Gen-1 learnset)
    let steps = vec![
        PolicyStep::enter(Map::CinnabarIsland),               // out of the gym
        PolicyStep::enter(Map::CinnabarPokecenter),
        PolicyStep::Interact(MS::CINNABARPOKECENTER_NURSE),   // revive Vaporeon + set Cinnabar as heal anchor
        PolicyStep::enter(Map::CinnabarIsland),
        PolicyStep::enter(Map::PokemonMansion1F),
        PolicyStep::GrindUntilLevel { target_level: target, on_map: Map::PokemonMansion1F, slot: 0 },
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(180), steps);
    let mut last = String::new();
    let mut saved_lv = 0u8;
    for i in 0..18_000_000 {
        fixture.step();
        if i % 2000 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let v = &s.pokemon[0];
                let cur = format!("Vaporeon lv{} ({}/{}hp) on {}", v.level, v.current_hp, v.stats.hp, s.map.map);
                if cur != last { last = cur.clone(); println!("  {i}: {cur}"); }
                // Bank progress at each new level >= 40 (when at healthy HP), so a stall doesn't lose the
                // grind — the highest-level fixture is always on disk.
                if v.level > saved_lv && v.level >= 40 && v.current_hp > 0 {
                    saved_lv = v.level;
                    fixture.gb.save_state_to_file("src/pokemon/data/at-mansion-grinded.bin").ok();
                    println!("  >> banked at-mansion-grinded.bin at Vaporeon lv{}", v.level);
                }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> grind done at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("final party:"); for p in s.pokemon.iter() {
        let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}", m.name)).collect();
        println!("  {:?} lv{} — {}", p.species, p.level, moves.join("/"));
    }
    if s.pokemon[0].level >= target {
        fixture.gb.save_state_to_file("src/pokemon/data/at-mansion-grinded.bin").ok();
        println!(">> saved at-mansion-grinded.bin (Vaporeon lv{}, in the Pokémon Mansion)", s.pokemon[0].level);
    }
}

// Explore Seafoam Islands to plan the Articuno route: from at-mansion-grinded (Cinnabar), teach Strength to
// Venusaur (for the boulders), Surf east onto Route 20, into Seafoam 1F, and dump each floor's tile grid +
// boulder positions so we can plan the boulder→hole current-stop puzzle down to Articuno on B4F.
#[test]
#[ignore]
fn probe_seafoam_explore() {
    use crate::pokemon::map::MapSprite as MS;
    let _ = MS::SEAFOAMISLANDS1F_BOULDER1;
    let bytes = std::fs::read("src/pokemon/data/at-mansion-grinded.bin").expect("no fixture");
    let steps = vec![
        PolicyStep::enter(Map::CinnabarIsland),
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::GreatBall, 10), map: Map::CinnabarMart },
        PolicyStep::enter(Map::CinnabarIsland),
        PolicyStep::enter(Map::Route20),
        PolicyStep::enter(Map::SeafoamIslands1F),
        // Seafoam is a cave with LAND encounters — the cave-wander (pace to boulders) triggers Seel.
        PolicyStep::CatchPokemon { species: crate::pokemon::species::PokemonSpecies::Seel, on_map: Map::SeafoamIslands1F },
        PolicyStep::TeachMove { item: ItemId::Hm04Strength, target_slot: 2 },
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(30), steps);
    let mut seen = std::collections::HashSet::new();
    let dump = |s: &crate::pokemon::GameState| {
        let m = &s.map;
        println!("== {} {}x{} player@{} ==", m.map, m.width, m.height, m.player_position);
        for (i, b) in m.sprites.iter().enumerate() { println!("  sprite{i}: {:?}@{} hidden={}", b.name, b.position, b.hidden); }
        println!("  strength_switches={:?} holes={:?} can_surf={}", m.strength_switches, m.holes, m.can_surf);
    };
    for i in 0..3_000_000 {
        fixture.step();
        if i % 500 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if seen.insert(s.map.map) { println!("--- reached {} at {i} ---", s.map.map); dump(&s); }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> done at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("FINAL:"); dump(&s);
}

// E4 attempt, Phase 1: take the strong grinded team (Vaporeon lv54 + Venusaur lv55, `at-mansion-grinded.bin`,
// at Cinnabar post-Volcano-Badge) through the back-half to the Indigo lobby: Earth Badge (Giovanni) → Victory
// Road (catch Machop HM-slave, Strength puzzle 1F/2F/3F). The fixture party is [Vaporeon(0), Venusaur(1),
// Pidgey(2)], so `MovePokemonToFront{1}` (inside the VR steps) leads Venusaur and the caught Machop lands at
// slot 3. Saves `at-indigo-strong.bin` for the gauntlet leg.
#[test]
#[ignore]
fn probe_e4_backhalf() {
    use crate::pokemon::map::MapSprite as MS;
    let bytes = std::fs::read("src/pokemon/data/at-mansion-blizzard.bin").expect("run probe_get_blizzard first");
    let mut steps = PolicyStep::earth_badge_steps();
    // Custom VR1F leading the bulky Blizzard VAPOREON (slot 0) at the Route-22 rival instead of Venusaur:
    // Venusaur is Grass/POISON → 2× to the rival's Alakazam Psychic and runs its RazorLeaf PP out, falling
    // into a heal-flee loop; Vaporeon (Water, 224 HP, full PP, Blizzard/Surf covers the rival's whole team)
    // tanks it. Otherwise identical to victory_road_1f_steps(2): catch Machop (slot 2) + Strength + solve 1F.
    steps.extend(vec![
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::Interact(MS::VIRIDIANPOKECENTER_NURSE),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianMart),
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 20), map: Map::ViridianMart },
        PolicyStep::enter(Map::ViridianCity),
        // NB: no MovePokemonToFront — Vaporeon already leads at slot 0.
        PolicyStep::enter(Map::Route22),
        PolicyStep::enter(Map::Route22Gate),
        PolicyStep::Interact(MS::ROUTE22GATE_GUARD),
        PolicyStep::enter(Map::Route23),
        PolicyStep::goto(Map::VictoryRoad1F),
        PolicyStep::CatchPokemon { species: crate::pokemon::species::PokemonSpecies::Machop, on_map: Map::VictoryRoad1F },
        PolicyStep::TeachMove { item: ItemId::Hm04Strength, target_slot: 2 },
        PolicyStep::UseStrength { slot: 2 },
        PolicyStep::SolveBoulders { switch: Point8 { x: 17, y: 13 } },
        PolicyStep::enter(Map::VictoryRoad2F),
    ]);
    steps.extend(PolicyStep::victory_road_2f_3f_steps(2));
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(90), steps);
    let mut last = Map::PalletTown;
    for i in 0..9_000_000 {
        fixture.step();
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            if s.map.map != last { last = s.map.map; println!("  {i}: -> {} (badges {:?})", s.map.map, s.badges); }
        }
        if fixture.agent.policy_exhausted() { println!(">> back-half complete at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("final: map {} @ {}", s.map.map, s.map.player_position);
    println!("team:"); for p in s.pokemon.iter() { println!("  {:?} lv{} {}/{}hp", p.species, p.level, p.current_hp, p.stats.hp); }
    if s.map.map == Map::IndigoPlateauLobby {
        fixture.gb.save_state_to_file("src/pokemon/data/at-indigo-ice.bin").ok();
        println!(">> saved at-indigo-ice.bin (Blizzard team at the Indigo lobby)");
    }
}

// Validate getting TM14 Blizzard (in Pokémon Mansion B1F) onto Vaporeon, from `at-mansion-grinded.bin`
// (in Mansion 1F). Blizzard (Ice, 120) is 2× on all of Lance's dragons — the Elite-Four Lance answer.
// pick_move_to_forget keeps the strongest moves, so Blizzard displaces Mist and survives the later Hydro
// Pump learn. Saves `at-mansion-blizzard.bin` for a back-half re-run that carries Blizzard to the E4.
#[test]
#[ignore]
fn probe_get_blizzard() {
    use crate::pokemon::map::MapSprite as MS;
    let bytes = std::fs::read("src/pokemon/data/at-mansion-grinded.bin").expect("run the grind first");
    // Reuse the proven B1F navigation from `mansion_secret_key_steps` (up to 3F, flip the switch, fall
    // through the hole to a new 1F section, take the B1F staircase, flip the B1F switches to open the walls),
    // then grab the Blizzard TM at (19,25) — right by the (18,25) switch — instead of the Secret Key.
    let steps = vec![
        PolicyStep::enter(Map::CinnabarIsland),        // exit the Mansion first (grind ends inside it)
        PolicyStep::enter(Map::CinnabarPokecenter),   // heal (restore Surf PP for the battle-heavy crossing)
        PolicyStep::Interact(MS::CINNABARPOKECENTER_NURSE),
        PolicyStep::enter(Map::CinnabarIsland),
        PolicyStep::enter(Map::PokemonMansion1F),
        PolicyStep::enter(Map::PokemonMansion2F),
        PolicyStep::EnterMap { to_map: Map::PokemonMansion3F, to_position: Some(Point8 { x: 6, y: 1 }) },
        PolicyStep::FlipSwitch { map: Map::PokemonMansion3F, at: Point8 { x: 10, y: 5 }, reveals: Map::PokemonMansion1F },
        PolicyStep::enter(Map::PokemonMansion1F),   // fall through a hole → 1F (16,14)
        PolicyStep::enter(Map::PokemonMansionB1F),  // (21,23) staircase down
        // The Blizzard TM (19,25) sits right by the (18,25) switch — flip it and grab the TM immediately,
        // BEFORE the (20,3) switch (which the Secret-Key route flips next but re-closes this section).
        PolicyStep::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 18, y: 25 }, reveals: Map::PokemonMansion1F },
        PolicyStep::CollectItem(MS::POKEMONMANSIONB1F_TM_BLIZZARD),
        PolicyStep::TeachMove { item: ItemId::Tm14Blizzard, target_slot: 0 }, // Vaporeon (slot 0)
        // Exit B1F → Cinnabar: flip (18,25) back to default (I flipped it once for the TM) — from the
        // adjacent Blizzard spot — so the (23,22) staircase area (reachable in the default state, where we
        // entered) reopens; then up to 1F and out to Cinnabar so the back-half starts cleanly.
        PolicyStep::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 18, y: 25 }, reveals: Map::PokemonMansion1F },
        PolicyStep::enter(Map::PokemonMansion1F),   // (23,22) up-staircase → 1F
        PolicyStep::enter(Map::CinnabarIsland),
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(25), steps);
    let mut last = Map::PalletTown;
    for i in 0..2_000_000 {
        fixture.step();
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            if s.map.map != last { last = s.map.map; println!("  {i}: -> {}", s.map.map); }
        }
        if fixture.agent.policy_exhausted() { println!(">> done at {i}"); break; }
    }
    let s = fixture.game_state();
    let v = &s.pokemon[0];
    let moves: Vec<String> = v.moves.iter().flatten().map(|m| format!("{:?}", m.name)).collect();
    println!("Vaporeon lv{} moves: {}", v.level, moves.join("/"));
    let has_blizzard = v.moves.iter().flatten().any(|m| format!("{:?}", m.name) == "Blizzard");
    if has_blizzard {
        fixture.gb.save_state_to_file("src/pokemon/data/at-mansion-blizzard.bin").ok();
        println!(">> BLIZZARD TAUGHT — saved at-mansion-blizzard.bin");
    } else { println!("!! Blizzard not on Vaporeon"); }
}

// E4 attempt, Phase 2: run the Elite Four gauntlet with the strong grinded team from `at-indigo-strong.bin`
// (Venusaur lv57 + Vaporeon lv55 Hydro Pump/Surf 238 HP, 54k money → ~15 Full Restores). Heal, stock items,
// lead Venusaur, then Lorelei → Bruno → Agatha → Lance → Champion. Logs how far it gets.
#[test]
#[ignore]
fn probe_e4_gauntlet() {
    use crate::pokemon::map::MapSprite as MS;
    let bytes = std::fs::read("src/pokemon/data/at-indigo-ice.bin").expect("run probe_e4_backhalf first");
    let steps = vec![
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::FullRestore, 15), map: Map::IndigoPlateauLobby },
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::Revive, 5), map: Map::IndigoPlateauLobby },
        PolicyStep::Interact(MS::INDIGOPLATEAULOBBY_NURSE),  // revive + restore all PP
        PolicyStep::MovePokemonToFront { slot: 1 },          // Venusaur (slot 1) leads; Vaporeon's Blizzard for Lance
        PolicyStep::enter(Map::LoreleisRoom),
        PolicyStep::BattleTrainer { trainer: MS::LORELEISROOM_LORELEI },
        PolicyStep::enter(Map::BrunosRoom),
        PolicyStep::BattleTrainer { trainer: MS::BRUNOSROOM_BRUNO },
        PolicyStep::enter(Map::AgathasRoom),
        PolicyStep::BattleTrainer { trainer: MS::AGATHASROOM_AGATHA },
        PolicyStep::enter(Map::LancesRoom),
        PolicyStep::BattleTrainer { trainer: MS::LANCESROOM_LANCE },
        PolicyStep::enter(Map::ChampionsRoom),
        PolicyStep::BattleTrainer { trainer: MS::CHAMPIONSROOM_RIVAL },
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(120), steps); // long attrition vs Lance
    let mut last = Map::PalletTown;
    let mut furthest = Map::IndigoPlateauLobby;
    for i in 0..4_500_000 {
        fixture.step();
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            if s.map.map != last { last = s.map.map; furthest = s.map.map; println!("  {i}: -> {}", s.map.map); }
        }
        if fixture.agent.policy_exhausted() { println!(">> GAUNTLET COMPLETE — E4 BEATEN at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("furthest room: {furthest}; final map {} @ {}", s.map.map, s.map.player_position);
    println!("final team:"); for p in s.pokemon.iter() { println!("  {:?} lv{} {}/{}hp", p.species, p.level, p.current_hp, p.stats.hp); }
    if s.map.map == Map::HallOfFame { println!(">> HALL OF FAME REACHED — CHAMPION!"); }
}

// Dump a fixture's team/location (edit the fixture list); used to plan grinds + the E4 attempt.
#[test]
#[ignore]
fn probe_peb_state() {
    for f in ["at-mansion-grinded"] {
        let bytes = match std::fs::read(format!("src/pokemon/data/{f}.bin")) { Ok(b) => b, Err(_) => continue };
        let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
        let s = fixture.game_state();
        println!("== {f}: map {} @ {} | badges {:?} | money {}", s.map.map, s.map.player_position, s.badges, s.money);
        for (i, p) in s.pokemon.iter().enumerate() {
            let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}(pp{})", m.name, m.pp)).collect();
            println!("  slot{i}: {:?} lv{} {}/{}hp — {}", p.species, p.level, p.current_hp, p.stats.hp, moves.join(", "));
        }
        let bag: Vec<String> = s.bag.iter().map(|it| format!("{:?}x{}", it.id, it.quantity)).collect();
        println!("  bag[{}/20]: {}", s.bag.iter().count(), bag.join(", "));
    }
}

// Dump the Indigo-lobby team + bag + money, to plan the Elite Four prep (grind / coverage TMs / items).
#[test]
#[ignore]
fn probe_e4_inventory() {
    let bytes = std::fs::read("src/pokemon/data/at-indigo.bin").expect("run can_solve_victory_road_2f_3f first");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    println!("money: {}", s.money);
    println!("bag:"); for it in s.bag.iter() { println!("  {:?} x{}", it.id, it.quantity); }
    println!("party:");
    for p in s.pokemon.iter() {
        let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}(pp{})", m.name, m.pp)).collect();
        println!("  {:?} lv{} hp{}/{} atk{} def{} spd{} spc{} — {}", p.species, p.level, p.current_hp, p.stats.hp,
            p.stats.attack, p.stats.defense, p.stats.speed, p.stats.special, moves.join(", "));
    }
}

// Exploratory Elite Four attempt with the CURRENT (unprepped) team, from the Indigo lobby fixture. Heals,
// then walks the gauntlet Lorelei→Bruno→Agatha→Lance→Champion. Logs how far it gets — grounds the team-prep
// plan in the real failure point + validates room navigation / auto-engage battles. (Run after
// `can_solve_victory_road_2f_3f` has created `at-indigo.bin`.)
#[test]
#[ignore]
fn probe_e4_first_attempt() {
    use crate::pokemon::map::MapSprite as MS;
    let bytes = std::fs::read("src/pokemon/data/at-indigo.bin").expect("run can_solve_victory_road_2f_3f first");
    let steps = vec![
        // Stock the gauntlet: Full Restores (heal HP+status) + Revives, from the lobby clerk. ~30k money.
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::FullRestore, 8), map: Map::IndigoPlateauLobby },
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::Revive, 4), map: Map::IndigoPlateauLobby },
        PolicyStep::Interact(MS::INDIGOPLATEAULOBBY_NURSE), // heal to full before the gauntlet
        PolicyStep::MovePokemonToFront { slot: 0 },         // Venusaur leads
        PolicyStep::enter(Map::LoreleisRoom),
        PolicyStep::BattleTrainer { trainer: MS::LORELEISROOM_LORELEI },
        PolicyStep::enter(Map::BrunosRoom),
        PolicyStep::BattleTrainer { trainer: MS::BRUNOSROOM_BRUNO },
        PolicyStep::enter(Map::AgathasRoom),
        PolicyStep::BattleTrainer { trainer: MS::AGATHASROOM_AGATHA },
        PolicyStep::enter(Map::LancesRoom),
        PolicyStep::BattleTrainer { trainer: MS::LANCESROOM_LANCE },
        PolicyStep::enter(Map::ChampionsRoom),
        PolicyStep::BattleTrainer { trainer: MS::CHAMPIONSROOM_RIVAL },
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(30), steps);
    { let s = fixture.game_state(); print!("team: "); for p in s.pokemon.iter() { print!("{:?}{} ", p.species, p.level); } println!(); }
    let mut last = Map::PalletTown;
    let mut furthest = Map::IndigoPlateauLobby;
    for i in 0..3_000_000 {
        fixture.step();
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            if s.map.map != last { last = s.map.map; furthest = s.map.map; println!("  {i}: -> {}", s.map.map); }
        }
        if fixture.agent.policy_exhausted() { println!(">> gauntlet complete at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("furthest room reached: {furthest}; final map {} @ {}", s.map.map, s.map.player_position);
    println!("final team:"); for p in s.pokemon.iter() { println!("  {:?} lv{} {}/{}hp", p.species, p.level, p.current_hp, p.stats.hp); }
}

// The Strength switch/hole positions are exposed on the map (`strength_switches` / `holes`) so a policy —
// deterministic or a future LLM — can discover where to push boulders without hardcoding coordinates.
#[test]
#[ignore]
fn strength_switches_are_exposed() {
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    assert_eq!(s.map.map, Map::VictoryRoad1F);
    assert_eq!(s.map.strength_switches, vec![Point8 { x: 17, y: 13 }], "VR1F switch should be exposed");
    assert!(s.map.holes.is_empty(), "VR1F has no holes");
}

// Validates the full interconnected VR2F/VR3F Strength puzzle → Indigo Plateau lobby. Self-contained: from
// `vr1f-strength.bin` (Machop HM-slave already caught+taught), solve VR1F, climb to VR2F, then run the
// `victory_road_2f_3f_steps` puzzle (switch1 → 3F → hole-drop reveals the hidden 2F boulder → fall → switch2
// → return trip → exit). Every boulder is a real Strength push. This is the proof the VR2F/VR3F mechanic
// works end-to-end; chaining it onto a *fresh* run is PP-marginal (see `victory_road_2f_3f_steps`).
#[test]
#[ignore]
fn can_solve_victory_road_2f_3f() {
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut steps = vec![
        PolicyStep::UseStrength { slot: 2 },
        PolicyStep::SolveBoulders { switch: Point8 { x: 17, y: 13 } },
        PolicyStep::enter(Map::VictoryRoad2F),
    ];
    steps.extend(PolicyStep::victory_road_2f_3f_steps(2));
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(60), steps);
    let mut last = (Map::PalletTown, Point8 { x: 255, y: 255 });
    for i in 0..6_000_000 {
        fixture.step();
        if i % 500 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  {i}: {} @ {}", s.map.map, s.map.player_position); }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> done at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::IndigoPlateauLobby, "should clear the VR2F/VR3F puzzle to the Indigo Plateau lobby");
    // Snapshot the Indigo Plateau lobby (weak team) for fast Elite Four iteration.
    fixture.gb.save_state_to_file("src/pokemon/data/at-indigo.bin").ok();
}

// Validate the Strength-activation executor: from vr1f-strength (Machop-with-Strength at slot 2), run
// PolicyStep::UseStrength and assert BIT_STRENGTH_ACTIVE gets set.
#[test]
#[ignore]
fn probe_vr_use_strength() {
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let steps = vec![PolicyStep::UseStrength { slot: 2 }];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(2), steps);
    let mut activated = false;
    let mut last = String::new();
    for i in 0..300_000 {
        fixture.step();
        if i % 20 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if s.strength_active { activated = true; }
                let mut api = PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache);
                let txt = api.on_screen_text(false).unwrap_or_default();
                let cur = format!("{:?} st={} str={} txt={:?}", s.mode, fixture.agent.state_debug(), s.strength_active, txt.chars().take(40).collect::<String>());
                if cur != last { println!("  {i}: {cur}"); last = cur; }
            }
        }
        if fixture.agent.policy_exhausted() { println!(">> policy done at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("strength_active = {} (seen active during run = {activated})", s.strength_active);
    assert!(activated, "Strength should have been activated (BIT_STRENGTH_ACTIVE set)");
}

// END-TO-END VR1F STRENGTH PROOF: from vr1f-strength.bin, arm Strength (Machop @ slot 2) then run the
// SolveBoulders driver. Prove — in the running emulator — that the agent pushes a real boulder onto the
// real switch (17,13): screenshot it, assert the boulder-on-switch event gets set by the map script, and
// assert the (1,1) → VR2F ladder becomes reachable (the ReplaceTileBlock barrier opened). This is the
// definitive "Strength CAN work" confirmation, with no RAM cheating.
#[test]
#[ignore]
fn probe_vr1f_strength_solve() {
    let switch = Point8 { x: 17, y: 13 };
    let ladder = Point8 { x: 1, y: 1 };
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let steps = vec![
        PolicyStep::UseStrength { slot: 2 },
        PolicyStep::SolveBoulders { switch },
    ];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(6), steps);

    let vr1_byte = 290u16; let vr1_mask = 0x80u8; // EVENT_VICTORY_ROAD_1_BOULDER_ON_SWITCH
    let event_set = |fx: &mut TestFixture| -> bool {
        let base = crate::pokemon::symbols::pokered_symbols::wEventFlags.address;
        (fx.gb.core_mut().mmu_mut().read(base + vr1_byte) & vr1_mask) != 0
    };
    let boulder_on_switch = |fx: &mut TestFixture| -> bool {
        let s = { PokemonApi::with_cache(&mut fx.gb, &mut fx.map_cache).game_state() };
        s.map(|s| s.map.sprites.iter().any(|sp| sp.name.starts_with("Boulder") && sp.position == switch)).unwrap_or(false)
    };

    let mut last = String::new();
    let mut shot_on_switch = false;
    for i in 0..2_000_000 {
        fixture.step();
        if i % 50 == 0 {
            let st = fixture.agent.state_debug();
            if st != last { println!("  {i}: state={st}"); last = st; }
        }
        if !shot_on_switch && boulder_on_switch(&mut fixture) {
            fixture.gb.save_screenshot_to_file(&format!("{SCRATCH}/vr1f_boulder_on_switch.png")).ok();
            println!(">> boulder reached switch {switch} at step {i}");
            shot_on_switch = true;
        }
        if fixture.agent.policy_exhausted() { println!(">> policy exhausted at {i}"); break; }
    }

    let on_switch = boulder_on_switch(&mut fixture);
    let evt = event_set(&mut fixture);
    let s = fixture.game_state();
    let reach = s.map.reachable_tiles();
    let ladder_reachable = reach.contains(&ladder);
    fixture.gb.save_screenshot_to_file(&format!("{SCRATCH}/vr1f_after_solve.png")).ok();
    println!("boulder on switch = {on_switch}, event set = {evt}, (1,1) ladder reachable = {ladder_reachable}");
    println!("player @ {}, reachable region = {} tiles", s.map.player_position, reach.len());

    assert!(on_switch, "a boulder should be sitting on the switch {switch}");
    assert!(evt, "the boulder-on-switch event should be set by the map script");
    assert!(ladder_reachable, "the (1,1) → VR2F ladder should be reachable (ReplaceTileBlock barrier opened)");
}

// Validate the barrier model: force the VR1 boulder-on-switch event, reload VR1F so the game runs
// ReplaceTileBlock, and confirm the (1,1) up-ladder to VR2F becomes reachable.
#[test]
#[ignore]
fn probe_vr_barrier_test() {
    let vr1_byte = 290u16; let vr1_mask = 0x80u8;
    let set_vr1 = |fx: &mut TestFixture| {
        let base = crate::pokemon::symbols::pokered_symbols::wEventFlags.address;
        let mmu = fx.gb.core_mut().mmu_mut();
        let cur = mmu.read(base + vr1_byte);
        mmu.write(base + vr1_byte, cur | vr1_mask);
    };
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    // Exit to Route 23 and back into VR1F so the load script runs ReplaceTileBlock with the event set.
    let steps = vec![PolicyStep::enter(Map::Route23), PolicyStep::enter(Map::VictoryRoad1F)];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(6), steps);
    set_vr1(&mut fixture);
    for i in 0..600_000 {
        set_vr1(&mut fixture); // keep it set across any script that might clear it
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> back in VR1F at {i}"); break; }
    }
    let s = fixture.game_state();
    let reach = s.map.reachable_tiles();
    println!("map {} player @ {}", s.map.map, s.map.player_position);
    println!("(1,1) reachable = {}", reach.contains(&Point8 { x: 1, y: 1 }));
    println!("(9,12) walkable = {:?}", s.map.tile_at(Point8 { x: 9, y: 12 }));
}

// Can the player walk up Route 23 to the Indigo Plateau connection with 8 badges (is Victory Road
// mandatory)?  From vr1f-strength (in VR1F), exit to Route 23 and inspect reachability + connections.
#[test]
#[ignore]
fn probe_route23_walkup() {
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let steps = vec![PolicyStep::enter(Map::Route23)];
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(5), steps);
    for i in 0..400_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> reached Route23 at {i}"); break; }
    }
    let s = fixture.game_state();
    println!("map {} player @ {} size {}x{}", s.map.map, s.map.player_position, s.map.width, s.map.height);
    let reach = s.map.reachable_tiles();
    // Report every Connection tile and whether it's reachable (the north one → IndigoPlateau).
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        if let MetaTile::Connection { to_map, .. } = t {
            let p = Point8 { x: (i % s.map.width) as u8, y: (i / s.map.width) as u8 };
            println!("  Connection {p} -> {:?} reachable={}", to_map, reach.contains(&p));
        }
        if let MetaTile::Warp { to_map, .. } = t {
            let p = Point8 { x: (i % s.map.width) as u8, y: (i / s.map.width) as u8 };
            if format!("{:?}", to_map).contains("Victory") {
                println!("  Warp {p} -> {:?} reachable={}", to_map, reach.contains(&p));
            }
        }
    }
    let min_y = reach.iter().map(|p| p.y).min().unwrap_or(255);
    println!("northmost reachable y = {min_y} (0 = Indigo Plateau edge)");
}

// Test the single-boulder Sokoban solver against the VR1F puzzle (switch at (17,13)).
#[test]
#[ignore]
fn probe_vr1f_solve() {
    let bytes = std::fs::read("src/pokemon/data/vr1f-strength.bin").expect("no vr1f-strength fixture");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    println!("player @ {}, map {}", s.map.player_position, s.map.map);
    for sp in s.map.sprites.iter().filter(|sp| sp.name.starts_with("Boulder")) {
        println!("  {} @ {}", sp.name, sp.position);
    }
    // Dump the tile grid (F=floor/empty, G=grass, #=obstacle, ~=water, B=boulder, W=warp, .=other)
    println!("grid {}x{}:", s.map.width, s.map.height);
    for y in 0..s.map.height {
        let mut row = String::new();
        for x in 0..s.map.width {
            let p = Point8 { x: x as u8, y: y as u8 };
            let c = if s.map.sprites.iter().any(|sp| sp.name.starts_with("Boulder") && sp.position == p) { 'B' }
                else if p == (Point8 { x: 17, y: 13 }) { 'S' }
                else if p == s.map.player_position { '@' }
                else { match s.map.tile_at(p) {
                    MetaTile::Empty => '.',
                    MetaTile::Grass => 'G',
                    MetaTile::Water => '~',
                    MetaTile::Warp { .. } => 'W',
                    MetaTile::Obstacle => '#',
                    _ => '?',
                } };
            row.push(c);
        }
        println!("{y:2} {row}");
    }
    let reach = s.map.reachable_tiles();
    for target in [(1u8, 1u8), (8, 17), (17, 13), (17, 12), (14, 2), (2, 10), (5, 4), (11, 2)] {
        let p = Point8 { x: target.0, y: target.1 };
        println!("reachable {:?} = {}", target, reach.contains(&p));
    }
    println!("reachable region size = {}", reach.len());
    // The apparent column-7 passage (rows 6-8) that my grid shows connecting bottom<->top:
    for c in [(7u8, 6u8), (7, 7), (7, 8), (7, 9), (5, 7), (5, 8), (5, 9), (5, 10), (6, 6), (6, 7)] {
        let p = Point8 { x: c.0, y: c.1 };
        println!("  col-check {:?}: tile={:?} reachable={}", c, s.map.tile_at(p), reach.contains(&p));
    }
    let switch = Point8 { x: 17, y: 13 };
    match s.map.solve_boulder_push(switch) {
        Some(pushes) => {
            println!(">> solution ({} pushes):", pushes.len());
            for (from, dir) in &pushes {
                println!("   push boulder at {from} {:?}", dir);
            }
        }
        None => println!(">> NO SOLUTION"),
    }
}

#[test]
#[ignore]
fn probe_party() {
    let mut fixture = TestFixture::new(include_bytes!("data/at-saffron-post-silph.bin"), Duration::from_mins(1), vec![]);
    for _ in 0..60 { fixture.step(); }
    let s = fixture.game_state();
    println!("=== on {} @ {} — party ({}), badges={:?} money=₽{} ===", s.map.map, s.map.player_position, s.pokemon.len(), s.badges, s.money);
    let w = s.map.width;
    println!("ALL warps on this map:");
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        if let MetaTile::Warp { to_map, to_position } = t { println!("   ({},{}) -> {to_map} {to_position}", i % w, i / w); }
    }
    println!("sprites: {:?}", s.map.sprites.iter().map(|sp| (sp.name, sp.position, sp.hidden)).collect::<Vec<_>>());
    println!("map:\n{}", s.map);
    for (i, p) in s.pokemon.iter().enumerate() {
        let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}({})", m.name, m.pp)).collect();
        println!("  {i}: {:?} lv{} hp {}/{} types {:?} moves {:?}",
            p.species, p.level, p.current_hp, p.stats.hp, p.types, moves);
    }
    println!("=== bag ===");
    for item in s.bag.iter() { println!("  {} ×{}", item.id, item.quantity); }
    println!("=== reachable warps on {} ===", s.map.map);
    for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..} | MetaTile::Connection{..})) {
        println!("   {:?} dest={} route_len={}", a.tile, a.destination, a.route.len());
    }
}

/// From `eevee-and-stone.bin` (Eevee lv25 in slot 2 + one Water Stone): use the stone to evolve Eevee →
/// Vaporeon, then teach Surf (HM03) to it. Verifies slot 2 becomes Vaporeon and knows Surf, then saves
/// `vaporeon-ready.bin`.
#[test]
#[ignore]
fn probe_evolve_eevee() {
    let mut fixture = TestFixture::new(include_bytes!("data/eevee-and-stone.bin"), Duration::from_mins(5),
        vec![
            PolicyStep::EvolveWithStone { stone: ItemId::WaterStone, target_slot: 2 },
            PolicyStep::TeachMove { item: ItemId::Hm03Surf, target_slot: 2 },
        ]);
    for i in 0..400_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
    }
    let s = fixture.game_state();
    for (i, p) in s.pokemon.iter().enumerate() {
        let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}", m.name)).collect();
        println!("   {i}: {:?} lv{} moves {:?}", p.species, p.level, moves);
    }
    let slot2 = &s.pokemon[2];
    let is_vaporeon = format!("{:?}", slot2.species) == "Vaporeon";
    let knows_surf = slot2.moves.iter().flatten().any(|m| format!("{:?}", m.name) == "Surf");
    println!("slot2 vaporeon={is_vaporeon} knows_surf={knows_surf}");
    if is_vaporeon && knows_surf {
        fixture.save_state_named("src/pokemon/data/vaporeon-ready.bin").unwrap();
        println!(">> saved vaporeon-ready.bin");
    }
    assert!(is_vaporeon, "Eevee did not evolve into Vaporeon");
    assert!(knows_surf, "Vaporeon did not learn Surf");
}

/// From `vaporeon-ready.bin` (Vaporeon lv25 slot 2, knows Surf): grind it up on a nearby route,
/// switching it in each wild battle so it earns the XP. Prints every level-up to measure the rate.
#[test]
#[ignore]
fn probe_grind_vaporeon() {
    let grind_map = Map::Route6;
    let target = 30u8;
    let mut fixture = TestFixture::new(include_bytes!("data/vaporeon-ready.bin"), Duration::from_mins(240),
        vec![
            // Register the Celadon Pokémon Center so a fainted grind mon can be routed back to heal.
            PolicyStep::enter(Map::CeladonPokecenter),
            PolicyStep::enter(Map::CeladonCity),
            // Celadon → Saffron via the Route-7 gate (the reverse of the outbound crossing).
            PolicyStep::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 11, y: 10 }) },
            PolicyStep::enter(Map::Route7Gate),
            PolicyStep::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 19, y: 10 }) },
            PolicyStep::enter(Map::SaffronCity),
            // Saffron→Route6 lands in a walled north pocket; the real body (with grass) is through the
            // Route6 gate, same shape as the Route7 crossing.
            PolicyStep::enter(grind_map),
            PolicyStep::enter(Map::Route6Gate),
            PolicyStep::enter(grind_map),
            PolicyStep::GrindUntilLevel { target_level: target, on_map: grind_map, slot: 2 },
        ]);
    let mut last_dump = Point8 { x: 255, y: 255 };
    let mut last_lv = 0u8;
    for i in 0..3_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 500 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if s.map.map == grind_map && s.map.player_position != last_dump {
                    last_dump = s.map.player_position;
                    let grass = s.map.actions().iter().filter(|a| a.tile == MetaTile::Grass).count();
                    println!("  Route6 @ {} grass_reachable={grass}", s.map.player_position);
                }
            }
        }
        if i % 2000 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if let Some(v) = s.pokemon.get(2) {
                    if v.level != last_lv {
                        last_lv = v.level;
                        println!("  step {i}: slot2 {:?} lv{} hp {}/{} @ {}",
                            v.species, v.level, v.current_hp, v.stats.hp, s.map.map);
                    }
                    if v.level >= target { println!(">> reached target lv{target} at step {i}"); break; }
                }
            }
        }
    }
    let s = fixture.game_state();
    let v = &s.pokemon[2];
    println!("final: slot2 {:?} lv{} on {}", v.species, v.level, s.map.map);
    if v.level >= target {
        fixture.save_state_named("src/pokemon/data/vaporeon-grinded.bin").unwrap();
        println!(">> saved vaporeon-grinded.bin");
    }
}

/// Train Vaporeon on the Silph Co trainer gauntlet: from `vaporeon-ready.bin` (Celadon), cross to
/// Saffron, enter Silph, turn on train-slot mode (Vaporeon = slot 2 switched into every battle), and
/// walk up floor-by-floor. Line-of-sight grunts auto-trigger and feed Vaporeon XP. Reports how far the
/// floor navigation gets and Vaporeon's level, so we can see whether the gauntlet is a viable trainer.
#[test]
#[ignore]
fn probe_silph_train() {
    use crate::pokemon::map::MapSprite as MS;
    let train = |s: MS| PolicyStep::InteractIfReachable(s);
    let mut steps = vec![
        // Stock Super Potions at Celadon Dept 2F first — the battle policy heals the active mon at
        // <25% HP if it has them, keeping Vaporeon alive across the gauntlet (₽6728 ≈ 9 Super Potions).
        PolicyStep::enter(Map::CeladonMart1F),
        PolicyStep::enter(Map::CeladonMart2F),
        PolicyStep::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::SuperPotion, 9), map: Map::CeladonMart2F },
        PolicyStep::enter(Map::CeladonMart1F),
        PolicyStep::enter(Map::CeladonCity),
        // Celadon → Saffron via the Route-7 gate (reverse of the outbound crossing).
        PolicyStep::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 11, y: 10 }) },
        PolicyStep::enter(Map::Route7Gate),
        PolicyStep::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 19, y: 10 }) },
        PolicyStep::enter(Map::SaffronCity),
        PolicyStep::enter(Map::SilphCo1F),
        // Make Vaporeon (slot 2) the lead so it fights — and levels — from the start of every battle;
        // the battle policy heals it with the Super Potions when it drops below 25% HP.
        PolicyStep::MovePokemonToFront { slot: 2 },
    ];
    // Train each floor's Rocket/Scientist grunts (Silph Workers are non-battle hostages), with a
    // Pokémon-Center heal excursion after every floor to top up Vaporeon's HP *and* PP (and revive it
    // if it fainted mid-floor). InteractIfReachable skips any trainer walled off by the teleport maze.
    // Floors are reached by climbing the stairs from 1F each time (cleared trainers don't re-battle);
    // 7F is skipped for now (the rival is there — don't feed Vaporeon in under-levelled).
    let floors: &[(Map, &[MS])] = &[
        (Map::SilphCo2F, &[MS::SILPHCO2F_SCIENTIST1, MS::SILPHCO2F_SCIENTIST2, MS::SILPHCO2F_ROCKET1, MS::SILPHCO2F_ROCKET2]),
        (Map::SilphCo3F, &[MS::SILPHCO3F_ROCKET, MS::SILPHCO3F_SCIENTIST]),
        (Map::SilphCo4F, &[MS::SILPHCO4F_ROCKET1, MS::SILPHCO4F_SCIENTIST, MS::SILPHCO4F_ROCKET2]),
        (Map::SilphCo5F, &[MS::SILPHCO5F_ROCKET1, MS::SILPHCO5F_SCIENTIST, MS::SILPHCO5F_ROCKET2]),
        (Map::SilphCo6F, &[MS::SILPHCO6F_ROCKET1, MS::SILPHCO6F_SCIENTIST, MS::SILPHCO6F_ROCKET2]),
    ];
    let climb: &[Map] = &[Map::SilphCo2F, Map::SilphCo3F, Map::SilphCo4F, Map::SilphCo5F, Map::SilphCo6F];
    for (i, (floor, trainers)) in floors.iter().enumerate() {
        // Climb the stairs from 1F up to this floor (each `enter` walks one flight).
        for m in &climb[..=i] { steps.push(PolicyStep::enter(*m)); }
        for &t in *trainers { steps.push(train(t)); }
        // Heal excursion: elevator → 1F → Saffron → Pokécenter nurse → back into Silph 1F.
        steps.push(PolicyStep::enter(Map::SilphCoElevator));
        steps.push(PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 0 });
        steps.push(PolicyStep::enter(Map::SaffronCity));
        steps.push(PolicyStep::enter(Map::SaffronPokecenter));
        steps.push(PolicyStep::Interact(MS::SAFFRONPOKECENTER_NURSE));
        steps.push(PolicyStep::enter(Map::SaffronCity));
        steps.push(PolicyStep::enter(Map::SilphCo1F));
    }
    let mut fixture = TestFixture::new(include_bytes!("data/vaporeon-ready.bin"), Duration::from_mins(240), steps);
    let mut last_lv = 0u8;
    let mut last_map = Map::CeladonCity;
    for i in 0..2_500_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 1000 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if let Some(v) = s.pokemon.iter().find(|p| format!("{:?}", p.species) == "Vaporeon") {
                    if v.level != last_lv || s.map.map != last_map {
                        last_lv = v.level; last_map = s.map.map;
                        println!("  step {i}: Vaporeon lv{} hp {}/{} @ {} pos {}",
                            v.level, v.current_hp, v.stats.hp, s.map.map, s.map.player_position);
                    }
                }
            }
        }
    }
    let s = fixture.game_state();
    let v = s.pokemon.iter().find(|p| format!("{:?}", p.species) == "Vaporeon").unwrap();
    println!("final: Vaporeon lv{} exp{} on {} @ {}", v.level, v.experience, s.map.map, s.map.player_position);
    if v.level >= 28 {
        fixture.save_state_named("src/pokemon/data/vaporeon-trained.bin").unwrap();
        println!(">> saved vaporeon-trained.bin (Vaporeon lv{})", v.level);
    }
}

/// Diagnostic: from `post-silph-rival.bin` (on 7F, post-rival) ride the elevator to 11F and dump its
/// tile map, Giovanni's position, every warp/pad + destination, and reachability from the elevator
/// exit — to plan the route to Giovanni (walled off) and which guards drop the walls.
#[test]
#[ignore]
fn probe_silph11f_dump() {
    // Load the post-Giovanni state directly and dump 11F reachability (is the president reachable?).
    let mut fixture = TestFixture::new(include_bytes!("data/post-silph-giovanni.bin"), Duration::from_mins(30), vec![]);
    for _ in 0..80 { fixture.step(); }
    let s = fixture.game_state();
    println!("on {} @ {}", s.map.map, s.map.player_position);
    println!("{}", s.map);
    println!("sprites: {:?}", s.map.sprites.iter().filter(|sp| !sp.hidden).map(|sp| (sp.name, sp.position)).collect::<Vec<_>>());
    println!("Giovanni pos: {:?}", s.map.sprites.iter().find(|sp| sp.name == "Giovanni").map(|sp| (sp.position, sp.hidden)));
    println!("reachable warp/sprite actions:");
    for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..} | MetaTile::Sprite(_))) {
        println!("   {:?} dest={} route_len={}", a.tile, a.destination, a.route.len());
    }
    let w = s.map.width;
    println!("EVERY warp meta-tile (incl. walled-off pads):");
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        if let MetaTile::Warp { to_map, to_position } = t {
            println!("   ({},{}) -> {to_map} {to_position}", i % w, i / w);
        }
    }
}

/// Diagnostic: get the agent to 11F(6,14) (via pad + Interact-Giovanni, which stalls there), then take
/// MANUAL button control and probe which moves the game actually allows around the Giovanni trigger
/// tiles (6,13)/(7,12) — to find a direction the game permits and whether the trigger (curScr→3) fires.
#[test]
#[ignore]
fn probe_11f_manual() {
    use crate::pokemon::map::MapSprite as MS;
    let mut steps = vec![
        PolicyStep::EnterMap { to_map: Map::SilphCo11F, to_position: Some(Point8 { x: 3, y: 2 }) },
        PolicyStep::InteractIfReachable(MS::SILPHCO11F_ROCKET1),
    ];
    for _ in 0..6 { steps.push(PolicyStep::Interact(MS::SILPHCO11F_GIOVANNI)); }
    let mut fixture = TestFixture::new(include_bytes!("data/post-silph-rival.bin"), Duration::from_mins(60), steps);
    let read = |gb: &crate::game_boy::GameBoy, a: u16| gb.core().mmu().read(a);
    // Run the policy until the agent reaches (6,14) and settles there.
    let mut at = None;
    for _ in 0..600_000 {
        fixture.step();
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            if s.map.player_position == (Point8 { x: 6, y: 14 }) { at = Some(()); }
        }
        if at.is_some() {
            // once it's been at (6,14) for a bit, stop the policy loop
            for _ in 0..200 { fixture.step(); }
            break;
        }
    }
    let pos = |gb: &mut crate::game_boy::GameBoy, mc: &mut crate::pokemon::map_metadata::MapMetadataCache| {
        PokemonApi::with_cache(gb, mc).game_state().map(|s| s.map.player_position).unwrap_or(Point8{x:0,y:0})
    };
    println!("start manual @ {}", pos(&mut fixture.gb, &mut fixture.map_cache));
    // What the GAME reads for collision: standing tile + the four in-front screen tiles.
    let standing = read(&fixture.gb, 0xCF0E);
    let up    = read(&fixture.gb, 0xC3A0 + 7*20 + 8);
    let down  = read(&fixture.gb, 0xC3A0 + 11*20 + 8);
    let left  = read(&fixture.gb, 0xC3A0 + 9*20 + 6);
    let right = read(&fixture.gb, 0xC3A0 + 9*20 + 10);
    println!("GAME tiles: standing=0x{standing:02x} up=0x{up:02x} down=0x{down:02x} left=0x{left:02x} right=0x{right:02x}");
    if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
        let w = s.map.width;
        let raw = |x: usize, y: usize| s.map.raw_tile_ids[x + y*w];
        println!("AGENT raw: (6,13)=0x{:02x} (6,14)=0x{:02x} (7,13)=0x{:02x} (6,12)=0x{:02x}", raw(6,13), raw(6,14), raw(7,13), raw(6,12));
    }
    // Dump the GAME's on-screen tile grid (wTileMap) around the player — player standing tile is
    // screen (8,9); rows above are the chamber/trigger. Shows the true 2×2 sub-tile structure.
    println!("GAME wTileMap rows 5-13, cols 4-12 (player screen col 8):");
    for sy in 5..14usize {
        let row: Vec<String> = (4..13usize).map(|sx| format!("{:02x}", read(&fixture.gb, 0xC3A0 + (sy*20 + sx) as u16))).collect();
        println!("  sy={sy}: {}", row.join(" "));
    }
    // Try a sequence of directions, holding each for ~16 frames, logging pos + Giovanni script var.
    let dirs = [
        ("Up", JoypadButton::Up), ("Up", JoypadButton::Up),
        ("Right", JoypadButton::Right), ("Up", JoypadButton::Up), ("Up", JoypadButton::Up),
        ("Left", JoypadButton::Left), ("Left", JoypadButton::Left), ("Up", JoypadButton::Up),
    ];
    for (name, btn) in dirs {
        for _ in 0..16 {
            { let mut api = PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache); api.release_all_buttons(); api.press_button(btn); }
            fixture.step();
        }
        let p = pos(&mut fixture.gb, &mut fixture.map_cache);
        println!("  after {name}: @ {p} curScr={}", read(&fixture.gb, 0xD659));
    }
}

/// Diagnostic: re-fight Giovanni from `post-silph-rival.bin` and densely trace the IMMEDIATE post-battle
/// window (mode / pos / on-screen text / Giovanni-present) to see whether his flee dialogue+script plays
/// and where it breaks — he's supposed to vanish on defeat, which would unblock the president.
#[test]
#[ignore]
fn probe_giovanni_flee() {
    use crate::pokemon::map::MapSprite as MS;
    let mut fixture = TestFixture::new(include_bytes!("data/post-silph-rival.bin"), Duration::from_mins(60),
        vec![
            PolicyStep::EnterMap { to_map: Map::SilphCo11F, to_position: Some(Point8 { x: 3, y: 2 }) },
            PolicyStep::Interact(MS::SILPHCO11F_GIOVANNI),
        ]);
    let read = |gb: &crate::game_boy::GameBoy, a: u16| gb.core().mmu().read(a);
    let mut fought = false;
    let mut post = 0u32;
    let mut last = String::new();
    for _ in 0..600_000 {
        fixture.step();
        let cur_script = read(&fixture.gb, 0xD659);
        let is_in_battle = read(&fixture.gb, 0xD057);
        let mut api = PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache);
        let in_batt = api.game_state().map(|g| matches!(g.mode, GameMode::TrainerBattle)).unwrap_or(false);
        if in_batt { fought = true; }
        if fought && !in_batt {
            post += 1;
            if post % 5 == 0 && post < 8000 {
                let txt = api.on_screen_text(false).unwrap_or_default();
                if let Ok(s) = api.game_state() {
                    let gio = s.map.sprites.iter().any(|sp| sp.name == "Giovanni" && !sp.hidden);
                    let cur = format!("{:?} @ {} gio={gio} curScr={cur_script} inBatt={is_in_battle} st={} txt={:?}",
                        s.mode, s.map.player_position, fixture.agent.state_debug(), txt.chars().take(30).collect::<String>());
                    if cur != last { last = cur.clone(); println!("  post{post}: {cur}"); }
                }
            }
            if post > 8000 { break; }
        }
    }
    let s = fixture.game_state();
    let gio = s.map.sprites.iter().any(|sp| sp.name == "Giovanni" && !sp.hidden);
    println!("final: gio_present={gio} @ {}", s.map.player_position);
}

/// Diagnostic: from `post-silph-giovanni.bin`, talk to the Silph President and trace the liberation
/// dialogue (position / mode / on-screen text) to see where it gets stuck.
#[test]
#[ignore]
fn probe_silph_president() {
    use crate::pokemon::map::MapSprite as MS;
    let mut fixture = TestFixture::new(include_bytes!("data/post-silph-giovanni.bin"), Duration::from_mins(60),
        vec![
            // The post-battle position (3,13) can't route past Giovanni to the president, but the pad
            // entry (3,2) can. Reposition via a pad round-trip (11F→7F pocket→11F lands at (3,2)).
            PolicyStep::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 7 }) },
            PolicyStep::EnterMap { to_map: Map::SilphCo11F, to_position: Some(Point8 { x: 3, y: 2 }) },
            PolicyStep::Interact(MS::SILPHCO11F_SILPH_PRESIDENT),
        ]);
    let mut last = String::new();
    for i in 0..200_000 {
        fixture.step();
        if i % 100 == 0 {
            let mut api = PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache);
            let txt = api.on_screen_text(false).unwrap_or_default();
            if let Ok(s) = api.game_state() {
                let master = s.bag.iter().any(|it| format!("{:?}", it.id) == "MasterBall");
                let gio = s.map.sprites.iter().any(|sp| sp.name == "Giovanni" && !sp.hidden);
                let cur = format!("{:?} @ {} gio={gio} txt={:?} master={master}", s.mode, s.map.player_position, txt.chars().take(36).collect::<String>());
                if cur != last { last = cur.clone(); println!("  {i}: {cur}"); }
                if master { println!(">> Master Ball obtained at {i}"); break; }
            }
        }
    }
    let s = fixture.game_state();
    println!("final @ {} map:\n{}", s.map.player_position, s.map);
    println!("sprites: {:?}", s.map.sprites.iter().filter(|sp| !sp.hidden).map(|sp| (sp.name, sp.position)).collect::<Vec<_>>());
}

/// From `at-route2.bin` (Diglett's exit pocket on Route 2, walled by Cut trees): cut the trees, then
/// dump what warps/connections become reachable — to plan the leg south to Viridian.
#[test]
#[ignore]
fn probe_route2_postcut() {
    let steps = vec![PolicyStep::CutTree { map: Map::Route2 }];
    let mut fixture = TestFixture::new(include_bytes!("data/at-route2.bin"), Duration::from_mins(10), steps);
    let mut last = String::new();
    for i in 0..300_000 {
        fixture.step();
        let line = {
            let mmu = fixture.gb.core().mmu();
            use crate::pokemon::symbols::pokered_symbols as ps;
            format!("raw=({},{}) state={}", mmu.read_pointer(&ps::wXCoord), mmu.read_pointer(&ps::wYCoord), fixture.agent.state_debug())
        };
        if line != last { if i < 40000 { println!("  {i}: {line}"); } last = line; }
        if fixture.agent.policy_exhausted() { println!(">> cut done at {i}"); break; }
    }
    fixture.gb.save_state_to_file("src/pokemon/data/at-route2-cut.bin").ok();
    let s = fixture.game_state();
    println!("on {} @ {}", s.map.map, s.map.player_position);
    for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..} | MetaTile::Connection{..} | MetaTile::ConnectionWater(_) | MetaTile::CutTree)) {
        println!("   dest={} route_len={} tile={:?}", a.destination, a.route.len(), a.tile);
    }
}

/// Fast-iteration resume from `at-route11.bin` (agent on Route 11, past Vermilion): Diglett's Cave →
/// Route 2 → Viridian → Route 1 → Pallet → Surf to Cinnabar. Used to debug the western legs.
#[test]
#[ignore]
fn probe_route11_to_cinnabar() {
    let steps = vec![
        PolicyStep::enter(Map::DiglettsCaveRoute11),
        PolicyStep::enter(Map::DiglettsCave),
        PolicyStep::enter(Map::DiglettsCaveRoute2),
        PolicyStep::enter(Map::Route2),
        PolicyStep::CutTree { map: Map::Route2 },     // open the Cut-gated Diglett's pocket
        PolicyStep::enter(Map::Route2Gate),           // walk south to the mid-Route-2 gate
        PolicyStep::EnterMap { to_map: Map::Route2, to_position: Some(Point8 { x: 15, y: 39 }) }, // exit gate south
        PolicyStep::CutTree { map: Map::Route2 },     // more Cut trees on south Route 2
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::PalletTown),
        PolicyStep::enter(Map::Route21),              // Surf south
        PolicyStep::enter(Map::CinnabarIsland),       // Surf south
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-route11.bin"), Duration::from_mins(90), steps);
    let mut last = (Map::SaffronGym, Point8 { x: 255, y: 255 });
    let mut saved_pallet = false;
    for i in 0..2_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let surf = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wWalkBikeSurfState);
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  step {i}: {} @ {} surf={surf}", s.map.map, s.map.player_position); }
                if s.map.map == Map::PalletTown && !saved_pallet {
                    saved_pallet = true;
                    fixture.gb.save_state_to_file("src/pokemon/data/at-pallet.bin").ok();
                    println!("  *** saved at-pallet.bin ***");
                }
                if s.map.map == Map::CinnabarIsland {
                    fixture.gb.save_state_to_file("src/pokemon/data/at-cinnabar.bin").ok();
                    println!("  *** reached CINNABAR — saved at-cinnabar.bin ***");
                    break;
                }
            }
        }
    }
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
}

/// From `at-cinnabar.bin`: get the Secret Key from the Pokémon Mansion B1F (it unlocks the gym door),
/// then beat Blaine in Cinnabar Gym for the Volcano Badge. Saves `post-volcano-badge.bin`.
#[test]
#[ignore]
fn probe_cinnabar_volcano() {
    use crate::pokemon::badge::Badge;
    use crate::pokemon::map::MapSprite as MS;
    let steps = vec![
        PolicyStep::enter(Map::PokemonMansion1F),
        // The 1F switch (hidden object at (2,5), face up) toggles the gate blocking the B1F stairs.
        PolicyStep::FlipSwitch { map: Map::PokemonMansion1F, at: Point8 { x: 2, y: 5 }, reveals: Map::PokemonMansionB1F },
        PolicyStep::enter(Map::PokemonMansionB1F),
        PolicyStep::CollectItem(MS::POKEMONMANSIONB1F_SECRET_KEY),
        PolicyStep::enter(Map::PokemonMansion1F),
        PolicyStep::enter(Map::CinnabarIsland),
        PolicyStep::enter(Map::CinnabarGym),
        PolicyStep::DefeatGymLeader { leader: MS::CINNABARGYM_BLAINE, badge: Badge::VolcanoBadge },
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-cinnabar.bin"), Duration::from_mins(60), steps);
    let mut last = (Map::SaffronGym, Point8 { x: 255, y: 255 });
    for i in 0..2_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  step {i}: {} @ {} badges={:?}", s.map.map, s.map.player_position, s.badges); }
                if s.badges.contains(Badge::VolcanoBadge) {
                    fixture.gb.save_state_to_file("src/pokemon/data/post-volcano-badge.bin").ok();
                    println!("  *** VOLCANO BADGE at step {i} — saved post-volcano-badge.bin ***");
                    break;
                }
            }
        }
    }
    let s = fixture.game_state();
    println!("final: {} @ {} badges={:?}", s.map.map, s.map.player_position, s.badges);
}

/// Fast-iteration resume from `at-pallet.bin`: Surf south from Pallet → Route 21 → Cinnabar Island.
/// The final surf crossings; saves `at-cinnabar.bin`.
#[test]
#[ignore]
fn probe_pallet_to_cinnabar() {
    let steps = vec![
        PolicyStep::enter(Map::Route21),
        PolicyStep::enter(Map::CinnabarIsland),
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-pallet.bin"), Duration::from_mins(30), steps);
    let mut last = (Map::SaffronGym, Point8 { x: 255, y: 255 });
    for i in 0..1_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 100 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let surf = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wWalkBikeSurfState);
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  step {i}: {} @ {} surf={surf}", s.map.map, s.map.player_position); }
                if s.map.map == Map::CinnabarIsland {
                    fixture.gb.save_state_to_file("src/pokemon/data/at-cinnabar.bin").ok();
                    println!("  *** reached CINNABAR — saved at-cinnabar.bin ***");
                    break;
                }
            }
        }
    }
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    if s.map.map == Map::CinnabarIsland {
        fixture.gb.save_state_to_file("src/pokemon/data/at-cinnabar.bin").ok();
        println!("saved at-cinnabar.bin");
    }
    assert_eq!(s.map.map, Map::CinnabarIsland, "should have surfed to Cinnabar");
}

/// Fast-iteration resume from `at-route2-south.bin` (south of the Route 2 gate): cut the remaining trees,
/// then Viridian → Route 1 → Pallet → Surf to Cinnabar. Validates the final legs quickly.
#[test]
#[ignore]
fn probe_route2south_to_cinnabar() {
    let steps = vec![
        PolicyStep::CutTree { map: Map::Route2 },
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::PalletTown),
        PolicyStep::enter(Map::Route21),
        PolicyStep::enter(Map::CinnabarIsland),
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-route2-south.bin"), Duration::from_mins(60), steps);
    let mut last = (Map::SaffronGym, Point8 { x: 255, y: 255 });
    let mut saved_pallet = false;
    for i in 0..1_500_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let surf = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wWalkBikeSurfState);
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  step {i}: {} @ {} surf={surf}", s.map.map, s.map.player_position); }
                if s.map.map == Map::PalletTown && !saved_pallet {
                    saved_pallet = true;
                    fixture.gb.save_state_to_file("src/pokemon/data/at-pallet.bin").ok();
                    println!("  *** saved at-pallet.bin ***");
                }
                if s.map.map == Map::CinnabarIsland {
                    fixture.gb.save_state_to_file("src/pokemon/data/at-cinnabar.bin").ok();
                    println!("  *** reached CINNABAR — saved at-cinnabar.bin ***");
                    break;
                }
            }
        }
    }
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
}

/// Dump the connectivity of the current floor: player pos, reachable warps/items, all sprites, and the
/// rendered map (reachable tiles marked '+'). A reusable aid for reverse-engineering switch-gate mazes.
#[allow(dead_code)]
fn dump_floor(fixture: &mut TestFixture) {
    let s = fixture.game_state();
    let w = s.map.width;
    println!("=== on {} @ {} (badges={:?}) bag={} slots ===", s.map.map, s.map.player_position, s.badges, s.bag.iter().count());
    println!("reachable actions:");
    for a in s.map.actions().iter() {
        println!("   {:?} dest={} route_len={}", a.tile, a.destination, a.route.len());
    }
    println!("sprites: {:?}", s.map.sprites.iter().map(|sp| (sp.name, sp.position, sp.hidden)).collect::<Vec<_>>());
    println!("ALL warps on this map:");
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        if let MetaTile::Warp { to_map, to_position } = t { println!("   ({},{}) -> {to_map} {to_position}", i % w, i / w); }
    }
    // Render with reachable tiles marked '+' so the connected region is visible.
    let reach = s.map.reachable_tiles();
    println!("map (+ = reachable):");
    for y in 0..s.map.height {
        let mut line = String::new();
        for x in 0..s.map.width {
            let p = Point8 { x: x as u8, y: y as u8 };
            if p == s.map.player_position { line.push('P'); continue; }
            let ch = match s.map.meta_tiles[x + y * w] {
                MetaTile::Obstacle => 'O', MetaTile::Water => 'X', MetaTile::Sprite(_) => 'S',
                MetaTile::Warp { .. } => 'W', MetaTile::Connection { .. } => 'C',
                _ if reach.contains(&p) => '+',
                _ => '_',
            };
            line.push(ch);
        }
        println!("{line}");
    }
}

/// From `post-secret-key.bin` (B1F, Secret Key in bag): exit the mansion, heal, and clear the Cinnabar
/// Gym's quiz-gate snake maze — `DefeatGymLeader` beats each fire trainer via line of sight to unlock
/// the gate ahead — then beat Blaine for the **Volcano Badge**. Snapshots `post-volcano-badge.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_volcano_badge() {
    let steps = PolicyStep::volcano_badge_steps();
    let mut fixture = TestFixture::new(include_bytes!("data/post-secret-key.bin"), Duration::from_mins(40), steps);
    for i in 0..2_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> done at step {i}"); break; }
    }
    let s = fixture.game_state();
    println!("on {} @ {} — badges = {:?}", s.map.map, s.map.player_position, s.badges);
    assert!(s.badges.contains(Badge::VolcanoBadge), "should have won the Volcano Badge from Blaine");
    fixture.gb.save_state_to_file("src/pokemon/data/post-volcano-badge.bin").ok();
    println!(">> saved post-volcano-badge.bin");
}

/// From `post-volcano-lone.bin` (in Blaine's gym after the Volcano Badge, the full-playthrough party —
/// Venusaur + Vaporeon, 7 badges): Surf back to Pallet and up to Viridian, then clear Giovanni's
/// **Viridian Gym** spinner-tile maze for the **Earth Badge** — the 8th and final gym badge. Exercises
/// the `ViridianGym` arrow-tile table. Snapshots `post-earth-badge.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_earth_badge() {
    let steps = PolicyStep::earth_badge_steps();
    let mut fixture = TestFixture::new(include_bytes!("data/post-volcano-lone.bin"), Duration::from_mins(40), steps);
    for i in 0..2_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> done at step {i}"); break; }
    }
    let s = fixture.game_state();
    println!("on {} @ {} — badges = {:?}", s.map.map, s.map.player_position, s.badges);
    assert!(s.badges.contains(Badge::EarthBadge), "should have won the Earth Badge from Giovanni");
    fixture.gb.save_state_to_file("src/pokemon/data/post-earth-badge.bin").ok();
    println!(">> saved post-earth-badge.bin");
}

/// From `at-cinnabar.bin`: navigate the Pokémon Mansion switch-gate maze (fall through a 3F hole to
/// 1F's right side → B1F, flipping the global switch at each floor) and collect the Secret Key that
/// unlocks the Cinnabar Gym. Snapshots `post-secret-key.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_secret_key() {
    let steps = PolicyStep::mansion_secret_key_steps();
    let mut fixture = TestFixture::new(include_bytes!("data/at-cinnabar.bin"), Duration::from_mins(20), steps);
    for i in 0..1_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> done at step {i}"); break; }
    }
    let s = fixture.game_state();
    println!("on {} @ {} — bag has SecretKey = {}", s.map.map, s.map.player_position,
        s.bag.iter().any(|b| b.id == ItemId::SecretKey));
    assert!(s.bag.iter().any(|b| b.id == ItemId::SecretKey), "should have collected the Secret Key");
    fixture.gb.save_state_to_file("src/pokemon/data/post-secret-key.bin").ok();
    println!(">> saved post-secret-key.bin");
}

/// Tick-by-tick trace of the Route 6 gate crossing to see why the warp won't fire.
#[test]
#[ignore]
fn probe_route6_gate_trace() {
    use crate::pokemon::symbols::pokered_symbols as ps;
    {
        use crate::pokemon::map_metadata::MapMetadataReader;
        let gb = GameBoy::dmg(roms::POKERED);
        for gate in [Map::Route6Gate, Map::Route2Gate] {
            let meta = gb.core().mmu().read_map_metadata(gate).unwrap();
            let dims = meta.dimensions();
            let w = dims.full_width();
            println!("{gate} {}x{} north_extra={} west_extra={}", w, dims.full_height(), dims.north_extra, dims.west_extra);
            for (i, t) in meta.meta_tiles_base.iter().enumerate() {
                if let MetaTile::Warp { to_map, to_position } = t {
                    println!("   gate warp @({},{}) -> {to_map} {to_position}", i % w, i / w);
                }
            }
        }
    }
    let steps = vec![
        PolicyStep::enter(Map::Route6Gate),
        PolicyStep::EnterMap { to_map: Map::Route6, to_position: Some(Point8 { x: 10, y: 7 }) },
        PolicyStep::enter(Map::VermilionCity),
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-route6-apron.bin"), Duration::from_mins(5), steps);
    let mut last = String::new();
    for i in 0..8000 {
        fixture.step();
        let (map, rx, ry, facing) = {
            let mmu = fixture.gb.core().mmu();
            let m = mmu.read_pointer(&ps::wCurMap);
            let rx = mmu.read_pointer(&ps::wXCoord);
            let ry = mmu.read_pointer(&ps::wYCoord);
            let f = mmu.read_pointer(&ps::wPlayerDirection);
            (m, rx, ry, f)
        };
        let st = fixture.agent.state_debug();
        let line = format!("map={map} raw=({rx},{ry}) facing={facing} state={st}");
        if line != last {
            println!("  {i}: {line}");
            if map == 73 && !last.contains("map=73") {
                let s = fixture.game_state();
                println!("     -- Route6Gate actions --");
                for a in s.map.actions().iter() {
                    println!("     dest={} route_len={} tile={:?}", a.destination, a.route.len(), a.tile);
                }
            }
            last = line;
        }
        if fixture.agent.policy_exhausted() { println!(">> exhausted at {i}"); break; }
    }
}

/// Fast-iteration resume from `at-route6-apron.bin` (agent stuck in Route 6's north apron, sealed from
/// the south by the Route 6 gate building). Threads the gate, then continues to Pallet + Surfs to
/// Cinnabar. Used to debug the overland gate-crossings without re-running the Saffron→Route6 leg.
#[test]
#[ignore]
fn probe_route6_to_cinnabar() {
    let steps = vec![
        // Thread the Route 6 gate: apron → gate building → exit its SOUTH door onto Route 6 proper.
        // Route6Gate's south door statically resolves to Route6 (17,13), so target that to pick it
        // (the running game then does the correct runtime LAST_MAP warp to just south of the gate).
        PolicyStep::enter(Map::Route6Gate),
        PolicyStep::EnterMap { to_map: Map::Route6, to_position: Some(Point8 { x: 10, y: 7 }) },
        PolicyStep::enter(Map::VermilionCity),
        PolicyStep::enter(Map::Route11),
        PolicyStep::enter(Map::DiglettsCaveRoute11), // Route 11 → cave entrance room
        PolicyStep::enter(Map::DiglettsCave),        // → main cave
        PolicyStep::enter(Map::DiglettsCaveRoute2),  // → Route 2 entrance room
        PolicyStep::enter(Map::Route2),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::PalletTown),
        PolicyStep::enter(Map::Route21),
        PolicyStep::enter(Map::CinnabarIsland),
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-route6-apron.bin"), Duration::from_mins(90), steps);
    let mut last = (Map::SaffronGym, Point8 { x: 255, y: 255 });
    let mut saved_pallet = false;
    for i in 0..2_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let surf = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wWalkBikeSurfState);
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  step {i}: {} @ {} surf={surf}", s.map.map, s.map.player_position); }
                if s.map.map == Map::PalletTown && !saved_pallet {
                    saved_pallet = true;
                    fixture.gb.save_state_to_file("src/pokemon/data/at-pallet.bin").ok();
                    println!("  *** saved at-pallet.bin ***");
                }
                if s.map.map == Map::CinnabarIsland {
                    fixture.gb.save_state_to_file("src/pokemon/data/at-cinnabar.bin").ok();
                    println!("  *** reached CINNABAR — saved at-cinnabar.bin ***");
                    break;
                }
            }
        }
    }
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
}

/// From `post-marsh-badge.bin` (Saffron), trek overland to Pallet Town, then Surf south down Route 21
/// to Cinnabar Island. Exercises the new Surf pathfinding: Pallet→Route21 and Route21→Cinnabar are
/// water connections crossed by mounting Surf (Vaporeon knows it). Saves `at-pallet.bin` on the way and
/// `at-cinnabar.bin` on arrival.
#[test]
#[ignore]
fn probe_saffron_to_cinnabar() {
    let steps = vec![
        // The fixture is deep in the gym maze — thread back out to Saffron City first.
        PolicyStep::enter(Map::SaffronCity),
        // Saffron → Vermilion: Route 6 is split by a gate building; thread it (exit its south door,
        // which resolves to Route6 (10,7)).
        PolicyStep::enter(Map::Route6),
        PolicyStep::enter(Map::Route6Gate),
        PolicyStep::EnterMap { to_map: Map::Route6, to_position: Some(Point8 { x: 10, y: 7 }) },
        PolicyStep::enter(Map::VermilionCity),
        // Vermilion → Route 2 via Diglett's Cave (3 maps; the shortcut avoids Mt Moon).
        PolicyStep::enter(Map::Route11),
        PolicyStep::enter(Map::DiglettsCaveRoute11),
        PolicyStep::enter(Map::DiglettsCave),
        PolicyStep::enter(Map::DiglettsCaveRoute2),
        PolicyStep::enter(Map::Route2),
        // Route 2 is Cut-gated on both sides of its mid-route gate — cut, thread the gate, cut again.
        PolicyStep::CutTree { map: Map::Route2 },
        PolicyStep::enter(Map::Route2Gate),
        PolicyStep::EnterMap { to_map: Map::Route2, to_position: Some(Point8 { x: 15, y: 39 }) },
        PolicyStep::CutTree { map: Map::Route2 },
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::PalletTown),
        // Surf south: Pallet → Route 21 → Cinnabar Island (both water connections).
        PolicyStep::enter(Map::Route21),
        PolicyStep::enter(Map::CinnabarIsland),
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/post-marsh-badge.bin"), Duration::from_mins(120), steps);
    let mut last = (Map::SaffronGym, Point8 { x: 255, y: 255 });
    let mut saved_pallet = false;
    let mut surfed = false;
    for i in 0..3_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let surf = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wWalkBikeSurfState);
                if surf == 2 { surfed = true; }
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  step {i}: {} @ {} surf={surf}", s.map.map, s.map.player_position); }
                if s.map.map == Map::PalletTown && !saved_pallet {
                    saved_pallet = true;
                    fixture.gb.save_state_to_file("src/pokemon/data/at-pallet.bin").ok();
                    println!("  *** saved at-pallet.bin ***");
                }
                if s.map.map == Map::CinnabarIsland {
                    fixture.gb.save_state_to_file("src/pokemon/data/at-cinnabar.bin").ok();
                    println!("  *** reached CINNABAR — saved at-cinnabar.bin ***");
                    break;
                }
            }
        }
    }
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::CinnabarIsland, "should have surfed to Cinnabar Island");
    assert!(surfed, "should have mounted Surf (surf state 2) at some point");
}

/// From `at-saffron-post-silph.bin` (Saffron liberated + healed after beating Giovanni), enter the
/// Saffron Gym and beat Sabrina for the Marsh Badge. The gym is a 3×3 grid of rooms joined only by
/// teleport pads (intra-map warps); the agent solves the maze automatically because `bfs_from_player`
/// now routes *through* self-referential warp tiles (like arrow/spinner tiles). Saves
/// `post-marsh-badge.bin`.
#[test]
#[ignore]
fn probe_saffron_gym_marsh_badge() {
    use crate::pokemon::badge::Badge;
    use crate::pokemon::map::MapSprite as MS;
    let steps = vec![
        PolicyStep::enter(Map::SaffronGym),
        PolicyStep::DefeatGymLeader { leader: MS::SAFFRONGYM_SABRINA, badge: Badge::MarshBadge },
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/at-saffron-post-silph.bin"), Duration::from_mins(30), steps);
    let mut last = (Map::CeladonCity, Point8 { x: 255, y: 255 });
    for i in 0..600_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> policy exhausted at step {i}"); break; }
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let k = (s.map.map, s.map.player_position);
                if k != last { last = k; println!("  step {i}: {} @ {}", s.map.map, s.map.player_position); }
            }
        }
    }
    let s = fixture.game_state();
    println!("final: map={} @ {} party0 lv{} MarshBadge={}",
        s.map.map, s.map.player_position,
        s.pokemon.get(0).map(|p| p.level).unwrap_or(0),
        s.badges.contains(Badge::MarshBadge));
    assert!(s.badges.contains(Badge::MarshBadge), "did not obtain the Marsh Badge");
    fixture.gb.save_state_to_file("src/pokemon/data/post-marsh-badge.bin").ok();
}

/// From `post-silph-giovanni.bin` (11F), thread the pads back down out of Silph to Saffron, heal at the
/// Pokécenter, and buy a stack of Super Potions for the Sabrina fight. Saves `at-saffron-post-silph.bin`.
#[test]
#[ignore]
fn probe_exit_silph_to_saffron() {
    use crate::pokemon::map::MapSprite as MS;
    let steps = vec![
        // Talk to the Silph President (11F (7,5)) first — this is what liberates Silph AND makes the
        // Rockets leave Saffron (and hands over the Master Ball); without it the Saffron Gym door stays
        // blocked by a Rocket. Then thread the pads back out.
        PolicyStep::Interact(MS::SILPHCO11F_SILPH_PRESIDENT),
        // 11F → 7F rival pocket (via the (3,2) pad) → 3F(11,11) (via the pocket's (5,3) pad) → elevator.
        PolicyStep::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 7 }) },
        PolicyStep::EnterMap { to_map: Map::SilphCo3F, to_position: Some(Point8 { x: 11, y: 11 }) },
        PolicyStep::enter(Map::SilphCoElevator),
        PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 0 },
        PolicyStep::enter(Map::SaffronCity),
        PolicyStep::enter(Map::SaffronPokecenter),
        PolicyStep::Interact(MS::SAFFRONPOKECENTER_NURSE),
        PolicyStep::enter(Map::SaffronCity),
    ];
    let mut fixture = TestFixture::new(include_bytes!("data/post-silph-giovanni.bin"), Duration::from_mins(90), steps);
    let mut last = Map::CeladonCity;
    for i in 0..1_500_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 500 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if s.map.map != last { last = s.map.map; println!("  step {i}: @ {}", s.map.map); }
            }
        }
    }
    let s = fixture.game_state();
    println!("=== on {} @ {} money=₽{} ===", s.map.map, s.map.player_position, s.money);
    for (i, p) in s.pokemon.iter().enumerate() { println!("  {i}: {:?} lv{} hp {}/{}", p.species, p.level, p.current_hp, p.stats.hp); }
    if s.map.map == Map::SaffronCity && s.pokemon[0].current_hp == s.pokemon[0].stats.hp {
        fixture.save_state_named("src/pokemon/data/at-saffron-post-silph.bin").unwrap();
        println!(">> saved at-saffron-post-silph.bin (healed)");
    }
}

/// From `post-silph-rival.bin` (7F rival pocket, Venusaur lv50 lead / Vaporeon lv32), ride the pocket's
/// pad up to 11F(3,2) and fight Giovanni — his Ground/Rock team is 2–4× weak to Venusaur's Grass and
/// Vaporeon's Surf. Reports the outcome and saves `post-silph-giovanni.bin` on a win.
#[test]
#[ignore]
fn probe_silph_giovanni() {
    use crate::pokemon::map::MapSprite as MS;
    // Giovanni's scripted battle only fires when the player STANDS ON 11F(6,13)/(7,12) — talking to him
    // does nothing, and the after-battle script is what liberates Saffron. Rocket 1 sits in the path and
    // interrupts the single-shot Interact, so clear it first, then walk into the Giovanni trigger (the
    // route to his front (6,10) passes through (6,13)), then talk to the freed President for the liberation.
    // `Interact` is single-shot (pops when it issues the walk), so a battle/route-abort mid-walk leaves
    // it consumed. Queue MANY Giovanni interacts so one fires while the agent is in position — walking
    // toward Giovanni's front (6,10) steps through the trigger tile (6,13) and starts his scripted battle
    // (whose after-script liberates Saffron). Then many President interacts (they WAIT while unreachable,
    // firing once Giovanni moves aside) to collect the Master Ball.
    let mut steps = vec![
        PolicyStep::EnterMap { to_map: Map::SilphCo11F, to_position: Some(Point8 { x: 3, y: 2 }) },
        PolicyStep::InteractIfReachable(MS::SILPHCO11F_ROCKET1),
    ];
    for _ in 0..14 { steps.push(PolicyStep::Interact(MS::SILPHCO11F_GIOVANNI)); }
    let mut fixture = TestFixture::new(include_bytes!("data/post-silph-rival.bin"), Duration::from_mins(120), steps);
    let read = |gb: &crate::game_boy::GameBoy, a: u16| gb.core().mmu().read(a);
    let mut last = String::new();
    let mut max_script = 0u8;
    let mut done_at = None;
    for i in 0..2_000_000 {
        fixture.step();
        let cur_script = read(&fixture.gb, 0xD659);
        max_script = max_script.max(cur_script);
        // The whole Giovanni fight + liberation is done once his after-battle script (curScr=5) has run
        // to completion (back to 0) in the overworld — that's when TeamRocketLeavesScript frees Saffron.
        if max_script >= 5 && cur_script == 0 && done_at.is_none() {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if matches!(s.mode, GameMode::Overworld) { done_at = Some(i); }
            }
        }
        if let Some(d) = done_at { if i > d + 600 { break; } }
        if i % (if i > 5500 { 30 } else { 300 }) == 0 {
            let st = fixture.agent.state_debug();
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let cur = format!("{:?} @ {} curScr={cur_script} maxScr={max_script} st={st}", s.mode, s.map.player_position);
                if cur != last { last = cur.clone(); println!("  step {i}: {cur}"); }
            }
        }
    }
    let s = fixture.game_state();
    let beat = read(&fixture.gb, 0xD838); // wEventFlags byte holding EVENT_BEAT_SILPH_CO_GIOVANNI
    println!("=== on {} @ {} maxScript={max_script} eventByte=0x{beat:02x} ===", s.map.map, s.map.player_position);
    for (i, p) in s.pokemon.iter().enumerate() { println!("  {i}: {:?} lv{} hp {}/{}", p.species, p.level, p.current_hp, p.stats.hp); }
    if max_script >= 5 {
        fixture.save_state_named("src/pokemon/data/post-silph-giovanni.bin").unwrap();
        println!(">> saved post-silph-giovanni.bin (Giovanni BEATEN + Saffron liberated)");
    }
}

/// Diagnostic: climb to 7F and dump its tile map, the Rival's position, every warp/teleport-pad (with
/// destination + reachability), and the reachable action set from the stairs entry — to plan the
/// teleport-maze route to the Rival (unreachable by walking from the entry).
#[test]
#[ignore]
fn probe_silph7f_dump() {
    let mut steps = vec![];
    for m in [Map::SilphCo2F, Map::SilphCo3F, Map::SilphCo4F, Map::SilphCo5F, Map::SilphCo6F, Map::SilphCo7F] {
        steps.push(PolicyStep::enter(m));
    }
    let mut fixture = TestFixture::new(include_bytes!("data/vaporeon-trained.bin"), Duration::from_mins(30), steps);
    for _ in 0..400_000 {
        fixture.step();
        let cur = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wCurMap);
        if cur == 0xE9 && fixture.agent.policy_exhausted() { break; } // SilphCo7F id 0xE9-ish; also stop on exhaust
        if fixture.agent.policy_exhausted() { break; }
    }
    for _ in 0..80 { fixture.step(); }
    let s = fixture.game_state();
    println!("on {} @ {}", s.map.map, s.map.player_position);
    println!("{}", s.map);
    println!("sprites: {:?}", s.map.sprites.iter().filter(|sp| !sp.hidden).map(|sp| (sp.name, sp.position)).collect::<Vec<_>>());
    println!("Rival raw pos: {:?}", s.map.sprites.iter().find(|sp| sp.name == "Rival").map(|sp| (sp.position, sp.hidden)));
    println!("reachable warp/sprite actions:");
    for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..} | MetaTile::Sprite(_))) {
        println!("   {:?} dest={} route_len={}", a.tile, a.destination, a.route.len());
    }
    println!("ALL warps/connections (reachable or not):");
    for (p, t) in s.map.all_reachable_warps_and_connections() {
        println!("   {p} -> {t:?}");
    }
    // Teleport-pad tiles ($58/$20/$43) anywhere on the map.
    let w = s.map.width;
    print!("teleport-pad cells:");
    for (i, &t) in s.map.raw_tile_ids.iter().enumerate() {
        if t == 0x58 || t == 0x20 || t == 0x43 { print!(" ({},{})=0x{:02x}", i % w, i / w, t); }
    }
    println!();
    println!("EVERY warp meta-tile (incl. walled-off pads):");
    for (i, t) in s.map.meta_tiles.iter().enumerate() {
        if let MetaTile::Warp { to_map, to_position } = t {
            println!("   ({},{}) -> {to_map} {to_position}", i % w, i / w);
        }
    }
}

/// From `vaporeon-trained.bin` (Silph 1F, Vaporeon lv32 lead + Venusaur lv48), climb to 7F and fight
/// the rival. Vaporeon's Surf hard-counters the rival's Charizard ace (Fire/Flying); with Venusaur as
/// the second body the two-mon team may already clear the fight that lone Venusaur lost. Reports the
/// party after, and whether the rival was beaten (7F leads to the Card-Key doors / Lapras beyond).
#[test]
#[ignore]
fn probe_silph_7f_rival() {
    use crate::pokemon::map::MapSprite as MS;
    // The rival (7F (3,7)) is in a pocket the stairs entry can't reach; it's served by the teleport pad
    // 7F(5,3), whose partner is 3F(11,11). So climb to 3F, step on the 3F(11,11) pad (EnterMap → 7F
    // landing at (5,3)), which drops us in the rival's pocket, then walk into the rival.
    let mut steps = vec![
        // Lead with Venusaur (lv49, well above the rival's team) so it sweeps; Vaporeon (slot 1 after
        // this) comes in as the second body / Charizard answer. In vaporeon-trained.bin the party is
        // [Vaporeon, Venusaur, Pidgey], so Venusaur is slot 1.
        PolicyStep::MovePokemonToFront { slot: 1 },
        PolicyStep::enter(Map::SilphCo2F),
        PolicyStep::enter(Map::SilphCo3F),
        PolicyStep::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 3 }) },
        PolicyStep::Interact(MS::SILPHCO7F_RIVAL),
    ];
    let _ = &steps;
    let mut fixture = TestFixture::new(include_bytes!("data/vaporeon-trained.bin"), Duration::from_mins(120), steps);
    let mut last = (Map::CeladonCity, GameMode::Overworld, 0u8);
    let mut fought = false;
    let mut battle_ended_seen = false;
    // Keep stepping past queue-exhaustion so the rival battle (triggered after Interact pops) fully
    // resolves. Stop once we've seen a battle and then returned to the overworld (win or black-out).
    for i in 0..1_500_000 {
        fixture.step();
        if i % 200 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let in_batt = matches!(s.mode, GameMode::TrainerBattle);
                if in_batt { fought = true; }
                if fought && !in_batt && s.mode == GameMode::Overworld { battle_ended_seen = true; }
                let vhp = s.pokemon.get(0).map(|p| p.current_hp).unwrap_or(0) as u8;
                let key = (s.map.map, s.mode, vhp.min(1));
                if key != last {
                    last = key;
                    println!("  step {i}: @ {} mode={:?} lead_hp={}", s.map.map, s.mode, s.pokemon.get(0).map(|p| p.current_hp).unwrap_or(0));
                }
                if battle_ended_seen && s.mode == GameMode::Overworld {
                    // give it a moment to settle, then stop
                    println!(">> battle resolved by step {i}");
                    break;
                }
            }
        }
    }
    let s = fixture.game_state();
    println!("=== after rival attempt (fought={fought}) on {} ===", s.map.map);
    for (i, p) in s.pokemon.iter().enumerate() {
        println!("  {i}: {:?} lv{} hp {}/{}", p.species, p.level, p.current_hp, p.stats.hp);
    }
    // A win = still on a Silph floor with at least one mon standing (a loss black-outs to a Pokécenter).
    let won = matches!(s.map.map, Map::SilphCo7F) && s.pokemon.iter().any(|p| p.current_hp > 0);
    println!("RIVAL BEATEN: {won}");
    if won {
        fixture.save_state_named("src/pokemon/data/post-silph-rival.bin").unwrap();
        println!(">> saved post-silph-rival.bin");
    }
}

/// Test whether the Silph Co 9F "Nurse" heals the party (like a Pokémon Center) and is reachable —
/// if so it's a free in-building heal point for training. Damages Vaporeon on 2F, then routes to 9F
/// and talks to the nurse; logs HP before/after.
#[test]
#[ignore]
fn probe_silph_9f_nurse() {
    use crate::pokemon::map::MapSprite as MS;
    let mut fixture = TestFixture::new(include_bytes!("data/vaporeon-in-silph.bin"), Duration::from_mins(40),
        vec![
            PolicyStep::SetTrainSlot(Some(2)),
            PolicyStep::InteractIfReachable(MS::SILPHCO2F_SCIENTIST1),
            PolicyStep::InteractIfReachable(MS::SILPHCO2F_ROCKET1),
            PolicyStep::SetTrainSlot(None),
            PolicyStep::enter(Map::SilphCo9F),
            PolicyStep::Interact(MS::SILPHCO9F_NURSE),
        ]);
    let mut last = (0u8, 0u16, Map::CeladonCity);
    for i in 0..800_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 300 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if let Some(v) = s.pokemon.get(2) {
                    let cur = (v.level, v.current_hp, s.map.map);
                    if cur != last { last = cur;
                        println!("  step {i}: Vaporeon lv{} hp {}/{} @ {} pos {}", v.level, v.current_hp, v.stats.hp, s.map.map, s.map.player_position);
                    }
                }
            }
        }
    }
    let s = fixture.game_state();
    let v = &s.pokemon[2];
    println!("final: Vaporeon hp {}/{} on {}", v.current_hp, v.stats.hp, s.map.map);
}

/// Fast iteration of the Silph floor-training from `vaporeon-in-silph.bin` (already on 2F with 9 Super
/// Potions), skipping the ~3000-step Celadon→Silph prefix. Logs Vaporeon HP + Super Potion count so we
/// can see whether the battle-heal keeps it alive across floors.
///
/// Test a Pokémon-Center heal excursion between floors (restores HP *and* PP, unlike potions): from
/// `vaporeon-in-silph.bin` train 2F, ride the elevator out to Saffron, heal at the Pokécenter, climb
/// back up, and train 3F. Verifies the excursion restores Vaporeon and the loop doesn't stall.
#[test]
#[ignore]
fn probe_silph_heal_loop() {
    use crate::pokemon::map::MapSprite as MS;
    // Elevator-out → Saffron Pokécenter heal → back into Silph 1F. `panel (3,0)` / `floor 0` = 1F.
    let heal_excursion = || vec![
        PolicyStep::enter(Map::SilphCoElevator),
        PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 0 },
        PolicyStep::enter(Map::SaffronCity),
        PolicyStep::enter(Map::SaffronPokecenter),
        PolicyStep::Interact(MS::SAFFRONPOKECENTER_NURSE),
        PolicyStep::enter(Map::SaffronCity),
        PolicyStep::enter(Map::SilphCo1F),
    ];
    let mut steps = vec![PolicyStep::MovePokemonToFront { slot: 2 }];
    for s in [MS::SILPHCO2F_SCIENTIST1, MS::SILPHCO2F_SCIENTIST2, MS::SILPHCO2F_ROCKET1, MS::SILPHCO2F_ROCKET2] {
        steps.push(PolicyStep::InteractIfReachable(s));
    }
    steps.extend(heal_excursion());
    steps.push(PolicyStep::enter(Map::SilphCo2F));
    steps.push(PolicyStep::enter(Map::SilphCo3F));
    for s in [MS::SILPHCO3F_ROCKET, MS::SILPHCO3F_SCIENTIST] { steps.push(PolicyStep::InteractIfReachable(s)); }
    steps.extend(heal_excursion());
    let mut fixture = TestFixture::new(include_bytes!("data/vaporeon-in-silph.bin"), Duration::from_mins(90), steps);
    let vap = |s: &GameState| s.pokemon.iter().find(|p| format!("{:?}", p.species) == "Vaporeon").cloned();
    let mut last = (0u8, 0u16, Map::CeladonCity);
    for i in 0..2_000_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 500 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                if let Some(v) = vap(&s) {
                    let surf_pp = v.moves.iter().flatten().find(|m| format!("{:?}", m.name) == "Surf").map(|m| m.pp).unwrap_or(0);
                    let cur = (v.level, v.current_hp, s.map.map);
                    if cur != last {
                        last = cur;
                        println!("  step {i}: Vaporeon lv{} hp {}/{} SurfPP={surf_pp} @ {}", v.level, v.current_hp, v.stats.hp, s.map.map);
                    }
                }
            }
        }
    }
    let s = fixture.game_state();
    let v = vap(&s).unwrap();
    let surf_pp = v.moves.iter().flatten().find(|m| format!("{:?}", m.name) == "Surf").map(|m| m.pp).unwrap_or(0);
    println!("final: Vaporeon lv{} hp {}/{} SurfPP={surf_pp} on {}", v.level, v.current_hp, v.stats.hp, s.map.map);
}

#[test]
#[ignore]
fn probe_silph_train_fast() {
    use crate::pokemon::map::MapSprite as MS;
    let train = |s: MS| PolicyStep::InteractIfReachable(s);
    // Make Vaporeon (slot 2) the lead so it fights — and levels — from the start of every battle;
    // no in-battle switch-in needed.
    let mut steps = vec![PolicyStep::MovePokemonToFront { slot: 2 }];
    for s in [MS::SILPHCO2F_SCIENTIST1, MS::SILPHCO2F_SCIENTIST2, MS::SILPHCO2F_ROCKET1, MS::SILPHCO2F_ROCKET2] { steps.push(train(s)); }
    steps.push(PolicyStep::enter(Map::SilphCo3F));
    for s in [MS::SILPHCO3F_ROCKET, MS::SILPHCO3F_SCIENTIST] { steps.push(train(s)); }
    steps.push(PolicyStep::enter(Map::SilphCo4F));
    for s in [MS::SILPHCO4F_ROCKET1, MS::SILPHCO4F_SCIENTIST, MS::SILPHCO4F_ROCKET2] { steps.push(train(s)); }
    let mut fixture = TestFixture::new(include_bytes!("data/vaporeon-in-silph.bin"), Duration::from_mins(60), steps);
    let vap = |s: &GameState| s.pokemon.iter().find(|p| format!("{:?}", p.species) == "Vaporeon").cloned();
    let mut last = (0u8, 0u8, 0u16, Map::CeladonCity);
    for i in 0..1_500_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if i % 300 == 0 {
            if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
                let sp = s.bag.iter().find(|it| it.id == ItemId::SuperPotion).map(|it| it.quantity).unwrap_or(0);
                if let Some(v) = vap(&s) {
                    let cur = (v.level, sp, v.current_hp, s.map.map);
                    if cur != last {
                        last = cur;
                        println!("  step {i}: Vaporeon lv{} hp {}/{} superpotions={sp} @ {} (party0={:?})",
                            v.level, v.current_hp, v.stats.hp, s.map.map, s.pokemon.get(0).map(|p| p.species));
                    }
                }
            }
        }
    }
    let s = fixture.game_state();
    let v = vap(&s).unwrap();
    println!("final: Vaporeon lv{} exp{} hp {}/{} on {}", v.level, v.experience, v.current_hp, v.stats.hp, s.map.map);
}

/// Diagnostic: from `silph-card-key.bin` (in the 5F Card Key pocket), step out via the (9,15) pad to
/// 9F and dump which warps/sprites are walkable-reachable there — to plan the route up to 7F (Lapras)
/// and 11F (Giovanni) now that the Card Key doors are open.
#[test]
#[ignore]
fn probe_silph_post_cardkey() {
    let target = Map::SaffronCity;
    let mut fixture = TestFixture::new(include_bytes!("data/silph-card-key.bin"), Duration::from_mins(90),
        vec![
            // Exit Silph → Saffron.
            PolicyStep::enter(Map::SilphCo9F), PolicyStep::enter(Map::SilphCoElevator),
            PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 0 }, // 1F
            PolicyStep::enter(Map::SaffronCity),
            // Saffron → Celadon through the Route-7 gate. The plain Saffron→Route7 connection lands in a
            // lower pocket sealed off from the gate by one-way ledges, so cross next to the east gate
            // door (ROM (19,10), the tile just east of the gate warp).
            PolicyStep::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 19, y: 10 }) },
            PolicyStep::enter(Map::Route7Gate),
            PolicyStep::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 11, y: 10 }) },
            PolicyStep::enter(Map::CeladonCity),
            // Grab the free Eevee from the Celadon Mansion roof house. Use the BACK entrance (Celadon
            // (24,3) → 1F (4,0)); the front door (24,9)→(4,11) is the dead-end condos. The back door
            // reaches the stairwell that climbs to the roof.
            PolicyStep::EnterMap { to_map: Map::CeladonMansion1F, to_position: Some(Point8 { x: 4, y: 0 }) },
            PolicyStep::enter(Map::CeladonMansion2F),
            PolicyStep::enter(Map::CeladonMansion3F),
            PolicyStep::enter(Map::CeladonMansionRoof),
            PolicyStep::enter(Map::CeladonMansionRoofHouse),
            PolicyStep::CollectItem(crate::pokemon::map::MapSprite::CELADONMANSION_ROOF_HOUSE_EEVEE_POKEBALL),
            PolicyStep::enter(Map::CeladonMansionRoof),
            PolicyStep::enter(Map::CeladonMansion3F),
            PolicyStep::enter(Map::CeladonMansion2F),
            PolicyStep::EnterMap { to_map: Map::CeladonMansion1F, to_position: Some(Point8 { x: 4, y: 0 }) },
            PolicyStep::enter(Map::CeladonCity),
            // Dept Store 4F: buy a Water Stone (to evolve Eevee → Vaporeon).
            PolicyStep::enter(Map::CeladonMart1F),
            PolicyStep::enter(Map::CeladonMart2F),
            PolicyStep::enter(Map::CeladonMart3F),
            PolicyStep::enter(Map::CeladonMart4F),
            PolicyStep::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::WaterStone, 1), map: Map::CeladonMart4F },
            PolicyStep::enter(Map::CeladonMart1F),
            PolicyStep::enter(Map::CeladonCity),
        ]);
    let _ = target;
    let mut last = Point8 { x: 255, y: 255 };
    let mut same = 0;
    let mut dumped: std::collections::HashSet<Map> = std::collections::HashSet::new();
    for i in 0..800_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            let p = s.map.player_position;
            if matches!(s.map.map, Map::CeladonMansion1F|Map::CeladonMansion2F|Map::CeladonMansion3F|Map::CeladonMansionRoof) && dumped.insert(s.map.map) {
                println!("=== {} @ {p} ===\n{}", s.map.map, s.map);
                for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..})) {
                    println!("   {:?} dest={} route_len={}", a.tile, a.destination, a.route.len());
                }
            }
            if p != last { last = p; same = 0;
                if s.map.map == Map::Route7 || s.map.map == Map::Route7Gate { println!("  {} @ {p} facing {:?}", s.map.map, s.map.player_direction); }
                else if i % 300 == 0 { println!("  step {i}: {} @ {p}", s.map.map); } }
            else if s.mode == GameMode::Overworld { same += 1; if same == 4_000 {
                println!(">> STUCK (overworld) at {} @ {p} state={}", s.map.map, fixture.agent.state_debug());
                println!("{}", s.map);
                for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..} | MetaTile::Connection{..})) {
                    println!("   {:?} dest={} route_len={}", a.tile, a.destination, a.route.len());
                }
                break;
            } }
        }
    }
    if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
        let stones = s.bag.iter().find(|it| it.id == ItemId::WaterStone).map(|it| it.quantity).unwrap_or(0);
        println!("ended on {} @ {} — WaterStone ×{} exhausted={}",
            s.map.map, s.map.player_position, stones, fixture.agent.policy_exhausted());
        for (i, p) in s.pokemon.iter().enumerate() { println!("   {i}: {:?} lv{} hp {}/{}", p.species, p.level, p.current_hp, p.stats.hp); }
        if s.pokemon.iter().any(|p| format!("{:?}", p.species) == "Eevee") && s.bag.iter().any(|it| it.id == ItemId::WaterStone) {
            fixture.save_state_named("src/pokemon/data/eevee-and-stone.bin").unwrap();
            println!(">> saved eevee-and-stone.bin");
        }
    }
}

/// Diagnostic: dumps Silph Co 1F warps, then rides the elevator to 5F and prints which sprites/warps
/// are walkable-reachable from the exit — used to confirm the Card Key pocket needs teleport-maze
/// routing (only teleport-pad warps + trainer-guarded entrances are reachable by walking).
#[test]
#[ignore]
fn probe_silph1f() {
    let mut fixture = TestFixture::new(include_bytes!("data/at-saffron.bin"), Duration::from_mins(10), vec![
        PolicyStep::enter(Map::SilphCo1F),
    ]);
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("on {} @ {}", s.map.map, s.map.player_position);
    println!("{}", s.map);
    println!("reachable warps:");
    for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..})) {
        println!("   dest={} route_len={} tile={:?}", a.destination, a.route.len(), a.tile);
    }
    // Scan for warp-pad tiles ($20 in FACILITY, bottom-left standing sub-tile).
    let w = s.map.width;
    print!("$20 warp-pad cells (raw bottom-left):");
    for (i, &t) in s.map.raw_tile_ids.iter().enumerate() {
        if t == 0x43 || t == 0x58 || t == 0x20 { print!(" ({},{})=0x{:02x}", i % w, i / w, t); }
    }
    println!();
    // Ride the elevator to 11F and dump whether Giovanni is walkable-reachable from the exit.
    let mut fixture = TestFixture::new(include_bytes!("data/at-saffron.bin"), Duration::from_mins(8), vec![
        PolicyStep::enter(Map::SilphCo1F),
        PolicyStep::enter(Map::SilphCoElevator),
        PolicyStep::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 10 }, // 11F
    ]);
    for _ in 0..200_000 {
        fixture.step();
        let cur = fixture.gb.core().mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wCurMap);
        if cur == 0xEB { break; } // SilphCo11F
    }
    for _ in 0..40 { fixture.step(); } // settle
    match { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
        Err(e) => println!(">> 11F game_state ERR: {e}"),
        Ok(s) => {
            println!("on {} @ {}", s.map.map, s.map.player_position);
            println!("{}", s.map);
            println!("sprites: {:?}", s.map.sprites.iter().filter(|sp| !sp.hidden).map(|sp| (sp.name, sp.position)).collect::<Vec<_>>());
            println!("reachable sprite/warp actions:");
            for a in s.map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Sprite(_) | MetaTile::Warp{..})) {
                println!("   {:?} dest={} route_len={}", a.tile, a.destination, a.route.len());
            }
        }
    }
}

#[test]
#[ignore]
fn probe_route13_to_14() {
    // From post-snorlax, cross to Route 13 and dump EVERY reachable Route-14 connection tile (with its
    // landing to_position), to see whether the agent can cross at an open Route-14 row (8/10) instead of
    // the row-6 trainer pocket.
    let mut fixture = TestFixture::new(include_bytes!("data/post-snorlax.bin"), Duration::from_mins(20), vec![
        PolicyStep::enter(Map::Route13),
    ]);
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("on {} @ {}", s.map.map, s.map.player_position);
    println!("reachable Route14 connections:");
    for (p, t) in s.map.all_reachable_warps_and_connections() {
        if let MetaTile::Connection { to_map: Map::Route14, to_position } = t {
            println!("   cross at {p} -> Route14 lands {to_position}");
        }
    }
    // Also show all reachable connection targets for context.
    println!("all reachable warps/connections:");
    for (p, t) in s.map.all_reachable_warps_and_connections() {
        println!("   {p} -> {t:?}");
    }
}

#[test]
#[ignore]
fn probe_stall_dump() {
    // Generic: load the last saved stall state and dump map / player / sprites / reachable actions.
    let bytes = std::fs::read("test_stall_state.bin").expect("stall state");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    {
        let mut api = PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache);
        let mode = api.game_state().map(|g| g.mode);
        println!("mode={mode:?} menu_state={:?}", api.menu_state());
        println!("on_screen_text(false)={:?}", api.on_screen_text(false));
        println!("on_screen_text(true)={:?}", api.on_screen_text(true));
    }
    println!("agent state: {}", fixture.agent.state_debug());
    {
        let mut api = PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache);
        if let Ok(g) = api.game_state() { if let Some(b) = &g.battle {
            println!("battle: active_slot={} player={:?} enemy lv{} {:?}", b.active_party_slot, b.player.species, b.enemy.level, b.enemy.species);
        }}
    }
    let s = fixture.game_state();
    println!("map={} player={} facing={:?}", s.map.map, s.map.player_position, s.map.player_direction);
    println!("{}", s.map);
    println!("sprites:");
    for sp in s.map.sprites.iter() {
        println!("   {} @ {} hidden={} on_screen={}", sp.name, sp.position, sp.hidden, sp.on_screen);
    }
    println!("reachable actions:");
    for a in s.map.actions().iter() {
        println!("   dest={} route_len={} tile={:?}", a.destination, a.route.len(), a.tile);
    }
    let w = s.map.width;
    let (px, py) = (s.map.player_position.x as usize, s.map.player_position.y as usize);
    for y in py.saturating_sub(2)..(py+6).min(s.map.height as usize) {
        for x in px.saturating_sub(6)..(px+3).min(w) {
            print!("({x},{y})=0x{:02x}:{:?}  ", s.map.raw_tile_ids[x + y*w], s.map.meta_tiles[x + y*w]);
        }
        println!();
    }
}

#[test]
#[ignore]
fn probe_elevator_fresh() {
    // Reproduce the REAL elevator path (fresh entry from B2F) and log wTextBoxID/wCurrentMenuItem +
    // player pos + agent state each step while in the elevator, to see the exact selection sequence.
    let mut fixture = TestFixture::new(include_bytes!("data/rocket-hideout-lift-key.bin"), Duration::from_mins(10), vec![
        PolicyStep::enter(Map::RocketHideoutB3F),
        PolicyStep::enter(Map::RocketHideoutB2F),
        PolicyStep::enter(Map::RocketHideoutElevator),
        PolicyStep::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 2 },
    ]);
    // NOTE: do NOT pimp — `pimp_pokemon` rewrites the bag and drops the Lift Key, sending the elevator
    // down the "appears to need a key" path where the floor menu never opens.
    use crate::pokemon::symbols::pokered_symbols as ps;
    let mut in_elev = false;
    let mut last = String::new();
    for _ in 0..400_000 {
        fixture.step();
        let s = fixture.game_state();
        if in_elev && s.map.map == Map::RocketHideoutB4F { println!("REACHED B4F @ {}", s.map.player_position); break; }
        if s.map.map == Map::RocketHideoutElevator {
            in_elev = true;
            let mmu = fixture.gb.core().mmu();
            let line = format!("pos={} state={} mode={:?} wTextBoxID=0x{:02x} wListMenuID=0x{:02x} wCurMenuItem={} warps={:02x}{:02x}",
                s.map.player_position, fixture.agent.state_debug(), s.mode,
                mmu.read_pointer(&ps::wTextBoxID), mmu.read_pointer(&ps::wListMenuID), mmu.read_pointer(&ps::wCurrentMenuItem),
                mmu.read(ps::wWarpEntries.address + 2), mmu.read(ps::wWarpEntries.address + 3));
            if line != last { println!("{line}"); last = line; }
        } else if in_elev {
            println!("LEFT elevator to {} @ {}", s.map.map, s.map.player_position); break;
        }
    }
}

#[test]
#[ignore]
fn probe_elevator_stall() {
    // Load the saved elevator stall state and dump map/menu/warp actions to see why the ride-out jams.
    let bytes = std::fs::read("test_stall_state.bin").expect("stall state");
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    println!("map={} player={} facing={:?}", s.map.map, s.map.player_position, s.map.player_direction);
    println!("{}", s.map);
    {
        use crate::pokemon::symbols::pokered_symbols as ps;
        let mmu = fixture.gb.core().mmu();
        println!("wCurMap=0x{:02x} wIsInBattle=0x{:02x} wJoyIgnore=0x{:02x} wCurrentMapScriptFlags=0x{:02x}",
            mmu.read_pointer(&ps::wCurMap), mmu.read(0xd057), mmu.read_pointer(&ps::wJoyIgnore),
            mmu.read(0xd5a4));
        // wWarpEntries dump (destination redirection)
        print!("wWarpEntries: ");
        for i in 0..16u16 { print!("{:02x} ", mmu.read(ps::wWarpEntries.address + i)); }
        println!();
    }
    println!("warp actions:");
    for a in fixture.game_state().map.actions().iter().filter(|a| matches!(a.tile, MetaTile::Warp{..})) {
        println!("   dest={} route={:?} tile={:?}", a.destination, a.route, a.tile);
    }
    // Replay the elevator step from this state and log map/pos/state each step.
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(2), vec![
        PolicyStep::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 2 },
    ]);
    let mut last = (Map::RocketHideoutElevator, Point8 { x: 255, y: 255 }, String::new());
    for i in 0..3000 {
        fixture.step();
        let s = fixture.game_state();
        let st = fixture.agent.state_debug();
        let cur = (s.map.map, s.map.player_position, st.clone());
        if cur != last { println!("[{i}] map={} pos={} state={}", s.map.map, s.map.player_position, st); last = cur; }
        if s.map.map == Map::RocketHideoutB4F { println!("REACHED B4F @ {}", s.map.player_position); break; }
    }
}

#[test]
#[ignore]
fn probe_b4f_giovanni() {
    // Load the post-lift-key state (on B4F) and dump the full B4F map + reachable actions, to decide
    // whether Giovanni @ (25,3) can be reached by pure navigation (fighting Rocket1/2 to open the B4F
    // door block) instead of the elevator.
    let mut fixture = TestFixture::new(include_bytes!("data/rocket-hideout-lift-key.bin"), Duration::from_mins(5), vec![]);
    fixture.pimp_pokemon();
    for _ in 0..2000 { fixture.step(); }
    let s = fixture.game_state();
    println!("state: {} @ {} facing {:?}", s.map.map, s.map.player_position, s.map.player_direction);
    {
        use crate::pokemon::symbols::pokered_symbols as ps;
        let mmu = fixture.gb.core().mmu();
        // EVENT_BEAT_ROCKET_HIDEOUT_4_TRAINER_0/1 and EVENT_ROCKET_HIDEOUT_4_DOOR_UNLOCKED
        let ev = |i: u16| mmu.read(ps::wEventFlags.address + i);
        for byte in 0..0x140u16 { let v = ev(byte); if v != 0 { print!("ev[{byte}]=0x{v:02x} "); } }
        println!();
    }
    println!("{}", s.map);
    println!("B4F sprites:");
    for sp in s.map.sprites.iter() {
        println!("   {} @ {} hidden={} on_screen={}", sp.name, sp.position, sp.hidden, sp.on_screen);
    }
    println!("reachable actions:");
    for a in s.map.actions().iter() {
        println!("   dest={} tile={:?}", a.destination, a.tile);
    }
}

#[test]
#[ignore]
fn probe_rocket_hideout_spinners() {
    // From B1F, descend through the spinner floors B2F/B3F to B4F (Giovanni's floor). Pimped party so
    // trainer battles don't cloud the navigation test. Reports how far the spinner routing gets.
    let mut fixture = TestFixture::new(include_bytes!("data/at-rocket-hideout.bin"), Duration::from_mins(30), vec![
        PolicyStep::enter(Map::RocketHideoutB2F),
        PolicyStep::enter(Map::RocketHideoutB3F),
        PolicyStep::enter(Map::RocketHideoutB4F),
    ]);
    fixture.pimp_pokemon();
    let mut last = fixture.game_state().map.map;
    for _ in 0..400_000 {
        fixture.step();
        let m = fixture.game_state().map.map;
        if m != last { println!("--> reached {m} @ {}", fixture.game_state().map.player_position); last = m; }
        if m == Map::RocketHideoutB4F { break; }
    }
    for _ in 0..3000 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    println!("{}", s.map);
    println!("B4F sprites:");
    for sp in s.map.sprites.iter() {
        println!("   {} @ {} hidden={} on_screen={}", sp.name, sp.position, sp.hidden, sp.on_screen);
    }
    println!("reachable actions from {}:", s.map.player_position);
    for a in s.map.actions().iter() {
        println!("   dest={} tile={:?}", a.destination, a.tile);
    }
}

#[test]
#[ignore]
fn probe_gamecorner_fresh() {
    // From a fresh pre-gym Celadon state, walk to the Game Corner and dump the Rocket sprite state
    // + the poster-approach tile (9,5) the moment we arrive — to see whether the Rocket is shown and
    // blocking the poster, or genuinely hidden.
    let mut fixture = TestFixture::new(include_bytes!("data/at-celadon.bin"), Duration::from_mins(15), vec![
        PolicyStep::CutTree { map: Map::CeladonCity },
        PolicyStep::enter(Map::GameCorner),
    ]);
    for _ in 0..200_000 {
        fixture.step();
        if fixture.game_state().map.map == Map::GameCorner
            && fixture.agent.policy_steps_remaining().map_or(true, |n| n == 0) { break; }
    }
    for _ in 0..2000 { fixture.step(); }
    let s = fixture.game_state();
    println!("=== {} @ {} ===", s.map.map, s.map.player_position);
    let w = s.map.width;
    let at = |x: usize, y: usize| s.map.meta_tiles.get(x + y*w).copied();
    println!("meta_tile (9,4)={:?} (9,5)={:?} (9,6)={:?}", at(9,4), at(9,5), at(9,6));
    for sp in s.map.sprites.iter().filter(|sp| sp.name == "Rocket") {
        println!("Rocket @ {} hidden={} on_screen={} pic={:?}", sp.position, sp.hidden, sp.on_screen, sp.picture_id);
    }
    println!("route_to_face((9,4)) = {:?}", s.map.route_to_face(Point8 { x: 9, y: 4 }));
}

#[test]
#[ignore]
fn probe_gamecorner_stall() {
    let mut fixture = TestFixture::new(
        &std::fs::read("test_stall_state.bin").unwrap(), Duration::from_mins(5), vec![]);
    let s = fixture.game_state();
    println!("=== {} @ {} facing {:?} ===", s.map.map, s.map.player_position, s.map.player_direction);
    println!("{}", s.map);
    println!("tile in front: {:?}", s.map.tile_in_front());
    println!("sprites:");
    for sp in s.map.sprites.iter() {
        println!("   {} @ {} hidden={} on_screen={}", sp.name, sp.position, sp.hidden, sp.on_screen);
    }
    println!("actions:");
    for a in s.map.actions() {
        println!("   dest={} tile={:?} route={:?}", a.destination, a.tile, a.route);
    }
}

#[test]
#[ignore]
fn probe_celadon_exit_stall() {
    // Load the saved stuck state (post-Erika, trying to exit the gym) and dump the situation.
    let mut fixture = TestFixture::new(
        &std::fs::read("test_timeout_state.bin").expect("run can_get_rainbow_badge first to produce it"),
        Duration::from_mins(5),
        vec![PolicyStep::enter(Map::CeladonCity)],
    );
    let s = fixture.game_state();
    println!("=== STUCK: {} @ {} facing {:?} mode={:?} ===",
        s.map.map, s.map.player_position, s.map.player_direction, s.mode);
    println!("{}", s.map);
    println!("tile in front: {:?}", s.map.tile_in_front());
    println!("badges: {:?}", s.badges);
    for a in s.map.actions() {
        println!("   dest={} tile={:?} route_len={}", a.destination, a.tile, a.route.len());
    }
    // Directly mash DOWN (bypassing the policy) and see if the player walks onto (5,7) — if it moves,
    // the game tree there is actually cut; if not, it regrew (stale cut_tiles).
    let start = s.map.player_position;
    for _ in 0..200 {
        fixture.api().release_all_buttons();
        fixture.api().press_button(crate::joypad::JoypadButton::Down);
        fixture.gb.run(crate::pokemon::agent::AGENT_RESOLUTION);
        fixture.api().release_all_buttons();
        fixture.gb.run(crate::pokemon::agent::AGENT_RESOLUTION);
    }
    let s2 = fixture.game_state();
    println!("after mashing DOWN: {} @ {} (was {})", s2.map.map, s2.map.player_position, start);
}

#[test]
#[ignore]
fn probe_celadon_gym() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-celadon.bin"),
        Duration::from_mins(15),
        vec![
            PolicyStep::CutTree { map: Map::CeladonCity },
            PolicyStep::enter(Map::CeladonGym),
        ],
    );
    for _ in 0..200_000 {
        fixture.step();
        if fixture.game_state().map.map == Map::CeladonGym
            && fixture.agent.policy_steps_remaining().map_or(true, |n| n == 0) { break; }
    }
    for _ in 0..3_000 { fixture.step(); }
    let s = fixture.game_state();
    println!("=== {} @ {} facing {:?} ===", s.map.map, s.map.player_position, s.map.player_direction);
    println!("{}", s.map);
    for a in s.map.actions() {
        println!("   dest={} tile={:?}", a.destination, a.tile);
    }
}

#[test]
#[ignore]
fn inspect_party_at_route10() {
    let mut fixture = TestFixture::new(include_bytes!("data/at-route10.bin"), Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    println!("map={} money={}", s.map.map, s.money);
    for p in s.pokemon.iter() {
        println!("{} '{}' lv{} hp{}/{}  moves:", p.species, p.nickname, p.level, p.current_hp, p.stats.hp);
        for m in p.moves.iter().flatten() {
            println!("    {:?} pp {}/{}", m.name, m.pp, m.name.metadata().pp);
        }
    }
}

/// Leg 4: from Celadon City, cut the trees sealing the gym, enter, and beat Erika for the Rainbow
/// Badge. `DefeatGymLeader` persists until the badge is won (self-heals on blackout).
#[test]
fn can_get_rainbow_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-celadon.bin"), Duration::from_mins(45), PolicyStep::celadon_rainbow_steps());
    // Step until the badge is earned (DefeatGymLeader never pops on its own).
    while !fixture.game_state().badges.contains(Badge::RainbowBadge) {
        fixture.step();
    }
    let s = fixture.game_state();
    println!("badges={:?} on {} party_lv={:?}", s.badges, s.map.map,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>());
    assert!(s.badges.contains(Badge::RainbowBadge), "should earn the Rainbow Badge from Erika");
    fixture.save_state_named("src/pokemon/data/post-rainbow-badge.bin").unwrap();
}

/// Stage 4a (WIP): from post-Erika (Celadon City), reach the Rocket Hideout — heal, walk to the Game
/// Corner, flip the poster switch (new `FlipSwitch` capability + `found_rocket_hideout` event), and
/// descend to B1F. **Blocked**: the guarding Rocket sprite reads as hidden@(14,5) instead of
/// shown@(9,5), so the agent can neither Interact him nor `route_to_face` the poster at (9,4) — a
/// sprite-reading issue to fix before this leg works. The Rocket-Hideout spinner-tile floors (B2F/B3F)
/// arrow→destination tables are decoded (see the plan doc) and await this entrance.
#[test]
fn can_reach_rocket_hideout() {
    // Start from the pre-gym Celadon state to exercise the entrance mechanic (Game Corner poster
    // Rocket → switch → staircase) decoupled from the separate Celadon-gym-exit blocker. The hideout
    // needs Cut (have it) not the Rainbow Badge, so this is valid before Erika.
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-celadon.bin"),
        Duration::from_mins(20),
        PolicyStep::rocket_hideout_entrance_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::RocketHideoutB1F, "should descend into the Rocket Hideout (B1F)");
    fixture.save_state_named("src/pokemon/data/at-rocket-hideout.bin").unwrap();
}

/// Stage 4b: from inside the Rocket Hideout (B1F), cross the spinner floors B2F/B3F and beat Giovanni's
/// guards on B4F to grab the **Silph Scope**. Exercises the new spinner-tile navigation with the real
/// party. Snapshots `post-silph-scope.bin`.
#[test]
fn can_get_lift_key() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-rocket-hideout.bin"),
        Duration::from_mins(30),
        PolicyStep::lift_key_steps(),
    );
    fixture.step_until_exhausted();
    // CollectItem pops when the walk is issued; keep stepping until the key is in the bag.
    for _ in 0..20_000 {
        if fixture.game_state().bag.contains(&ItemId::LiftKey) { break; }
        fixture.step();
    }
    let s = fixture.game_state();
    println!("ended on {} @ {} bag has lift key={}", s.map.map, s.map.player_position,
        s.bag.contains(&ItemId::LiftKey));
    assert!(s.bag.contains(&ItemId::LiftKey), "should obtain the Lift Key from the Rocket Hideout B4F");
    fixture.save_state_named("src/pokemon/data/rocket-hideout-lift-key.bin").unwrap();
}

/// Stage 4c: the full Silph Scope leg — from inside the hideout (B1F), get the Lift Key, take the
/// elevator (entered from B2F, whose warp is not gated by the Rocket-5 door) to Giovanni's split B4F
/// room, beat the two Rockets to drop the door wall, beat Giovanni, and grab the **Silph Scope**.
/// Relies on the runtime `ReplaceTileBlock` door-block modelling (`MetaTileMap::apply_door_blocks`) so
/// BFS avoids the event-gated B1F/B4F door walls that the static ROM map shows as open floor.
/// Snapshots `post-silph-scope.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_silph_scope() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-rocket-hideout.bin"),
        Duration::from_mins(40),
        PolicyStep::silph_scope_steps(),
    );
    fixture.step_until_exhausted();
    for _ in 0..20_000 {
        if fixture.game_state().bag.contains(&ItemId::SilphScope) { break; }
        fixture.step();
    }
    let s = fixture.game_state();
    println!("ended on {} @ {} bag has scope={}", s.map.map, s.map.player_position,
        s.bag.contains(&ItemId::SilphScope));
    assert!(s.bag.contains(&ItemId::SilphScope), "should obtain the Silph Scope from the Rocket Hideout");
    fixture.save_state_named("src/pokemon/data/post-silph-scope.bin").unwrap();
}

/// Stage 4d: the Poké Flute leg — from the hideout (post-Silph-Scope), leave, travel to Lavender, climb
/// Pokémon Tower (Channelers + the Scope-revealed ghost Marowak), beat the 7F Rockets and rescue Mr.
/// Fuji, who hands over the **Poké Flute**. Snapshots `post-poke-flute.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_poke_flute() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-silph-scope.bin"),
        Duration::from_mins(60),
        PolicyStep::poke_flute_steps(),
    );
    fixture.step_until_exhausted();
    for _ in 0..20_000 {
        if fixture.game_state().bag.contains(&ItemId::PokeFlute) { break; }
        fixture.step();
    }
    let s = fixture.game_state();
    println!("ended on {} @ {} bag has flute={}", s.map.map, s.map.player_position,
        s.bag.contains(&ItemId::PokeFlute));
    assert!(s.bag.contains(&ItemId::PokeFlute), "should obtain the Poké Flute from Mr. Fuji");
    fixture.save_state_named("src/pokemon/data/post-poke-flute.bin").unwrap();
}

/// Stage 4e: use the Poké Flute to wake the **Route 12 Snorlax** (new field item-use capability),
/// beating it in the wild battle to clear the road south. Snapshots `post-snorlax.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_wake_snorlax() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-poke-flute.bin"),
        Duration::from_mins(30),
        PolicyStep::snorlax_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    // The Snorlax sprite is gone once beaten; the step completes on that. Confirm we're on Route 12
    // and the blocker is cleared.
    let snorlax_present = s.map.sprites.iter().any(|sp| !sp.hidden && sp.name == "Snorlax");
    println!("ended on {} @ {} snorlax_present={}", s.map.map, s.map.player_position, snorlax_present);
    assert_eq!(s.map.map, Map::Route12, "should be on Route 12 after waking the Snorlax");
    assert!(!snorlax_present, "the Route 12 Snorlax should be defeated and gone");
    fixture.save_state_named("src/pokemon/data/post-snorlax.bin").unwrap();
}

/// Stage 4f: from Route 12 (post-Snorlax), travel Route 13 → 14 → 15 → Fuchsia City and beat Koga for
/// the **Soul Badge**. Snapshots `post-soul-badge.bin`.
///
/// Two navigation fixes made this work: (1) `actions()` now emits *every* reachable connection tile per
/// adjacent map (not just the nearest), so an `EnterMap { to_position }` can pick a landing — the
/// nearest Route 13→14 crossing drops into a trainer-sealed dead-end pocket (row 6), so we cross at
/// (0,9) to land at the open Route 14 (19,8); (2) Route 15 has a gate building walling off the Fuchsia
/// connection, traversed like the Route 12 gate (east door → west exit (7,8)).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_soul_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-snorlax.bin"),
        Duration::from_mins(60),
        PolicyStep::soul_badge_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {} badges={:?}", s.map.map, s.map.player_position, s.badges);
    assert!(s.badges.contains(Badge::SoulBadge), "should win the Soul Badge from Koga");
    fixture.save_state_named("src/pokemon/data/post-soul-badge.bin").unwrap();
}

/// Stage 4g: Safari Zone run for HM03 Surf + the Gold Teeth (exercises the new Safari battle handling —
/// the agent RUNs from every encounter). Snapshots `post-safari-surf.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_surf_safari() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-soul-badge.bin"),
        Duration::from_mins(45),
        PolicyStep::safari_zone_surf_steps(),
    );
    fixture.step_until_exhausted();
    // The final Interact pops as soon as it issues the walk; keep stepping until the guru hands over Surf.
    for _ in 0..20_000 {
        if fixture.game_state().bag.contains(&ItemId::Hm03Surf) { break; }
        fixture.step();
    }
    let s = fixture.game_state();
    println!("ended on {} @ {} bag has surf={} gold_teeth={}", s.map.map, s.map.player_position,
        s.bag.contains(&ItemId::Hm03Surf), s.bag.contains(&ItemId::GoldTeeth));
    assert!(s.bag.contains(&ItemId::Hm03Surf), "should obtain HM03 Surf from the Safari Zone Secret House");
    fixture.save_state_named("src/pokemon/data/post-safari-surf.bin").unwrap();
}

/// Stage 4g (cont.): exit the Safari Zone and give the Gold Teeth to the Warden for HM04 Strength.
/// Snapshots `post-safari.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_strength_warden() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-safari-surf.bin"),
        Duration::from_mins(30),
        PolicyStep::safari_zone_strength_steps(),
    );
    fixture.step_until_exhausted();
    for _ in 0..20_000 {
        if fixture.game_state().bag.contains(&ItemId::Hm04Strength) { break; }
        fixture.step();
    }
    let s = fixture.game_state();
    println!("ended on {} @ {} strength={}", s.map.map, s.map.player_position, s.bag.contains(&ItemId::Hm04Strength));
    assert!(s.bag.contains(&ItemId::Hm04Strength), "Warden should give HM04 Strength for the Gold Teeth");
    fixture.save_state_named("src/pokemon/data/post-safari.bin").unwrap();
}

/// Stage 4h: from Fuchsia (post-safari), trek to Celadon, buy a Fresh Water from the roof vending
/// machine (new `UseVendingMachine` step), and pass the Route-7 guard into Saffron. Reverses the
/// soul-badge gates (Route 15/12 gates west→east/south→north; the Lavender→Route8 and Route-7-gate
/// crossings use `EnterMap { to_position }`). Snapshots `at-saffron.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_enter_saffron() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/post-safari.bin"),
        Duration::from_mins(60),
        PolicyStep::saffron_entry_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {} has_water={}", s.map.map, s.map.player_position, s.bag.contains(&ItemId::FreshWater));
    assert_eq!(s.map.map, Map::SaffronCity, "should enter Saffron City");
    fixture.save_state_named("src/pokemon/data/at-saffron.bin").unwrap();
}

/// Stage 4i (part 1): from Saffron, enter Silph Co, ride the elevator to 5F, thread the teleport-pad
/// maze to the Card Key pocket, and grab the Card Key. Snapshots `silph-card-key.bin`.
///
/// Two things had to work. **The elevator** (1F → step into the (20,0) door → ride to any floor)
/// needed five fixes: (1) `read_warp_events` crashed `game_state()` the moment the player entered any
/// elevator, because the elevator's exits point at the header-less UNUSED_MAP_ED placeholder;
/// (2)/(3)/(4) three hard-coded `Map::RocketHideoutElevator` checks (policy ×2, agent ×1)
/// skipped/aborted the elevator for every non-Rocket elevator; (5) the floor menu scrolls (11 floors)
/// so the cursor is driven by *absolute* index, and the pick's A-press is re-pulsed until the ride
/// starts. **The maze**: the Card Key sits in a walled 5F pocket (row 16) reachable only by *arriving*
/// on the 5F (9,15) pad and stepping down. (9,15)↔9F(17,15) are a teleport pair, so the route is
/// `enter(9F)` (walk to the reachable (9,15) pad → 9F(17,15)) then `enter(5F)` (step back onto (17,15)
/// → arrive standing on 5F(9,15), now adjacent to the pocket) — expressed directly as `enter()` steps,
/// no new maze-routing machinery needed.
#[test]
#[ignore = "slow (release, ~minutes of game time); run with --ignored"]
fn can_get_silph_card_key() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/at-saffron.bin"),
        Duration::from_mins(30),
        PolicyStep::silph_co_card_key_steps(),
    );
    fixture.step_until_exhausted();
    for _ in 0..20_000 {
        if fixture.game_state().bag.contains(&ItemId::CardKey) { break; }
        fixture.step();
    }
    let s = fixture.game_state();
    println!("ended on {} @ {} has_card_key={}", s.map.map, s.map.player_position, s.bag.contains(&ItemId::CardKey));
    assert!(s.bag.contains(&ItemId::CardKey), "should get the Card Key from Silph Co 5F");
    fixture.save_state_named("src/pokemon/data/silph-card-key.bin").unwrap();
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
                    MetaTile::Pc => 'p',
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
            Map::MtMoonB2F => 3, Map::MtMoonB1F => 2, Map::MtMoon1F => 1,
            // Rock Tunnel: plunge into the tunnel interior (deepest) before resurfacing. Route 10 is
            // both the north entrance AND the south exit, so it must rank BELOW the tunnel interior —
            // otherwise the explorer keeps resurfacing to Route 10 (whose warp landing is perpetually
            // "unobserved" due to the ~1-tile connection-position mismatch). Lavender pulls it out south.
            Map::LavenderTown => 6, Map::RockTunnelB1F => 4, Map::RockTunnel1F => 3, Map::Route10 => 1,
            _ => 0,
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

/// Offline discovery of the Rock Tunnel warp maze: drive `ExplorerPolicy` from `back-in-cerulean.bin`
/// (a pimped party blasts through the tunnel trainers) east through Route 9 → Route 10 → Rock Tunnel
/// 1F/B1F to Lavender Town, printing the discovered warp chain as ready-to-paste `EnterMap` steps.
/// Not bound by the gameplay budget. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn discover_rock_tunnel_path() {
    use crate::pokemon::symbols::pokered_symbols as ps;
    let mut fixture = TestFixture::new(include_bytes!("data/at-route10.bin"), Duration::from_mins(120), vec![]);
    fixture.pimp_pokemon();
    // Destinations the explorer is allowed to walk into (Route 9 excluded so it never backtracks west).
    fixture.agent = PokemonAgent::new(Box::new(ExplorerPolicy::new(vec![
        Map::Route10, Map::RockTunnel1F, Map::RockTunnelB1F, Map::LavenderTown,
    ])));

    let start_map = fixture.game_state().map.map;
    let start_entry = Point8 {
        x: fixture.gb.core().mmu().read_pointer(&ps::wXCoord),
        y: fixture.gb.core().mmu().read_pointer(&ps::wYCoord),
    };
    println!("start section: {start_map} @ {start_entry}");

    let mut last_node_count = 0usize;
    let mut steps_since_growth = 0u32;
    for i in 0..2_000_000u32 {
        fixture.step();
        if i % 100 != 0 { continue; }

        if let Some(path) = fixture.agent.world_graph().shortest_node_path_from(start_map, start_entry, Map::LavenderTown) {
            println!("\n=== DISCOVERED path ({} hops) ===", path.len() - 1);
            for (m, p) in path.iter() { println!("  {m} @ {p}"); }
            println!("\n=== EnterMap steps ===");
            for (m, p) in path.iter().skip(1) {
                println!("Self::enter_at(Map::{m:?}, {}, {}),", p.x, p.y);
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
        if steps_since_growth >= 80_000 {
            println!("\n=== STUCK: no new sections observed. Observed graph ({} nodes): ===", nodes.len());
            let mut ns = nodes;
            ns.sort_by_key(|((m, p), _)| (*m as u16, p.x, p.y));
            for ((m, p), edges) in &ns {
                println!("  {m} @ {p}:");
                for e in edges { println!("      -> {} @ {} ({:?})", e.to.map, e.to.location, e.kind); }
            }
            let gs = fixture.game_state();
            println!("current: {} @ {} facing {:?}", gs.map.map, gs.map.player_position, gs.map.player_direction);
            for a in gs.map.actions() {
                println!("   dest={} tile={:?} route={:?}", a.destination, a.tile, a.route);
            }
            return;
        }
    }
    panic!("did not discover a path to Lavender Town");
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



