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

// ── W0.4 — the manual-input escape hatch ─────────────────────────────────────────────────────────

/// A queued raw press must reach the game through a path the state machine has no action for at all.
/// START is the clearest case: no `OverworldAction` can express it, and the agent never presses it of
/// its own accord in the overworld, so a menu on screen afterwards can only have come from the queue.
///
/// Checked on the tick the queue drains, before the agent has resumed and started reading the menu
/// as a text box — one tick later it is already mashing A through it.
#[test]
fn manual_input_presses_a_button_the_agent_never_would() {
    use crate::joypad::JoypadButton;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60), vec![]);
    // Settle into the ordinary overworld idle first, so what follows pre-empts a running state
    // machine rather than pressing into a fresh one.
    for _ in 0..20 { fixture.step(); }
    assert_eq!(fixture.api().game_mode(), Some(GameMode::Overworld), "expected a quiet overworld");

    fixture.agent.queue_manual_input([JoypadButton::Start]);
    assert_eq!(fixture.agent.manual_input_pending(), 1);
    assert_eq!(fixture.agent.state_debug(), "idle", "queueing must clear the current state");

    for _ in 0..MANUAL_INPUT_TICKS_PER_PRESS { fixture.step(); }
    assert_eq!(fixture.agent.manual_input_pending(), 0, "the press should be fully delivered");

    // The START menu is the one thing that can take a standing overworld into `TextBox` without the
    // player touching anything, and the agent presses START nowhere in the overworld. `on_screen_text`
    // deliberately cannot corroborate it: the overworld has not loaded `vFont`, so the reader has no
    // tiles to decode and answers `None` no matter what the menu says.
    assert_eq!(fixture.api().game_mode(), Some(GameMode::TextBox),
               "START should have opened the menu");
}

/// The press/hold/release cadence, asserted tick by tick, because both halves of it are load-bearing
/// and neither is visible in the game state:
///
/// - **the hold** is two ticks because one is not enough — see `MANUAL_INPUT_HOLD_TICKS`;
/// - **the release** is what separates repeats. pokered drives menus off *newly* pressed bits, so a
///   button held straight through would deliver "A, A" as a single A.
#[test]
fn manual_input_holds_then_releases_each_press() {
    use crate::joypad::JoypadButton;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60), vec![]);
    for _ in 0..20 { fixture.step(); }

    fixture.agent.queue_manual_input([JoypadButton::A, JoypadButton::A]);
    let mut held = Vec::new();
    for _ in 0..2 * MANUAL_INPUT_TICKS_PER_PRESS {
        fixture.step();
        held.push(fixture.api().mmu().joypad().is_button_pressed(JoypadButton::A));
    }

    assert_eq!(held, vec![true, true, false, true, true, false],
               "each press gets two held ticks and one released one");
    assert_eq!(fixture.agent.manual_input_pending(), 0);
}

/// The cap exists so a confused model cannot hand the agent a hundred buttons and take the game away
/// from the state machine for seconds at a time. Anything past it is dropped, not queued.
#[test]
fn manual_input_queue_is_capped() {
    use crate::joypad::JoypadButton;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(1), vec![]);
    fixture.agent.queue_manual_input(
        std::iter::repeat(JoypadButton::B).take(MANUAL_INPUT_CAPACITY * 3));
    assert_eq!(fixture.agent.manual_input_pending(), MANUAL_INPUT_CAPACITY);

    // A second call appends into what is left rather than resetting the cap.
    fixture.agent.queue_manual_input([JoypadButton::A]);
    assert_eq!(fixture.agent.manual_input_pending(), MANUAL_INPUT_CAPACITY);
}

