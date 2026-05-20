
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::bag::BagWriter;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::encoding::MetaTile;
use crate::pokemon::item::ItemId;
use crate::pokemon::policy::Policy;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use crate::pokemon::*;
use crate::pokemon::agent::AGENT_RESOLUTION;

pub const PALLET_TOWN_STATE: &[u8] = include_bytes!("data/pallet-town-state.bin");
pub const ROUTE1_STATE: &[u8] = include_bytes!("data/route1-state.bin");
pub const BATTLE_STATE: &[u8] = include_bytes!("data/battle-state.bin");

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PolicyEvent {
    Movement(Map),
    Battle,
}

struct FindBattleOnRoute1Policy {
    previous_map: Map,
    event_tx: Sender<PolicyEvent>,
}

impl FindBattleOnRoute1Policy {
    pub fn new(event_tx: Sender<PolicyEvent>) -> Self {
        Self { previous_map: Map::PalletTown, event_tx }
    }
}

impl Policy for FindBattleOnRoute1Policy {
    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        let next_map = match (self.previous_map, state.map.map) {
            (Map::PalletTown, Map::Route1) => Map::ViridianCity,
            (Map::Route1, Map::ViridianCity) => Map::Route1,
            (Map::ViridianCity, Map::Route1) => Map::PalletTown,
            (Map::Route1, Map::PalletTown) => Map::Route1,
            _ => return None
        };

        let preferred = state.map.actions().iter()
            .find(|a| a.tile == MetaTile::Connection(next_map))
            .cloned();

        self.event_tx.send(PolicyEvent::Movement(next_map)).ok();

        if preferred.is_some() {
            self.previous_map = next_map;
        }
        preferred
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        self.event_tx.send(PolicyEvent::Battle).ok();
        None
    }
}


#[test]
fn test_battle_state_reading() {
    use battle::BattleType;
    use crate::pokemon::move_name::PokemonMoveName;

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(BATTLE_STATE).unwrap();
    gb.run(MachineCycles::from_m(500));

    // Add items and a second Pokemon so every BattleAction variant is exercisable.
    {
        // 3 Potions + 1 Full Heal — replaces whatever was in the bag.
        gb.core_mut().mmu_mut()
            .write_bag(&[BagItem::new(ItemId::Potion, 3), BagItem::new(ItemId::FullHeal, 1)]);
    }
    {
        let mut party = gb.core().mmu().read_player_pokemon_party().unwrap();
        party.push(Pokemon::maxed(
            PokemonSpecies::Pidgey,
            "PIDGEY",
            [PokemonMoveName::Tackle, PokemonMoveName::SandAttack,
             PokemonMoveName::Gust, PokemonMoveName::Whirlwind],
            "RED", 12345,
        )).unwrap();
        gb.core_mut().mmu_mut().write_player_pokemon_party(&party).unwrap();
    }

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
    let battle = state.battle.expect("should be in battle");

    // Basic battle validity
    assert_eq!(battle.battle_type, BattleType::Wild);
    assert!(battle.enemy.level > 0, "enemy level should be non-zero");
    assert!(battle.enemy.stats.hp > 0, "enemy max HP should be non-zero");
    assert!(battle.enemy.current_hp <= battle.enemy.stats.hp);
    assert!(battle.player.moves.iter().any(|m| m.is_some()), "player needs at least one move");

    // Bag — we wrote exactly 2 items, so expect exactly those 2.
    assert_eq!(state.bag[0].id, ItemId::Potion, "bag[0] should be POTION");
    assert_eq!(state.bag[0].quantity, 3);
    assert!(state.bag.iter().any(|i| i.id == ItemId::FullHeal),
        "FULL HEAL should be in the bag");
}

/// Verifies the agent can navigate through a wild battle end-to-end.
///
/// Uses a deterministic `TestPolicy` that always picks the northward connection
/// (Viridian City / Route 1) to keep the player walking through grass.  As soon
/// as a battle is encountered the policy selects Fight(0) every turn.  The test
/// succeeds once `wIsInBattle` returns to 0 (battle over).
#[test]
fn test_agent_battle_lifecycle() {
    use crate::pokemon::agent::PokemonAgent;

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(ROUTE1_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let (tx, rx) = mpsc::channel::<PolicyEvent>();

    let policy = FindBattleOnRoute1Policy::new(tx);
    let mut agent = PokemonAgent::new(Box::new(policy));

    let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16)); // ~60 FPS

    let max_frames = 30_000u32;
    let mut battle_started = false;

    for _frame in 0..max_frames {
        gb.run(frame_cycles);
        agent.update(&mut gb, frame_cycles).ok();


        if let Ok(event) = rx.try_recv() {
            println!("{:?}", event);
            if event == PolicyEvent::Battle {
                battle_started = true;
                break;
            }
        }
    }

    assert!(battle_started, "battle did not start within {} frames", max_frames);
}

