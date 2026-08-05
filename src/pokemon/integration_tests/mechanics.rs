//! The fast tier: individual agent/state-reading mechanics, each from a snapshot that is already
//! sitting on the thing being tested. Most emulate seconds or nothing at all, so these run on a plain
//! `cargo test --release`.

use super::*;

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
    pub const STATE: &[u8] = include_bytes!("../data/oaks-lab-just-got-squirtle.bin");

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
        const REDS_HOUSE_1F_STATE: &[u8] = include_bytes!("../data/reds-house-1f-state.bin");
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
    const POKEMART_STATE: &[u8] = include_bytes!("../data/viridian-city-pokemart-during-script.bin");

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
    const STATE: &[u8] = include_bytes!("../data/viridian-city-pokemart-shopping.bin");
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
    const STATE: &[u8] = include_bytes!("../data/viridian-city-pokemart-shopping.bin");
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
    const BUSH_STATE: &[u8] = include_bytes!("../data/viridian-city-north-of-bush.bin");

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
        include_bytes!("../data/start-of-game-state.bin"),
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
        include_bytes!("../data/viridian-forest.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::CatchPokemon { species: PokemonSpecies::Weedle, on_map: Map::ViridianForest, ball: None },
            PolicyStep::goto(Map::ViridianForest),
        ]
    );
    fixture.step_until_exhausted();
    let state = fixture.game_state();
    let weedle = &state.pokemon[2];
    assert_eq!(weedle.species, PokemonSpecies::Weedle);
    assert_ne!(weedle.nickname.to_default_string(), "AAAAAAAAAA");
}

/// The Victory Road 1F Strength puzzle exposes exactly one switch tile and no holes. A pure state
/// read from a fixture standing on the floor — no emulation, so it stays in the fast tier while the
/// puzzle itself is solved by `endgame::can_solve_victory_road_1f`.
#[test]
fn strength_switches_are_exposed() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr1f-strength.bin"),
        Duration::from_mins(1),
        vec![],
    );
    let s = fixture.game_state();
    assert_eq!(s.map.map, Map::VictoryRoad1F);
    assert_eq!(s.map.strength_switches, vec![Point8 { x: 17, y: 13 }], "VR1F switch should be exposed");
    assert!(s.map.holes.is_empty(), "VR1F has no holes");
}

/// `UseRareCandy` on a party slot **other than 0**.
///
/// Pinned because it was once reported as broken — *"`UseRareCandy` only works on slot 0; asked for
/// slot 5 it spins and burns the candy on the wrong mon"* — and that report was **wrong**. What had
/// actually happened was a mis-read log: the leg that filed it caught a **wild lv21 Voltorb** instead
/// of the lv40 static one it was walking to, so the candy landed on the right Pokémon and simply took
/// it to lv22 rather than over its lv30 evolution. The menu chain took 447 ticks, which is normal.
///
/// So this test exists to stop that costing anyone else an afternoon. It drives the real chain —
/// START → ITEM → scroll the bag to a **deep** row → USE → the party menu → slot 5 — against a party
/// of six, and asserts the level went up on the mon that was asked for and on no other. ~1 s.
#[test]
fn rare_candy_works_on_a_late_party_slot() {
    const SLOT: usize = 5;
    let mut fixture = TestFixture::new(
        include_bytes!("../data/postgame-safari.bin"),
        Duration::from_mins(5),
        vec![PolicyStep::UseRareCandy { slot: SLOT as u8 }],
    );

    let before: Vec<(PokemonSpecies, u8)> =
        fixture.game_state().pokemon.iter().map(|p| (p.species, p.level)).collect();
    assert_eq!(before.len(), 6, "the point of the test is a slot the cursor has to travel to");
    assert!(fixture.api().bag_item_position(ItemId::RareCandy).is_some_and(|i| i > 8),
        "and a bag row the list has to scroll to");

    fixture.step_until_exhausted();

    let after: Vec<(PokemonSpecies, u8)> =
        fixture.game_state().pokemon.iter().map(|p| (p.species, p.level)).collect();
    println!("{before:?}\n{after:?}");
    assert_eq!(after[SLOT].1, before[SLOT].1 + 1, "slot {SLOT} should have gained exactly one level");
    for slot in (0..before.len()).filter(|s| *s != SLOT) {
        assert_eq!(after[slot], before[slot], "the candy must not touch slot {slot}");
    }
    assert!(fixture.game_state().bag.iter().all(|i| i.id != ItemId::RareCandy), "candy consumed");
}

/// `PokemonApiTrait::item_price` decodes the ROM's `ItemPrices` table, which the mart driver uses to
/// size a purchase to the wallet. The table is `table_width 3` BCD **indexed from item id 1**
/// (MASTER_BALL), so an off-by-one reads the neighbouring item's price and is otherwise invisible —
/// it would just buy a slightly wrong number of things. Values from `data/items/prices.asm`.
#[test]
fn item_prices_match_the_rom_table() {
    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(1), vec![]);
    let api = fixture.api();

    // First and last of the ball block, to catch an index slipped either way.
    assert_eq!(api.item_price(ItemId::UltraBall), Some(1200));
    assert_eq!(api.item_price(ItemId::GreatBall), Some(600));
    assert_eq!(api.item_price(ItemId::PokeBall), Some(200));
    // The potions the mainline's restocks actually buy.
    assert_eq!(api.item_price(ItemId::HyperPotion), Some(1500));
    assert_eq!(api.item_price(ItemId::SuperPotion), Some(700));
    assert_eq!(api.item_price(ItemId::WaterStone), Some(2100));
    // Priced at 0 in the table = no mart sells it; the driver must order these as asked rather than
    // divide by zero or trim to nothing.
    assert_eq!(api.item_price(ItemId::MasterBall), None);
    assert_eq!(api.item_price(ItemId::TownMap), None);
    // Past the end of the table entirely — HM/TM ids start at $C4 and are priced elsewhere. Without
    // the length bound this decoded three bytes of the *next* ROM table as a price.
    assert_eq!(api.item_price(ItemId::Hm01Cut), None);
    assert_eq!(api.item_price(ItemId::Tm14Blizzard), None);
}
