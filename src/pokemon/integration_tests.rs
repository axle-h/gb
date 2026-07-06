
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

/// The full end-to-end playthrough — the single source of truth for how far the agent can play. This
/// emulates the entire run frame-by-frame and takes ~2 hours of wall-clock, so it is **opt-in**: it
/// only runs with the `slow-tests` feature (`cargo test --release --features slow-tests can_start_game`).
/// The per-leg focused tests above (each seeded from a saved fixture) cover the same ground quickly and
/// run on a normal `cargo test`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "full ~2h playthrough; run with --features slow-tests")]
fn can_start_game() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/start-of-game-state.bin"),
        Duration::from_mins(190),
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
    // NB: do NOT resave post-cascade.bin here — the run now continues through the Rainbow Badge, so
    // the final state is no longer post-Cascade. The committed post-cascade.bin fixture (a viable
    // 2-mon, Tackle-keeping party) is regenerated by `can_start_game` only implicitly via the
    // per-leg tests that snapshot their own fixtures.

    assert!(state.badges.contains(Badge::BoulderBadge), "should have the Boulder Badge");
    assert!(state.badges.contains(Badge::CascadeBadge), "should have the Cascade Badge");
    assert!(state.badges.contains(Badge::ThunderBadge), "should have the Thunder Badge");
    assert!(state.badges.contains(Badge::RainbowBadge), "should have the Rainbow Badge");

    // A viable party keeps at least one damaging move on the starter (the move-learning heuristic
    // must not have silently dropped Tackle) plus a second Pokémon for the Nugget Bridge leg.
    assert!(state.pokemon.len() >= 2, "party should have ≥2 Pokémon (starter + caught mon)");
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

#[test]
#[ignore]
fn probe_party() {
    let mut fixture = TestFixture::new(include_bytes!("data/post-soul-badge.bin"), Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    println!("=== party ({} members), badges={:?} ===", s.pokemon.len(), s.badges);
    for (i, p) in s.pokemon.iter().enumerate() {
        let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}({})", m.name, m.pp)).collect();
        println!("  {i}: {:?} lv{} hp {}/{} types {:?} moves {:?}",
            p.species, p.level, p.current_hp, p.stats.hp, p.types, moves);
    }
}

/// Diagnostic: from `silph-card-key.bin` (in the 5F Card Key pocket), step out via the (9,15) pad to
/// 9F and dump which warps/sprites are walkable-reachable there — to plan the route up to 7F (Lapras)
/// and 11F (Giovanni) now that the Card Key doors are open.
#[test]
#[ignore]
fn probe_silph_post_cardkey() {
    let mut fixture = TestFixture::new(include_bytes!("data/silph-card-key.bin"), Duration::from_mins(40),
        vec![PolicyStep::enter(Map::SilphCo9F), PolicyStep::enter(Map::SilphCo3F)]);
    let mut last = Point8 { x: 255, y: 255 };
    let mut same = 0;
    for i in 0..400_000 {
        fixture.step();
        if fixture.agent.policy_exhausted() { println!(">> exhausted at step {i}"); break; }
        if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
            let p = s.map.player_position;
            if p != last { last = p; same = 0; if i % 200 == 0 { println!("  step {i}: {} @ {p} mode={:?}", s.map.map, s.mode); } }
            else { same += 1; if same == 30_000 { println!(">> STUCK at {} @ {p} mode={:?} step {i}", s.map.map, s.mode); } }
        }
    }
    if let Ok(s) = { PokemonApi::with_cache(&mut fixture.gb, &mut fixture.map_cache).game_state() } {
        println!("ended on {} @ {}", s.map.map, s.map.player_position);
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