/// The measurement behind [`MANUAL_INPUT_HOLD_TICKS`]: for each hold length, does one START press
/// open the menu, at each of 16 successive agent-tick alignments?
///
/// A hold of 1 tick (20 ms — longer than a frame, which is why it looks like it should be enough)
/// prints `.` at five of the sixteen. A hold of 2 prints `Y` at all of them. pokered does not sample
/// the pad on every frame in the overworld, and a dropped press is the worst failure mode this
/// feature has: the LLM cannot tell one from a button the game ignored deliberately.
///
/// `cargo test --release --features diagnostics --bin gb -- probe_manual_input_hold_length --ignored --nocapture`
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "probe — run with --ignored --nocapture"]
fn probe_manual_input_hold_length() {
    use crate::joypad::JoypadButton;

    for align in 0..16usize {
        let mut row = String::new();
        for hold in 1..=4usize {
            let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60), vec![]);
            for _ in 0..20 + align { fixture.step(); }
            fixture.api().press_button(JoypadButton::Start);
            // Driven straight, without the agent, so the probe measures the game's pad sampling and
            // nothing about the queue that is built on top of it.
            for _ in 0..hold { fixture.gb.run(AGENT_RESOLUTION); }
            fixture.api().release_all_buttons();
            for _ in 0..6 { fixture.gb.run(AGENT_RESOLUTION); }
            row.push_str(if fixture.api().game_mode() == Some(GameMode::Overworld) { " ." } else { " Y" });
        }
        println!("alignment {align:>2}: holds 1..4 ={row}");
    }
}

// ── W0.3 / W0.5b — the two policy seams ──────────────────────────────────────────────────────────

/// What [`RecordingPolicy`] saw, shared with the test because the agent owns the policy.
#[derive(Default)]
struct Recording {
    /// `AgentEvent` is `Debug`-only, so the debug rendering is the record. It is enough: the tests
    /// below ask which *kinds* of event arrived, not what was inside them.
    events: Vec<String>,
    /// One entry per `service_tools` call: the map it was told about, and how many maps the world
    /// graph it was handed knew. Enough to prove the triple arrives intact and is the agent's own.
    tool_polls: Vec<(Map, usize)>,
}

/// A `DeterministicPolicy` that also records what the agent asks of it. Every decision is delegated
/// unchanged, so the run is identical to the same steps without it — the recording is pure
/// observation.
struct RecordingPolicy {
    inner: DeterministicPolicy,
    log: std::rc::Rc<std::cell::RefCell<Recording>>,
}

impl RecordingPolicy {
    fn new(steps: Vec<PolicyStep>) -> (Box<Self>, std::rc::Rc<std::cell::RefCell<Recording>>) {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Recording::default()));
        (Box::new(Self { inner: DeterministicPolicy::new(42, steps), log: log.clone() }), log)
    }
}

impl crate::pokemon::policy::Policy for RecordingPolicy {
    fn on_event(&mut self, event: &AgentEvent) {
        self.log.borrow_mut().events.push(format!("{event:?}"));
    }

    fn service_tools(&mut self, state: &GameState, api: &mut PokemonApi<'_>,
                     graph: &crate::pokemon::world_graph::WorldGraph) {
        // Answer the poll the way `LlmPolicy` will: straight out of the observation facade, against
        // the state already in hand. If this runs, W5's tool dispatch is a match arm over W0.5.
        use crate::pokemon::observe;
        assert_eq!(observe::map_view(state).map, format!("{}", state.map.map),
                   "the facade must describe the state it was given");
        assert!(observe::trainer(state, api).badge_count <= 8);
        assert_eq!(observe::party(state).len(), state.pokemon.len());
        self.log.borrow_mut().tool_polls.push((state.map.map, graph.map_count()));
    }

    fn pick_overworld_action(&mut self, state: &GameState,
                             graph: &crate::pokemon::world_graph::WorldGraph)
        -> Option<crate::pokemon::actions::OverworldAction>
    {
        self.inner.pick_overworld_action(state, graph)
    }
    fn pick_battle_action(&mut self, state: &GameState)
        -> Option<crate::pokemon::battle::BattleAction>
    {
        self.inner.pick_battle_action(state)
    }
    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        self.inner.pick_nickname(species)
    }
    fn pick_mart_purchase(&mut self, state: &GameState) -> Option<Option<crate::pokemon::bag::BagItem>> {
        self.inner.pick_mart_purchase(state)
    }
    fn pick_move_to_forget(&mut self, moves: &[crate::pokemon::move_name::PokemonMove],
                           new_move: crate::pokemon::move_name::PokemonMoveName)
        -> Option<Option<usize>>
    {
        self.inner.pick_move_to_forget(moves, new_move)
    }
    fn pick_field_move(&mut self, state: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
        self.inner.pick_field_move(state)
    }
    fn is_exhausted(&self) -> bool { self.inner.is_exhausted() }
    fn steps_remaining(&self) -> Option<usize> { self.inner.steps_remaining() }
    fn current_step_is_long_running(&self) -> bool { self.inner.current_step_is_long_running() }
}