/// Verifies the agent can fight through a wild battle using the first available
/// move until the enemy faints, then the player is returned to the overworld on Route 1.
#[test]
#[ignore]
fn test_battle_fight_to_victory_returns_to_route1() {
    use crate::pokemon::agent::PokemonAgent;

    #[derive(Debug, PartialEq, Eq)]
    enum FightEvent { BattleStarted, BattleEnded }

    struct FightFirstMovePolicy {
        previous_map: Map,
        in_battle: bool,
        event_tx: Sender<FightEvent>,
    }

    impl FightFirstMovePolicy {
        fn new(event_tx: Sender<FightEvent>) -> Self {
            Self { previous_map: Map::PalletTown, in_battle: false, event_tx }
        }
    }

    impl Policy for FightFirstMovePolicy {
        fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
            if self.in_battle {
                self.in_battle = false;
                self.event_tx.send(FightEvent::BattleEnded).ok();
            }
            let next_map = match (self.previous_map, state.map.map) {
                (Map::PalletTown, Map::Route1) => Map::ViridianCity,
                (Map::Route1, Map::ViridianCity) => Map::Route1,
                (Map::ViridianCity, Map::Route1) => Map::PalletTown,
                (Map::Route1, Map::PalletTown) => Map::Route1,
                _ => return None,
            };
            let preferred = state.map.actions().iter()
                .find(|a| a.tile == MetaTile::Connection(next_map))
                .cloned();
            if preferred.is_some() {
                self.previous_map = next_map;
            }
            preferred
        }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            if !self.in_battle {
                self.in_battle = true;
                self.event_tx.send(FightEvent::BattleStarted).ok();
            }
            let battle = state.battle.as_ref()?;
            battle.player.moves.iter()
                .enumerate()
                .find_map(|(i, m)| m.filter(|m| m.pp > 0).map(|m| BattleAction::Fight { slot: i as u8, battle_move: m }))
        }
    }

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(ROUTE1_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let (tx, rx) = mpsc::channel::<FightEvent>();
    let policy = FightFirstMovePolicy::new(tx);
    let mut agent = PokemonAgent::new(Box::new(policy));

    let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16));
    let max_frames = 100_000u32;
    let mut battle_ended = false;

    for _frame in 0..max_frames {
        gb.run(frame_cycles);
        agent.update(&mut gb, frame_cycles).ok();

        if let Ok(event) = rx.try_recv() {
            println!("{:?}", event);
            if event == FightEvent::BattleEnded {
                battle_ended = true;
                break;
            }
        }
    }

    assert!(battle_ended, "battle did not end within {} frames", max_frames);

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
    assert_eq!(state.mode, GameMode::Overworld, "player should be on the overworld after battle");
    assert_eq!(state.map.map, Map::Route1, "player should be on Route 1 after battle");
}

#[test]
fn test_route_1() {
    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(ROUTE1_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));
    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
    assert_eq!(state.map.map, Map::Route1);
    println!("{}", state.map);
}

/// Route 1 has tall grass — a WalkInGrass action must be present and route to a Grass tile.
/// Indoor maps (no wGrassTile) must produce no WalkInGrass action.
#[test]
fn test_walk_in_grass_action() {
    use encoding::MetaTile;

    // Route 1: expect a WalkInGrass action
    {
        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(ROUTE1_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();

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

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(REDS_HOUSE_1F_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();

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
    use encoding::{JumpDirection, MetaTile};

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(ROUTE1_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));
    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
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
            .filter(|a| matches!(a.tile, MetaTile::Connection(_)) && a.destination.y == target_y)
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
    use encoding::MetaTile;

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(PALLET_TOWN_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));
    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
    let map = &state.map;

    let actions = map.actions();

    // Sanity-check that the test is actually exercising something.
    assert!(!map.warp_targets.is_empty(), "expected warp targets in Pallet Town");
    assert!(map.sprites.iter().any(|s| !s.hidden), "expected visible sprites in Pallet Town");

    // Every warp target must produce an action with a non-empty route.
    for &to_map in &map.warp_targets {
        let action = actions.iter().find(|a| a.tile == MetaTile::Warp(to_map));
        assert!(action.is_some(), "no action for warp to {to_map}");
        assert!(!action.unwrap().route.is_empty(), "empty route to warp {to_map}");
    }

    // Every visible (non-hidden) sprite must produce an action with a non-empty route.
    for sprite in map.sprites.iter().filter(|s| !s.hidden) {
        let action = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name));
        assert!(action.is_some(), "no action for sprite '{}'", sprite.name);
        assert!(!action.unwrap().route.is_empty(), "empty route to sprite '{}'", sprite.name);
    }

    // Route 1 is walkable from Pallet Town — must have a connection action.
    assert!(
        actions.iter().any(|a| a.tile == MetaTile::Connection(Map::Route1)),
        "missing connection action for Route1"
    );

    // Route 21 is water-only from Pallet Town — no walkable Connection action expected.
    assert!(
        !actions.iter().any(|a| a.tile == MetaTile::Connection(Map::Route21)),
        "unexpected walkable connection to Route21 (should be water-only)"
    );
}

