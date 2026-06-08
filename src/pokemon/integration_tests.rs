
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::pokemon::policy::{DeterministicPolicy, PolicyStep};
use crate::pokemon::*;
use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason, PokemonAgent, AGENT_RESOLUTION};
use crate::pokemon::battle::BattleType;
use crate::pokemon::world_graph::WorldGraph;
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

#[test]
fn can_navigate_mt_moon() {
    let mut fixture = TestFixture::new(
        include_bytes!("data/mt-moon.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::goto(Map::CeruleanCity),
        ]
    );

    fixture.pimp_pokemon();
    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::CeruleanCity, "agent should have navigated to Cerulean City");
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

        let world_graph = WorldGraph::build(gb.core().mmu());
        let policy = DeterministicPolicy::new(42, policy_steps, world_graph);

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
}