/// **W0.3** — every event the agent emits reaches the policy, *including* the ones collected into
/// `update`'s local `new_events` and drained at the end of the tick. That was the class the plan
/// warned might be missed; `OverworldActionCompleted` is only ever emitted that way, so seeing one
/// here is the proof that the drain goes through `event()`.
///
/// **W0.5b** — `service_tools` is called at the overworld poll, and the triple it receives is real:
/// a state that agrees with the map the agent is standing on, and the agent's own world graph rather
/// than an empty one.
#[test]
fn policy_sees_every_event_and_gets_a_tool_poll() {
    // The same short indoor walk as `test_debouncing`: it ends in a warp, so the run emits a text
    // box, an action completion and a map change for almost no game time.
    let (policy, log) = RecordingPolicy::new(vec![PolicyStep::goto(Map::PalletTown)]);
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/oaks-lab-just-got-squirtle.bin"),
        Duration::from_secs(200),
        policy,
    );
    fixture.step_until_exhausted();

    let log = log.borrow();
    assert!(!log.tool_polls.is_empty(), "service_tools was never called");
    assert!(log.tool_polls.iter().any(|(map, _)| *map == Map::OaksLab),
            "the poll should carry the map the agent is on; saw {:?}", log.tool_polls);
    assert!(log.tool_polls.iter().any(|(_, maps_known)| *maps_known > 0),
            "the world graph handed to service_tools should be the agent's, not an empty one");

    assert!(!log.events.is_empty(), "on_event was never called");
    assert!(log.events.iter().any(|e| e.starts_with("OverworldActionCompleted")),
            "events pushed into `new_events` and drained at the end of the tick must reach the \
             policy too; saw {:?}", log.events);
}

// ── W0.5 — the observation facade ────────────────────────────────────────────────────────────────

/// Every view against a known snapshot. These are the shapes the LLM sees, so the assertions are
/// about the things a wrong one would quietly get away with: a count that disagrees with the list it
/// counts, an HP over its maximum, a grid whose rows do not match the width it declares.
#[test]
fn observation_views_describe_the_snapshot() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();
    let api = fixture.api();

    let trainer = observe::trainer(&state, &api);
    assert_eq!(trainer.badge_count as usize, trainer.badges.len(), "badge count vs badge list");
    assert!(trainer.badge_count <= 8);
    assert_eq!(trainer.playtime.len(), 8, "playtime should read HH:MM:SS, got {}", trainer.playtime);
    assert!(trainer.pokedex_owned <= trainer.pokedex_seen,
            "you cannot own more species than you have seen ({} > {})",
            trainer.pokedex_owned, trainer.pokedex_seen);

    let party = observe::party(&state);
    assert_eq!(party.len(), state.pokemon.len());
    for (slot, mon) in party.iter().enumerate() {
        assert_eq!(mon.slot, slot, "slots must be the party index, not the enumeration of a filter");
        assert!(mon.hp <= mon.max_hp, "{}: {}/{}", mon.species, mon.hp, mon.max_hp);
        assert_eq!(mon.fainted, mon.hp == 0);
        assert!(!mon.moves.is_empty(), "{} knows no moves", mon.species);
        assert!(mon.moves.iter().all(|m| m.pp <= m.max_pp));
        assert!((1..=2).contains(&mon.types.len()), "{:?}", mon.types);
    }

    let bag = observe::bag(&state, &api);
    assert_eq!(bag.slots_used, bag.items.len());
    assert_eq!(bag.slots_total, 20);
    assert_eq!(bag.money, trainer.money);

    let status = observe::status(&state, &api);
    assert_eq!(status.badges.len(), 8, "the status reports every badge, earned or not");
    assert_eq!(
        status.badges.iter().filter(|badge| badge.earned).count() as u32,
        trainer.badge_count,
    );
    assert_eq!(
        status.badges.iter().filter(|badge| badge.earned).map(|badge| badge.name.clone()).collect::<Vec<_>>(),
        trainer.badges,
        "the two views must agree on which badges, not merely how many",
    );
    assert_eq!(status.playtime, trainer.playtime);
    assert_eq!(status.party_hp.len(), party.len());
    assert!(!status.in_battle, "the Pallet Town snapshot is not in a battle");
    assert!(observe::battle(&state).is_none(), "…so there is no battle to describe");

    // The overworld loads no dialogue font, so there is nothing on screen to decode. Reporting that
    // as `None` rather than an error is the contract.
    assert_eq!(observe::screen_text(&api), None);
}