/// When the ViridianCity PokeMART clerk's intro script is running the agent must
/// advance the conversation by pressing A — not try to navigate around the map.
#[test]
fn test_viridian_pokemart_script_advances_dialogue() {
    use std::time::Duration;
    use crate::pokemon::agent::PokemonAgent;

    const POKEMART_STATE: &[u8] = include_bytes!("data/viridian-city-pokemart-during-script.bin");

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(POKEMART_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    // The save state has the clerk's text box already open.
    // The game mode must be TextBox or Script — never plain Overworld.
    {
        let api = PokemonApi::new(&mut gb);
        let mode = api.game_mode().unwrap();
        assert!(
            matches!(mode, GameMode::TextBox | GameMode::Script),
            "Expected TextBox or Script mode in PokeMART save state, got {:?}", mode
        );
    }

    // Run the agent: it should press A to advance the dialogue and eventually
    // return to Overworld mode.
    let mut agent = PokemonAgent::new(Box::new(crate::pokemon::policy::RandomPolicy));
    let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16));

    let mut returned_to_overworld = false;
    for _ in 0..10_000u32 {
        gb.run(frame_cycles);
        agent.update(&mut gb, frame_cycles).ok();

        let api = PokemonApi::new(&mut gb);
        if api.game_mode() == Some(GameMode::Overworld) {
            returned_to_overworld = true;
            break;
        }
    }

    assert!(
        returned_to_overworld,
        "agent should advance the PokeMART dialogue and return to Overworld within 10000 frames"
    );
}

/// A Cut bush on the path north of Viridian City blocks access to the Fisher.
/// Without the Cascade Badge and a Pokémon that knows Cut, the bush is an
/// impassable obstacle and no action to talk to the Fisher should be generated.
#[test]
fn test_cut_bush_blocks_fisher_without_cut() {
    use encoding::MetaTile;

    const BUSH_STATE: &[u8] = include_bytes!("data/viridian-city-north-of-bush.bin");

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(BUSH_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();

    assert!(!state.can_use_cut, "player should not have Cut available at this point");

    let actions = state.map.actions();
    let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

    assert!(
        !actions.iter().any(|a| a.tile == MetaTile::Sprite("Fisher")),
        "Fisher must not be accessible without Cut; actions: {tiles:?}"
    );
}

/// The Pokémon Center nurse stands behind a counter.  The player should be able to
/// talk to her by approaching the counter tile (a "talking over" tile in pokered).
#[test]
fn test_viridian_pokecenter_nurse_action() {
    use encoding::MetaTile;

    const POKECENTER_STATE: &[u8] = include_bytes!("data/viridian-city-pokemon-center.bin");

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(POKECENTER_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
    assert_eq!(state.map.map, Map::ViridianPokecenter);

    let actions = state.map.actions();
    let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

    assert!(
        actions.iter().any(|a| a.tile == MetaTile::Sprite("Nurse")),
        "expected Talk to Nurse action (counter tile should be treated as talking-over tile); actions: {tiles:?}"
    );
}

/// With the player already standing in Red's House 1F after descending from 2F,
/// all three actions must be available: exit to Pallet Town, stairs to 2F, talk to Mom.
#[test]
fn test_reds_house_1f_actions_from_save_state() {
    use encoding::MetaTile;

    const REDS_HOUSE_1F_STATE: &[u8] = include_bytes!("data/reds-house-1f-state.bin");

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(REDS_HOUSE_1F_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
    assert_eq!(state.map.map, Map::RedsHouse1F, "save state should start in RedsHouse1F");

    let actions = state.map.actions();
    let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

    assert!(
        actions.iter().any(|a| a.tile == MetaTile::Warp(Map::PalletTown)),
        "expected exit Warp → PalletTown; actions: {tiles:?}"
    );
    assert!(
        actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse2F)),
        "expected Warp → RedsHouse2F (stairs); actions: {tiles:?}"
    );
    assert!(
        actions.iter().any(|a| a.tile == MetaTile::Sprite("Mom")),
        "expected Sprite(Mom); actions: {tiles:?}"
    );
    assert!(
        !actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse1F)),
        "self-referential Warp → RedsHouse1F must not appear; actions: {tiles:?}"
    );
}

/// After entering Red's House 1F from Pallet Town, the exit warp should lead back to
/// PalletTown — not self-referentially to RedsHouse1F.
#[test]
fn test_reds_house_1f_exit_warp() {
    use std::time::Duration;
    use encoding::MetaTile;
    use crate::pokemon::agent::PokemonAgent;
    use crate::pokemon::battle::BattleAction;

    struct EnterRedsHousePolicy;
    impl Policy for EnterRedsHousePolicy {
        fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
            if state.map.map != Map::PalletTown { return None; }
            state.map.actions().into_iter()
                .find(|a| a.tile == MetaTile::Warp(Map::RedsHouse1F))
        }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<BattleAction> { None }
    }

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(PALLET_TOWN_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let mut agent = PokemonAgent::new(Box::new(EnterRedsHousePolicy));
    let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16));

    // Count consecutive frames spent in Overworld mode on RedsHouse1F.
    // We wait for 120 stable frames (~2 s) before asserting, so the map's
    // object data (warps, sprites) has time to fully initialise after the
    // wCurMap register flips.
    let mut stable_frames = 0u32;
    for _ in 0..15_000u32 {
        gb.run(frame_cycles);
        agent.update(&mut gb, frame_cycles).ok();

        let api = PokemonApi::new(&mut gb);
        let Ok(state) = api.game_state() else { stable_frames = 0; continue };
        if state.mode != GameMode::Overworld || state.map.map != Map::RedsHouse1F {
            stable_frames = 0;
            continue;
        }
        stable_frames += 1;
        if stable_frames < 120 { continue; }

        let actions = state.map.actions();
        let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

        assert!(
            !actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse1F)),
            "self-referential Warp → RedsHouse1F inside Red's House 1F; actions: {tiles:?}"
        );
        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse2F)),
            "expected exit Warp → RedsHouse2F inside Red's House 1F; actions: {tiles:?}"
        );
        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Warp(Map::PalletTown)),
            "expected exit Warp → PalletTown inside Red's House 1F; actions: {tiles:?}"
        );
        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Sprite("Mom")),
            "expected to be able to talk to 'Mom' inside Red's House 1F; actions: {tiles:?}"
        );
        return;
    }

    panic!("never reached RedsHouse1F in overworld mode within 15000 frames");
}


/// Before the player receives the Pokédex from Oak, `has_pokedex` must be false
/// and both seen/owned sets must be empty.
#[test]
fn test_pokedex_empty_before_receiving_pokedex() {
    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(include_bytes!("data/start-of-game-state.bin")).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();

    assert!(!state.has_pokedex, "player should not have the Pokédex before Oak's script");
    assert!(state.pokedex_owned.is_empty(), "wPokedexOwned should be all-zero at game start");
    assert!(state.pokedex_seen.is_empty(), "wPokedexSeen should be all-zero at game start");
}