/// The map grid is the single densest thing the model reads, and three ways of getting it wrong are
/// invisible from the outside: rows that do not match the declared width, a legend that has fallen
/// behind `impl Display for MetaTileMap`, and an order that reshuffles between two identical reads.
#[test]
fn map_view_is_well_formed_stable_and_fully_documented() {
    use crate::pokemon::observe;

    const REDS_HOUSE_1F_STATE: &[u8] = include_bytes!("../data/reds-house-1f-state.bin");
    let legend: std::collections::HashSet<char> =
        observe::MAP_LEGEND.iter().map(|(c, _)| *c).collect();

    // An outdoor town, a route with grass and ledges, and an indoor map — between them they exercise
    // most of the tile alphabet.
    for (name, snapshot) in [("Pallet Town", PALLET_TOWN_STATE),
                             ("Route 1", ROUTE1_STATE),
                             ("Red's house", REDS_HOUSE_1F_STATE)] {
        let mut fixture = TestFixture::new(snapshot, Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let view = observe::map_view(&state);

        assert_eq!(view.grid.len(), view.height, "{name}: row count vs declared height");
        for (y, row) in view.grid.iter().enumerate() {
            assert_eq!(row.chars().count(), view.width, "{name}: row {y} is {row:?}");
        }
        assert_eq!(view.grid.iter().flat_map(|r| r.chars()).filter(|c| *c == 'P').count(), 1,
                   "{name}: exactly one player on the map");
        assert_eq!(view.grid[view.position.y as usize].chars().nth(view.position.x as usize), Some('P'),
                   "{name}: the reported position must be where the P is");

        // The legend travels with the grid, so a new symbol in `Display for MetaTileMap` that nobody
        // documented shows up here rather than as an unexplained character in a context window.
        for c in view.grid.iter().flat_map(|r| r.chars()) {
            assert!(legend.contains(&c), "{name}: grid uses '{c}', which MAP_LEGEND does not explain");
        }
        assert_eq!(view.legend.len(), observe::MAP_LEGEND.len());

        // `warp_targets` and the action list come off a `HashSet`. Two reads of an unchanged map must
        // still be equal, or the model sees churn that is not there.
        assert_eq!(view, observe::map_view(&state), "{name}: two reads of one state disagree");
        for warp in &view.warps {
            // 'P' is painted over whatever the player is standing on, and standing on a warp is
            // exactly what a door looks like the tick after you walk through it — so the `warps`
            // list, not the grid, is what tells you the warp is there.
            let drawn = view.grid[warp.at.y as usize].chars().nth(warp.at.x as usize);
            assert!(drawn == Some('W') || warp.at == view.position,
                    "{name}: warp at {:?} is drawn as {drawn:?}", warp.at);
        }
    }
}

/// The badge strip the web UI draws, against a snapshot that has actually earned some.
///
/// `observation_views_describe_the_snapshot` checks the shape on a fixture with **no** badges, which
/// cannot tell a working mapping from one that reports `false` eight times. This one uses the
/// end-of-game fixture, where the answer is known: index `i` is bit `i` of `wObtainedBadges`, and the
/// sprite at that index in `/api/badges.png` is the badge the name says it is.
#[test]
fn the_badge_strip_reports_which_badges_not_only_how_many() {
    use crate::pokemon::badge::Badge;
    use crate::pokemon::observe;
    const POST_EARTH_BADGE: &[u8] = include_bytes!("../data/post-earth-badge.bin");

    let mut fixture = TestFixture::new(POST_EARTH_BADGE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();
    let badges = observe::status(&state, &fixture.api()).badges;

    assert_eq!(badges.len(), 8);
    assert_eq!(
        badges.iter().map(|badge| badge.name.as_str()).collect::<Vec<_>>(),
        Badge::ORDER.iter().map(|badge| format!("{badge}")).collect::<Vec<_>>(),
        "the strip must be in bit order — the sprite sheet is indexed by it",
    );
    for badge in &badges {
        assert_eq!(
            badge.earned,
            state.badges.contains(Badge::from_name(&badge.name).expect("a real flag name")),
            "{} disagrees with the badge flags", badge.name,
        );
    }
    let earned = badges.iter().filter(|badge| badge.earned).count();
    assert_eq!(earned, 8, "the post-Earth-Badge fixture should hold every badge, not {earned}");
}

/// The battle view against a snapshot that is in one, including the option list a decider chooses
/// from — the thing that must never come back empty on a turn the game is waiting for.
#[test]
fn battle_view_describes_a_live_battle() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(BATTLE_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();

    let battle = observe::battle(&state).expect("the battle snapshot should be in a battle");
    assert!(battle.player.hp <= battle.player.max_hp);
    assert!(battle.enemy.hp <= battle.enemy.max_hp);
    assert!(battle.player.level > 0 && battle.enemy.level > 0);
    assert!(!battle.options.is_empty(), "a battle with no legal action would deadlock the agent");
    assert!(!battle.player.moves.is_empty(), "the active Pokémon knows no moves");
    assert_eq!(observe::status(&state, &fixture.api()).in_battle, true);
    assert_eq!(battle.active_party_slot as usize, {
        let slot = battle.active_party_slot as usize;
        assert!(slot < state.pokemon.len(), "active slot {slot} is outside the party");
        slot
    });
}

/// The world graph is built as the player walks, so its guarantee is negative: an absent map means
/// unvisited, never unreachable. The view has to hold that line and stay ordered.
#[test]
fn world_graph_view_reports_only_visited_maps() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60),
                                       vec![PolicyStep::goto(Map::Route1)]);
    let empty = observe::world_graph(fixture.agent.world_graph());
    assert_eq!(empty.map_count, 0, "nothing has been walked yet");
    assert!(empty.nodes.is_empty());

    fixture.step_until_exhausted();

    let graph = observe::world_graph(fixture.agent.world_graph());
    assert!(graph.map_count > 0, "walking to Route 1 should have observed at least one map");
    assert_eq!(graph.nodes.len(), graph.map_count);
    assert_eq!(graph.edge_count, graph.nodes.iter().map(|n| n.edges.len()).sum::<usize>());
    assert_eq!(graph, observe::world_graph(fixture.agent.world_graph()),
               "two reads of one graph must agree");

    let names: Vec<&str> = graph.nodes.iter().map(|n| n.map.as_str()).collect();
    assert!(names.iter().any(|n| *n == format!("{}", Map::PalletTown)),
            "Pallet Town was walked; saw {names:?}");
    assert_eq!(observe::known_maps(fixture.agent.world_graph()).len(), graph.map_count);
}

/// Under `--features web` the views serialise. Worth its own test because `cfg_attr` failing to
/// apply is silent — the code still compiles, it just stops being able to leave the process, and the
/// first sign would be W5's tool layer not building.
#[test]
#[cfg(feature = "web")]
fn observation_views_serialise_to_json() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();
    let json = serde_json::to_value(observe::map_view(&state)).expect("map view should serialise");

    assert_eq!(json["map"], format!("{}", state.map.map));
    // `Point` is a struct so the coordinates are named rather than a pair a model has to guess at.
    assert_eq!(json["position"]["x"], state.map.player_position.x);
    assert_eq!(json["grid"].as_array().expect("grid is an array").len(), state.map.height);

    for value in [serde_json::to_value(observe::party(&state)).unwrap(),
                  serde_json::to_value(observe::status(&state, &fixture.api())).unwrap(),
                  serde_json::to_value(observe::bag(&state, &fixture.api())).unwrap()] {
        assert!(!value.is_null());
    }
}