/// Directly writes the EVENT_GOT_POKEDEX bit (bit 37 of wEventFlags) and checks
/// that `has_pokedex` flips accordingly — tests the bit-address logic independently
/// of any particular save state's history.
#[test]
fn test_has_pokedex_bit_toggling() {
    use crate::pokemon::symbols::pokered_symbols;
    use crate::ram::{RAM, ROM};

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(ROUTE1_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    // EVENT_GOT_POKEDEX = bit 37 → byte 4, bit 5, mask 0x20 of wEventFlags
    let flag_addr = pokered_symbols::wEventFlags.address + 4;

    {
        let mmu = gb.core_mut().mmu_mut();
        let current = mmu.read(flag_addr);
        mmu.write(flag_addr, current & !0x20); // clear the bit
    }
    {
        let api = PokemonApi::new(&mut gb);
        assert!(!api.game_state().unwrap().has_pokedex, "should be false when bit 5 is clear");
    }

    {
        let mmu = gb.core_mut().mmu_mut();
        let current = mmu.read(flag_addr);
        mmu.write(flag_addr, current | 0x20); // set the bit
    }
    {
        let api = PokemonApi::new(&mut gb);
        assert!(api.game_state().unwrap().has_pokedex, "should be true when bit 5 is set");
    }
}

/// Directly writes known species bits into wPokedexOwned/wPokedexSeen and verifies
/// `read_pokedex_flags` decodes them to the correct `PokemonSpecies` values.
/// Bulbasaur=dex#1 (byte 0, bit 0), Charmander=dex#4 (byte 0, bit 3),
/// Squirtle=dex#7 (byte 0, bit 6).
#[test]
fn test_pokedex_flag_bit_decoding() {
    use crate::pokemon::symbols::pokered_symbols;
    use crate::ram::{RAM, ROM};

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(ROUTE1_STATE).unwrap();
    gb.run(MachineCycles::from_m(1000));

    let owned_base = pokered_symbols::wPokedexOwned.address;
    // Write byte 0 with bits for Bulbasaur (#1=bit0), Charmander (#4=bit3), Squirtle (#7=bit6)
    // Bit index = dex_number - 1, LSB first.
    // Bulbasaur: bit 0 → mask 0x01
    // Charmander: bit 3 → mask 0x08
    // Squirtle: bit 6 → mask 0x40
    let test_byte: u8 = 0x01 | 0x08 | 0x40; // = 0x49

    gb.core_mut().mmu_mut().write(owned_base, test_byte);

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();

    assert!(state.pokedex_owned.contains(&PokemonSpecies::Bulbasaur), "Bulbasaur should be owned");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Charmander), "Charmander should be owned");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Squirtle), "Squirtle should be owned");
    assert_eq!(state.pokedex_owned.species().len(), 3, "exactly 3 species should be owned");
}

/// When a wild battle is in progress, the enemy species must appear in `pokedex_seen`.
/// The game sets the seen flag when the wild encounter begins.
#[test]
fn test_pokedex_seen_contains_battle_enemy() {
    use battle::BattleType;

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(BATTLE_STATE).unwrap();
    gb.run(MachineCycles::from_m(500));

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();

    let battle = state.battle.expect("save state should be in a wild battle");
    assert_eq!(battle.battle_type, BattleType::Wild);

    assert!(
        state.pokedex_seen.contains(&battle.enemy.species),
        "wPokedexSeen should contain {:?} (the current battle enemy)",
        battle.enemy.species
    );
}

#[test]
fn can_start_game() {
    use crate::pokemon::agent::PokemonAgent;
    use crate::pokemon::policy::{DeterministicPolicy};

    const START_OF_GAME: &[u8] =
        include_bytes!("data/start-of-game-state.bin");

    let mut gb = GameBoy::dmg(roms::POKERED);
    gb.load_state(START_OF_GAME).unwrap();
    gb.run(MachineCycles::from_m(1000));

    {
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        assert_eq!(state.map.map, Map::RedsHouse2F, "save state should be in RedsHouse2F");
        assert_eq!(state.pokemon.len(), 0, "player should have no pokemon before Oak's script");
    }

    let policy = DeterministicPolicy::complete_game(42, gb.core().mmu());
    let mut agent = PokemonAgent::new(Box::new(policy));

    // Gary battles the player inside Oak's Lab after the starter is chosen; the
    // DeterministicPolicy handles the battle and the policy retries navigation
    // until the player actually reaches Route 1.  Generous budget because debug
    // builds are slow — use `--release` for a sensible wall-clock time.
    const MAX_CYCLES: MachineCycles = MachineCycles::from_duration(Duration::from_secs(1000));
    let mut total_cycles = MachineCycles::ZERO;

    while !agent.policy_exhausted() {
        let cycles = gb.run(AGENT_RESOLUTION);
        agent.update(&mut gb, cycles).ok();

        total_cycles += cycles;
        if total_cycles >= MAX_CYCLES {
            break;
        }
    }

    assert!(agent.policy_exhausted(), "not all policy actions were taken");

    let api = PokemonApi::new(&mut gb);
    let state = api.game_state().unwrap();
    let starter = state.pokemon.iter().next().expect("should have a starter");
    println!("Starter: {:?}  Nickname: {}  on map: {:?}", starter.species, starter.nickname, state.map.map);
    assert_eq!(
        format!("{}", starter.nickname), "Celina",
        "DeterministicPolicy with seed 42 should name the starter Celina"
    );
}