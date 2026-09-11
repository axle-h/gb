//! The fast tier: individual agent/state-reading mechanics, each from a snapshot that is already
//! sitting on the thing being tested.

use super::*;

#[test]
fn test_ledge_jump_does_not_abort_overworld_movement() {
    let mut fixture = TestFixture::new(
        ROUTE1_STATE,
        Duration::from_secs(200),
        vec![
            PolicyStep::GrindUntilLevel { target_level: 100, on_map: Map::Route1, target: PartyRef::Slot(0) },
        ]
    );

    let mut battle_in_grass = false;

    loop {
        fixture.step();
        for event in fixture.agent.drain_events() {
            match &event {
                AgentEvent::OverworldActionAborted { destination, reason, .. }
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

/// An arrow tile is the walk being carried out, not a script that ended it.
#[test]
fn an_arrow_tile_carries_the_walk_rather_than_ending_it() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-rocket-hideout.bin"),
        Duration::from_secs(240),
        vec![
            PolicyStep::enter(Map::RocketHideoutB2F),
            PolicyStep::enter(Map::RocketHideoutElevator),
        ],
    );

    let mut script_aborts: Vec<String> = Vec::new();
    while !fixture.agent.policy_exhausted() {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::OverworldActionAborted {
                reason: OverworldActionAbortedReason::Script, at, ..
            } = &event {
                script_aborts.push(format!("{at:?}"));
            }
        }
    }

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::RocketHideoutElevator,
               "should have crossed B2F to the lift, which is only reachable over the arrows");
    assert!(script_aborts.is_empty(),
            "a spin tile is the walk, not a script that ended it — aborted at {script_aborts:?}");
}

/// A lift's doors lead back to the floor it was entered from until its panel picks another.
#[test]
fn a_lifts_doors_lead_to_the_floor_it_was_entered_from() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-silph-scope.bin"),
        Duration::from_secs(240),
        vec![PolicyStep::enter(Map::RocketHideoutElevator)],
    );
    fixture.step_until_exhausted();
    fixture.run_until(|state| state.map.map == Map::RocketHideoutElevator);
    // A second inside, for the lift's own script to have run.
    for _ in 0..50 {
        fixture.step();
    }
    let state = fixture.game_state();

    let doors: Vec<Map> = state.map.actions().iter().filter_map(|action| match action.tile {
        MetaTile::Warp { to_map, .. } => Some(to_map),
        _ => None,
    }).collect();
    assert!(!doors.is_empty(), "the lift offered no door at all");
    assert!(doors.iter().all(|&to| to == Map::RocketHideoutB4F),
            "the lift was entered from B4F and its doors say {doors:?}");
}

/// A text box in answer to an A press is the interaction landing, not a failure.
#[test]
fn talking_to_a_sprite_is_a_success_not_an_abort() {
    let mut fixture = TestFixture::new(
        PALLET_TOWN_STATE,
        Duration::from_secs(400),
        vec![
            PolicyStep::goto(Map::RedsHouse1F),
            PolicyStep::Interact(MapSprite::REDSHOUSE1F_MOM),
            PolicyStep::goto(Map::RedsHouse2F),
            PolicyStep::UsePc { map: Map::RedsHouse2F },
        ],
    );

    // Not `step_until_exhausted`: `Interact` pops once the walk is issued, before the talk starts.
    let mut landed: Vec<AgentEvent> = Vec::new();
    let mut interrupted: Vec<AgentEvent> = Vec::new();
    while landed.len() < 2 {
        fixture.step();
        for event in fixture.agent.drain_events() {
            match event {
                AgentEvent::OverworldInteractionCompleted { .. } => landed.push(event),
                AgentEvent::OverworldActionAborted {
                    destination: MetaTile::Sprite(_) | MetaTile::Pc,
                    reason: OverworldActionAbortedReason::Textbox,
                    ..
                } => interrupted.push(event),
                _ => {}
            }
        }
    }

    assert!(interrupted.is_empty(), "an answered A press is not an interruption; saw {interrupted:?}");
    assert_eq!(
        landed.iter().map(|event| format!("{event}")).collect::<Vec<_>>(),
        ["✓ talked to Mom", "✓ used the PC"],
    );
}

/// A script that interrupts a walk is still an abort, which is why the check is on the faced tile.
#[test]
fn a_script_that_interrupts_a_walk_is_still_an_abort() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/oaks-lab-just-got-squirtle.bin"),
        Duration::from_secs(120),
        vec![PolicyStep::Interact(MapSprite::OAKSLAB_SCIENTIST1)],
    );

    let outcome = 'walk: loop {
        fixture.step();
        for event in fixture.agent.drain_events() {
            match event {
                AgentEvent::OverworldInteractionCompleted { .. }
                | AgentEvent::OverworldActionAborted { .. } => break 'walk event,
                _ => {}
            }
        }
    };

    assert_eq!(
        format!("{outcome}"),
        "✗ gave up on Scientist 1 at (5, 6): the game stopped you to say something",
        "the aide was never reached, so this is the abort it always was; got {outcome:?}",
    );
}

/// The person you are talking to is very often not the tile you are facing.
#[test]
fn talking_over_a_counter_is_a_success_not_an_abort() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/back-in-cerulean.bin"),
        Duration::from_secs(200),
        vec![
            PolicyStep::goto(Map::CeruleanPokecenter),
            PolicyStep::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
        ],
    );

    // `Interact` pops when the walk is issued, so run until the outcome is emitted.
    let outcome = 'walk: loop {
        fixture.step();
        for event in fixture.agent.drain_events() {
            match &event {
                AgentEvent::OverworldInteractionCompleted { target: MetaTile::Sprite("Nurse") }
                | AgentEvent::OverworldActionAborted { destination: MetaTile::Sprite("Nurse"), .. } =>
                    break 'walk event,
                _ => {}
            }
        }
    };

    assert_eq!(format!("{outcome}"), "✓ talked to Nurse", "got {outcome:?}");
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

    // Southward: one Down press north of the ledge jumps it, landing two tiles south.
    let north_to_south = shortest_to_connection(north_of_ledge, south_row)
        .expect("Pallet Town reachable from north of ledge via jump");
    let south_to_south = shortest_to_connection(south_of_ledge, south_row)
        .expect("Pallet Town reachable from south of ledge");

    assert!(
        north_to_south <= south_to_south + 1,
        "jumping south over ledge ({north_to_south} steps) should cost at most \
         1 more than starting just south of it ({south_to_south} steps)"
    );

    // Northward: ledges block, so the route detours.
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

    assert!(!map.warp_targets.is_empty(), "expected warp targets in Pallet Town");
    assert!(map.sprites.iter().any(|s| !s.hidden), "expected visible sprites in Pallet Town");

    for &(warp_map, warp_pos) in &map.warp_targets {
        let action = actions.iter()
            .find(|a| matches!(a.tile, MetaTile::Warp { to_map, to_position } if to_map == warp_map && to_position == warp_pos));
        assert!(action.is_some(), "no action for warp to {warp_map} at {warp_pos:?}");
        assert!(!action.unwrap().route.is_empty(), "empty route to warp {warp_map} at {warp_pos:?}");
    }

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

/// The Viridian Mart clerk's intro script is advanced with A rather than navigated around.
#[test]
fn test_viridian_pokemart_script_advances_dialogue() {
    const POKEMART_STATE: &[u8] = include_bytes!("../data/viridian-city-pokemart-during-script.bin");

    let mut fixture = TestFixture::new(
        POKEMART_STATE,
        Duration::from_secs(60),
        vec![PolicyStep::goto(Map::ViridianCity)],
    );

    // The save state has the clerk's text box already open.
    {
        let mode = fixture.game_state().mode;
        assert!(
            matches!(mode, GameMode::TextBox | GameMode::Script),
            "Expected TextBox or Script mode in PokeMART save state, got {:?}", mode
        );
    }

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

/// The mart sequence: an explicit `Interact(Clerk)` opens the shop, then `BuyFromMart`.
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

/// Before Oak's Pokédex, `has_pokedex` is false and seen and owned are empty.
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

/// Toggling the EVENT_GOT_POKEDEX bit in RAM flips `has_pokedex`, whatever the save's history.
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

/// Species bits written into wPokedexOwned/wPokedexSeen decode to the right `PokemonSpecies`.
#[test]
fn test_pokedex_flag_bit_decoding() {
    let mut fixture = TestFixture::new(ROUTE1_STATE, Duration::from_secs(10), vec![]);

    let owned_base = pokered_symbols::wPokedexOwned.address;
    // Bit index = dex number - 1, LSB first: Bulbasaur bit 0, Charmander bit 3, Squirtle bit 6.
    let test_byte: u8 = 0x01 | 0x08 | 0x40; // = 0x49

    fixture.gb.core_mut().mmu_mut().write(owned_base, test_byte);

    let state = fixture.game_state();

    assert!(state.pokedex_owned.contains(&PokemonSpecies::Bulbasaur), "Bulbasaur should be owned");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Charmander), "Charmander should be owned");
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Squirtle), "Squirtle should be owned");
    assert_eq!(state.pokedex_owned.species().len(), 3, "exactly 3 species should be owned");
}

/// When a wild battle is in progress, the enemy species must appear in `pokedex_seen`.
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

/// The Victory Road 1F Strength puzzle exposes exactly one switch tile and no holes.
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

/// `UseRareCandy` works on a party slot other than 0.
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

/// `item_price` decodes the ROM's `ItemPrices` table, which the mart driver sizes a purchase by.
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
    // Price 0 means no mart sells it; the driver orders these as asked.
    assert_eq!(api.item_price(ItemId::MasterBall), None);
    assert_eq!(api.item_price(ItemId::TownMap), None);
    // Past the end of the table entirely — HM/TM ids start at $C4 and are priced elsewhere.
    assert_eq!(api.item_price(ItemId::Hm01Cut), None);
    assert_eq!(api.item_price(ItemId::Tm14Blizzard), None);
}

/// A queued raw press reaches the game by a path the state machine has no action for.
#[test]
fn manual_input_presses_a_button_the_agent_never_would() {
    use gb::joypad::JoypadButton;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60), vec![]);
    // Settle into the overworld idle first, so the press pre-empts a running state machine.
    for _ in 0..20 { fixture.step(); }
    assert_eq!(fixture.api().game_mode(), Some(GameMode::Overworld), "expected a quiet overworld");

    fixture.agent.queue_manual_input([JoypadButton::Start]);
    assert_eq!(fixture.agent.manual_input_pending(), 1);
    assert_eq!(fixture.agent.state_debug(), "idle", "queueing must clear the current state");

    for _ in 0..MANUAL_INPUT_TICKS_PER_PRESS { fixture.step(); }
    assert_eq!(fixture.agent.manual_input_pending(), 0, "the press should be fully delivered");

    // Only the START menu takes a standing overworld into `TextBox`, and the agent never opens it.
    assert_eq!(fixture.api().game_mode(), Some(GameMode::TextBox),
               "START should have opened the menu");
}

/// Each manual press is held for two ticks and released for one; neither shows in the game state.
#[test]
fn manual_input_holds_then_releases_each_press() {
    use gb::joypad::JoypadButton;

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

/// The manual input queue is capped, so a model cannot take the game from the agent for long.
#[test]
fn manual_input_queue_is_capped() {
    use gb::joypad::JoypadButton;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(1), vec![]);
    fixture.agent.queue_manual_input(
        std::iter::repeat(JoypadButton::B).take(MANUAL_INPUT_CAPACITY * 3));
    assert_eq!(fixture.agent.manual_input_pending(), MANUAL_INPUT_CAPACITY);

    // A second call appends into what is left rather than resetting the cap.
    fixture.agent.queue_manual_input([JoypadButton::A]);
    assert_eq!(fixture.agent.manual_input_pending(), MANUAL_INPUT_CAPACITY);
}

/// What [`RecordingPolicy`] saw, shared with the test because the agent owns the policy.
#[derive(Default)]
struct Recording {
    /// `AgentEvent` is `Debug`-only, so the debug rendering is the record.
    events: Vec<String>,
    tool_polls: Vec<(Map, usize)>,
}

/// A `DeterministicPolicy` that also records what the agent asks of it.
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
    fn name(&self) -> &'static str { "recording" }

    fn on_event(&mut self, event: &AgentEvent) {
        self.log.borrow_mut().events.push(format!("{event:?}"));
    }

    fn service_tools(&mut self, state: &GameState, api: &mut PokemonApi<'_>,
                     graph: &crate::pokemon::world_graph::WorldGraph) {
        // Answer the poll as `LlmPolicy` does: from the observation facade, on the state in hand.
        use crate::pokemon::observe;
        assert_eq!(observe::map_view(state).map, format!("{}", state.map.map),
                   "the facade must describe the state it was given");
        assert_eq!(observe::bag(state, api).slots_used, state.bag.len());
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
    fn pick_move_to_forget(&mut self, slot: usize, moves: &[crate::pokemon::move_name::PokemonMove],
                           new_move: crate::pokemon::move_name::PokemonMoveName)
        -> Option<Option<usize>>
    {
        self.inner.pick_move_to_forget(slot, moves, new_move)
    }
    fn pick_field_move(&mut self, state: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
        self.inner.pick_field_move(state)
    }
    fn is_exhausted(&self) -> bool { self.inner.is_exhausted() }
    fn steps_remaining(&self) -> Option<usize> { self.inner.steps_remaining() }
    fn current_step_is_long_running(&self) -> bool { self.inner.current_step_is_long_running() }
}

#[test]
fn policy_sees_every_event_and_gets_a_tool_poll() {
    // The indoor walk from `test_debouncing`: a text box, a completion and a map change, quickly.
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

#[test]
fn a_policy_can_ask_for_a_raw_press_and_the_agent_delivers_it() {
    use gb::joypad::JoypadButton;

    /// Decides at the overworld poll and hands the press over a tick later, as `LlmPolicy` does.
    #[derive(Default)]
    struct AsksForStart {
        decided: bool,
    }
    impl crate::pokemon::policy::Policy for AsksForStart {
        fn name(&self) -> &'static str { "asks-for-start" }

        fn take_manual_input(&mut self) -> Vec<JoypadButton> {
            match std::mem::take(&mut self.decided) {
                true => vec![JoypadButton::Start],
                false => Vec::new(),
            }
        }
        fn pick_overworld_action(&mut self, _: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction>
        {
            self.decided = true;
            None
        }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<crate::pokemon::battle::BattleAction> {
            None
        }
    }

    let mut fixture =
        TestFixture::with_policy(PALLET_TOWN_STATE, Duration::from_secs(60), Box::new(AsksForStart::default()));

    // Step until the agent has pulled the press, which nothing in this test queues by hand.
    let mut collected = false;
    for _ in 0..200 {
        fixture.step();
        if fixture.agent.manual_input_pending() > 0 {
            collected = true;
            break;
        }
    }
    assert!(collected, "the agent never collected the press the policy was holding");

    for _ in 0..MANUAL_INPUT_TICKS_PER_PRESS { fixture.step(); }
    assert_eq!(fixture.agent.manual_input_pending(), 0, "the press should be fully delivered");
    assert_eq!(fixture.api().game_mode(), Some(GameMode::TextBox),
               "START should have opened the menu — the agent never presses it by itself");
}

/// A policy that plays ordinarily and writes down everything the watchdog does to it.
struct WatchdogSpy {
    timeout: Option<Duration>,
    log: std::rc::Rc<std::cell::RefCell<WatchdogLog>>,
}

#[derive(Default)]
struct WatchdogLog {
    /// Every `pick_unstick`, as the policy saw it.
    jams: Vec<(String, Duration)>,
    /// Real decision points; the watchdog does not count `service_tools` as one.
    polls: usize,
    /// Handed to the agent on the tick after the first nudge is asked for.
    nudge: Option<Vec<JoypadButton>>,
    /// Whether a nudge has already been armed, so the spy asks for exactly one.
    nudged: bool,
}

impl WatchdogSpy {
    fn new(timeout: Option<Duration>) -> (Box<Self>, std::rc::Rc<std::cell::RefCell<WatchdogLog>>) {
        let log = std::rc::Rc::new(std::cell::RefCell::new(WatchdogLog::default()));
        (Box::new(Self { timeout, log: log.clone() }), log)
    }
}

impl crate::pokemon::policy::Policy for WatchdogSpy {
    fn name(&self) -> &'static str { "watchdog-spy" }

    fn stuck_timeout(&self) -> Option<Duration> {
        self.timeout
    }

    fn pick_unstick(&mut self, _state: &GameState, jam: crate::pokemon::policy::Jam<'_>) {
        let mut log = self.log.borrow_mut();
        log.jams.push((jam.agent_state.to_string(), jam.stuck_for));
        // Answers on the third ask, as an `LlmPolicy` turn is polled on every tick while it runs.
        if !log.nudged && log.jams.len() >= 3 {
            log.nudged = true;
            log.nudge = Some(vec![JoypadButton::A]);
        }
    }

    fn service_tools(&mut self, _: &GameState, _: &mut PokemonApi<'_>,
                     _: &crate::pokemon::world_graph::WorldGraph) {
        self.log.borrow_mut().polls += 1;
    }

    fn take_manual_input(&mut self) -> Vec<JoypadButton> {
        self.log.borrow_mut().nudge.take().unwrap_or_default()
    }

    fn pick_overworld_action(&mut self, state: &GameState,
                             _: &crate::pokemon::world_graph::WorldGraph)
        -> Option<crate::pokemon::actions::OverworldAction>
    {
        state.map.actions().into_iter().next()
    }

    fn pick_battle_action(&mut self, _: &GameState) -> Option<crate::pokemon::battle::BattleAction> {
        None
    }
}

#[test]
fn the_watchdog_wakes_a_policy_the_agent_has_stopped_asking() {
    let (policy, log) = WatchdogSpy::new(Some(Duration::from_secs(1)));
    let mut fixture = TestFixture::with_policy(PALLET_TOWN_STATE, Duration::from_secs(120), policy);

    let mut fired_after = None;
    for _ in 0..2_000 {
        fixture.step();
        if fired_after.is_none() && !log.borrow().jams.is_empty() {
            fired_after = Some(fixture.agent.since_last_policy_poll());
        }
        // Stop once the nudge has been collected — that is the last link in the chain.
        if fired_after.is_some() && fixture.agent.manual_input_pending() > 0 {
            break;
        }
    }

    let jams = log.borrow().jams.clone();
    let (state, stuck_for) = jams.first().cloned().expect("the watchdog never fired");
    assert!(stuck_for >= Duration::from_secs(1),
            "the watchdog fired early, after only {stuck_for:?}");
    assert!(!state.is_empty() && state != "idle",
            "the jam has to name what the agent thought it was doing, got `{state}`");
    assert!(fired_after.is_some(), "the watchdog never fired");

    // Asked on every tick of the jam, so a turn's tool batch is serviced and a `wait` counts down.
    assert!(jams.len() > 1, "the watchdog asked once and gave up; a turn needs polling to complete");

    // The answer travels the escape hatch; `queue_manual_input` resets the state machine to `Idle`.
    assert!(fixture.agent.manual_input_pending() > 0, "the nudge never reached the agent");

    // The event reaches the model, the UI, the transcript and stdout.
    let reported = fixture.agent.drain_events().into_iter().any(|event| {
        matches!(event, AgentEvent::WatchdogFired { ref agent_state, .. } if !agent_state.is_empty())
    });
    assert!(reported, "a firing must be reported, not quietly recovered from");

    // Once the press is delivered the agent asks again, so the clock is back to zero.
    let polls_before = log.borrow().polls;
    let mut lowest = Duration::MAX;
    for _ in 0..200 {
        fixture.step();
        lowest = lowest.min(fixture.agent.since_last_policy_poll());
    }
    assert!(log.borrow().polls > polls_before, "the agent never went back to asking for decisions");
    assert!(lowest < Duration::from_millis(100),
            "the clock must be reset by a real decision point, not by the watchdog itself; the \
             closest it came to zero was {lowest:?}");
}

/// Ordinary play never gets close to the stuck timeout.
#[test]
fn ordinary_play_stays_far_inside_the_stuck_timeout() {
    let (policy, log) = WatchdogSpy::new(
        Some(Duration::from_secs(crate::llm::config::DEFAULT_STUCK_TIMEOUT_SECS)));
    let mut fixture = TestFixture::with_policy(PALLET_TOWN_STATE, Duration::from_secs(120), policy);

    let mut longest = Duration::ZERO;
    for _ in 0..3_000 {
        fixture.step();
        longest = longest.max(fixture.agent.since_last_policy_poll());
    }

    println!("[watchdog] longest stretch without a decision point: {longest:?} of game time");
    assert!(log.borrow().jams.is_empty(),
            "the watchdog fired during ordinary play, {longest:?} without a poll");
    assert!(longest < Duration::from_secs(30),
            "an agent playing normally went {longest:?} without asking anything — either the \
             default timeout is too tight or something is genuinely wedged");
}

/// Every view against a known snapshot.
#[test]
fn observation_views_describe_the_snapshot() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();
    let api = fixture.api();

    // The two figures that come from `PokemonApi` rather than `GameState`.
    assert_eq!(observe::playtime(&api).len(), 8, "playtime should read HH:MM:SS");
    assert_eq!(observe::playtime_seconds(&api), {
        let clock = observe::playtime(&api);
        let parts: Vec<u32> = clock.split(':').map(|p| p.parse().expect("digits")).collect();
        parts[0] * 3600 + parts[1] * 60 + parts[2]
    }, "the sortable clock and the printed one must be the same instant");

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
    assert_eq!(bag.money, state.money);

    let status = observe::status(&state, &api);
    assert_eq!(status.badges.len(), 8, "the status reports every badge, earned or not");
    assert_eq!(
        status.badges.iter().filter(|badge| badge.earned).count() as u32,
        state.badges.bits().count_ones(),
    );
    assert_eq!(
        status.badges.iter().filter(|badge| badge.earned).map(|badge| badge.name.clone()).collect::<Vec<_>>(),
        state.badges.iter_names().map(|(name, _)| name.to_string()).collect::<Vec<_>>(),
        "the heartbeat and the state must agree on which badges, not merely how many",
    );
    assert_eq!(status.playtime, observe::playtime(&api));
    assert_eq!(status.party.len(), party.len());
    // The client turns `dex` into `/api/pokemon/{dex}/front.png`, so a zero is a broken image.
    for (slot, mon) in status.party.iter().zip(party.iter()) {
        // The heartbeat carries the stored name (`CHARMANDER`); the LLM view `None` for an
        // un-renamed one.
        assert!(
            slot.nickname.eq_ignore_ascii_case(&mon.nickname.clone().unwrap_or_else(|| mon.species.clone())),
            "{} vs {:?}/{}", slot.nickname, mon.nickname, mon.species,
        );
        assert!((1..=151).contains(&slot.dex), "{} has dex number {}", slot.nickname, slot.dex);
        assert_eq!(slot.level, mon.level);
        assert_eq!((slot.hp, slot.max_hp), (mon.hp, mon.max_hp));
    }
    assert!(!status.in_battle, "the Pallet Town snapshot is not in a battle");
    assert!(observe::battle(&state).is_none(), "…so there is no battle to describe");

    // The overworld loads no dialogue font, so there is nothing on screen to decode.
    assert_eq!(observe::screen_text(&api), None);
}

/// What `read_map` says about the map, and the picture that goes with it.
#[test]
fn map_view_is_well_formed_stable_and_fully_documented() {
    use crate::pokemon::observe;

    const REDS_HOUSE_1F_STATE: &[u8] = include_bytes!("../data/reds-house-1f-state.bin");
    let legend: std::collections::HashSet<char> =
        observe::MAP_LEGEND.iter().map(|(c, _)| *c).collect();

    // A town, a route with grass and ledges, and an indoor map cover most of the tile alphabet.
    for (name, snapshot) in [("Pallet Town", PALLET_TOWN_STATE),
                             ("Route 1", ROUTE1_STATE),
                             ("Red's house", REDS_HOUSE_1F_STATE)] {
        let mut fixture = TestFixture::new(snapshot, Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let view = observe::map_view(&state);

        assert!(view.position.x < view.width as u8 && view.position.y < view.height as u8,
                "{name}: the player is at {:?} on a {}x{} map", view.position, view.width, view.height);
        for warp in &view.warps {
            assert!(warp.at.x < view.width as u8 && warp.at.y < view.height as u8,
                    "{name}: warp at {:?} is off a {}x{} map", warp.at, view.width, view.height);
        }

        // `warp_targets` and the action list come off a `HashSet`.
        assert_eq!(view, observe::map_view(&state), "{name}: two reads of one state disagree");

        // `Display for MetaTileMap` is what the agent log, the probes and the renderer's fallback
        // print.
        let drawn = format!("{}", state.map);
        let grid: Vec<&str> = drawn.trim_end_matches('\n').lines().collect();
        assert_eq!(grid.len(), view.height, "{name}: row count vs declared height");
        for (y, row) in grid.iter().enumerate() {
            assert_eq!(row.chars().count(), view.width, "{name}: row {y} is {row:?}");
        }
        assert_eq!(grid.iter().flat_map(|r| r.chars()).filter(|c| *c == 'P').count(), 1,
                   "{name}: exactly one player on the map");
        assert_eq!(grid[view.position.y as usize].chars().nth(view.position.x as usize), Some('P'),
                   "{name}: the reported position must be where the P is");
        for c in grid.iter().flat_map(|r| r.chars()) {
            assert!(legend.contains(&c), "{name}: grid uses '{c}', which MAP_LEGEND does not explain");
        }

        // The picture the model is sent draws without complaint.
        let canvas = crate::llm::map_image::render(&state.map).expect("a fixture is a real map");
        assert_eq!(canvas.width() as usize,
                   crate::llm::map_image::RULER_LEFT + view.width * crate::llm::map_image::CELL_PX,
                   "{name}: the picture and the JSON disagree about the width");
    }
}

/// The JSON's `people` and the action menu agree on who can be talked to.
#[test]
fn map_view_lists_only_the_people_the_menu_offers() {
    use crate::pokemon::observe;
    use crate::pokemon::tile::MetaTile;
    let mut someone_out_of_reach = false;
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data")).expect("the fixture directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|e| e != "bin") { continue; }
        let snapshot = std::fs::read(&path).expect("a readable fixture");
        let mut fixture = TestFixture::new(&snapshot, Duration::from_secs(10), vec![]);
        let Ok(state) = fixture.try_game_state() else { continue };
        let view = observe::map_view(&state);
        let offered: std::collections::BTreeSet<String> = state.map.actions().iter()
            .filter_map(|a| match a.tile { MetaTile::Sprite(name) => Some(MetaTile::Sprite(name).id_kind().into_owned()), _ => None })
            .collect();
        let listed: std::collections::BTreeSet<String> = view.people.iter().map(|p| p.name.clone()).collect();
        assert_eq!(listed, offered, "{}", path.display());
        let present = state.map.sprites.iter().filter(|s| !s.hidden).count();
        if listed.len() < present { eprintln!("{}: {} of {present} people reachable", path.display(), listed.len()); someone_out_of_reach = true; }
    }
    assert!(someone_out_of_reach, "no fixture has anyone out of reach, so the filter is untested");
}

/// Every warp the menu cannot offer is flagged in the map view.
#[test]
fn map_view_flags_every_warp_the_menu_cannot_offer() {
    use crate::pokemon::observe;
    use crate::pokemon::tile::MetaTile;
    let mut something_out_of_reach = false;
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data")).expect("the fixture directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|e| e != "bin") { continue; }
        let snapshot = std::fs::read(&path).expect("a readable fixture");
        let mut fixture = TestFixture::new(&snapshot, Duration::from_secs(10), vec![]);
        let Ok(state) = fixture.try_game_state() else { continue };
        let view = observe::map_view(&state);
        // Keyed on the warp's own tile, where its label is drawn, not on where it lands.
        let reachable: std::collections::BTreeSet<(u8, u8)> = view.warps.iter()
            .filter(|w| w.reachable_from_here)
            .map(|w| (w.at.x, w.at.y))
            .collect();
        for action in state.map.actions() {
            if !matches!(action.tile, MetaTile::Warp { .. }) { continue }
            let at = (action.destination.x, action.destination.y);
            assert!(reachable.contains(&at),
                    "{}: the menu offers the warp at {at:?} and `read_map` says there is no route to it",
                    path.display());
        }
        if view.warps.iter().any(|w| !w.reachable_from_here) { something_out_of_reach = true; }
    }
    assert!(something_out_of_reach, "no fixture has a door out of reach, so the flag is untested");
}

/// The badge strip the web UI draws, against a snapshot that has actually earned some.
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

/// The battle view of a live battle, including an option list that is never empty.
#[test]
fn battle_view_describes_a_live_battle() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(BATTLE_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();

    let battle = observe::battle(&state).expect("the battle snapshot should be in a battle");
    assert!(battle.player.hp <= battle.player.max_hp);
    assert!(battle.enemy.hp <= battle.enemy.max_hp);
    assert!(battle.player.level > 0 && battle.enemy.level > 0);
    // The legal actions come from the turn's battle menu, not this view.
    assert!(!crate::pokemon::policy::battle_options(&state).unwrap_or_default().is_empty(),
            "a battle with no legal action would deadlock the agent");
    assert!(!battle.player.moves.is_empty(), "the active Pokémon knows no moves");
    assert_eq!(observe::status(&state, &fixture.api()).in_battle, true);
    assert_eq!(battle.active_party_slot as usize, {
        let slot = battle.active_party_slot as usize;
        assert!(slot < state.pokemon.len(), "active slot {slot} is outside the party");
        slot
    });
}

/// Routes run only over visited maps: an absent map means unvisited, never unreachable.
#[test]
fn a_route_is_only_ever_over_ground_already_walked() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60),
                                       vec![PolicyStep::goto(Map::Route1)]);
    assert!(observe::known_maps(fixture.agent.world_graph()).is_empty(), "nothing has been walked yet");
    assert!(observe::route(fixture.agent.world_graph(), Map::PalletTown, Map::Route1).is_none(),
            "a route cannot be known before the ground under it has been");

    fixture.step_until_exhausted();

    let known = observe::known_maps(fixture.agent.world_graph());
    assert!(known.contains(&Map::PalletTown), "Pallet Town was walked; saw {known:?}");

    let route = observe::route(fixture.agent.world_graph(), Map::PalletTown, Map::Route1)
        .expect("Route 1 was walked to, so there is a way back to it");
    assert_eq!(route.first().map(|hop| hop.map.as_str()), Some(format!("{}", Map::PalletTown).as_str()),
               "a route opens on the map it starts from: {route:?}");
    assert_eq!(route.last().map(|hop| hop.map.as_str()), Some(format!("{}", Map::Route1).as_str()),
               "…and ends on the one asked for: {route:?}");
    assert!(route[0].via.is_none(), "the first hop is stood on, not entered");
    assert!(route[1..].iter().all(|hop| hop.via.as_deref().is_some_and(|v| v.starts_with("Connection at ("))),
            "every later hop says how, and which tile of the map before it to leave by: {route:?}");

    // An unvisited map is `None`, meaning not visited, never nonexistent.
    assert!(observe::route(fixture.agent.world_graph(), Map::PalletTown, Map::CinnabarIsland).is_none());
}

/// Under `--features slow-tests` the views serialise.
#[test]
fn observation_views_serialise_to_json() {
    use crate::pokemon::observe;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(10), vec![]);
    let state = fixture.game_state();
    let json = serde_json::to_value(observe::map_view(&state)).expect("map view should serialise");

    assert_eq!(json["map"], format!("{}", state.map.map));
    // `Point` is a struct so the coordinates are named.
    assert_eq!(json["position"]["x"], state.map.player_position.x);
    // The terrain is a picture, so names and quotable coordinates are what must serialise.
    assert!(json["warps"].is_array(), "warps is an array");
    // `people`, not `sprites`.
    assert!(json["people"].is_array(), "people is an array");
    assert!(json.get("sprites").is_none(), "and nothing is called a sprite");
    assert_eq!(json["height"], state.map.height);
    assert!(json.get("grid").is_none() && json.get("legend").is_none(),
            "the ASCII grid was replaced by the rendered map, not kept beside it");

    for value in [serde_json::to_value(observe::party(&state)).unwrap(),
                  serde_json::to_value(observe::status(&state, &fixture.api())).unwrap(),
                  serde_json::to_value(observe::bag(&state, &fixture.api())).unwrap()] {
        assert!(!value.is_null());
    }
}

/// The generic PC menu is a closed loop under A-only input, and this is the test that says so.
#[test]
fn the_generic_pc_menu_is_backed_out_of_rather_than_mashed() {
    let mut fixture = TestFixture::new(
        crate::pokemon::data::START_OF_GAME,
        Duration::from_secs(180),
        vec![
            PolicyStep::UsePc { map: Map::RedsHouse2F },
            PolicyStep::goto(Map::RedsHouse1F),
        ],
    );

    let mut opened = false;
    while !fixture.agent.policy_exhausted() {
        fixture.step();
        opened |= fixture.api().in_pc_menu();
    }

    assert!(opened, "the PC never opened, so this test says nothing about leaving one");
    assert!(!fixture.api().in_pc_menu(), "the run finished still inside the PC menu");
    assert_eq!(fixture.game_state().map.map, Map::RedsHouse1F,
               "the agent should have logged off and walked downstairs");
}

// ── The START menu: the row index, and never A-mashing one that was left open ──

#[test]
fn the_item_row_of_the_start_menu_is_found_without_the_pokedex() {
    use crate::pokemon::agent::{start_menu_row, AgentState, StartMenuRow};
    use crate::pokemon::item::ItemId;
    use crate::pokemon::pokedex::PokedexReader;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60), vec![]);
    for _ in 0..20 { fixture.step(); }
    assert!(!fixture.api().mmu().read_has_pokedex(),
            "this fixture has to predate the Pokédex or the test asserts nothing");
    assert_eq!(start_menu_row(&fixture.api(), StartMenuRow::Item), 1,
               "without the Pokédex, ITEM is row 1 — row 2 is the trainer card");
    assert_eq!(start_menu_row(&fixture.api(), StartMenuRow::Pokemon), 0,
               "without the Pokédex, POKéMON is row 0");

    // Now make the game agree: a driver navigates START then ITEM in exactly that window.
    let mut fixture = TestFixture::new(ROUTE1_STATE, Duration::from_secs(60), vec![]);
    for _ in 0..20 { fixture.step(); }
    assert!(!fixture.api().mmu().read_has_pokedex(), "Route 1 has to predate the Pokédex too");
    let item: ItemId = fixture.api().game_state().expect("game state")
        .bag.iter().next().expect("Route 1's bag should not be empty").id;

    fixture.agent.set_state(AgentState::TossingItem { item, press: true, entered_menu: false });

    let mut reached_the_bag = false;
    for _ in 0..600 {
        fixture.step();
        // BADGES appears on `DrawTrainerInfo`, the failure mode, and on no other screen here.
        if let Some(text) = fixture.api().on_screen_text(false) {
            assert!(!text.contains("BADGES"),
                    "the driver opened the trainer card instead of the bag: {text:?}");
            if text.contains("CANCEL") {
                reached_the_bag = true;
                break;
            }
        }
    }
    assert!(reached_the_bag, "the toss driver never reached the bag");
}

/// A menu the agent did not open is closed, not confirmed.
#[test]
fn a_menu_left_open_is_closed_rather_than_confirmed() {
    use gb::joypad::JoypadButton;

    let mut fixture = TestFixture::new(PALLET_TOWN_STATE, Duration::from_secs(60), vec![]);
    for _ in 0..20 { fixture.step(); }
    assert_eq!(fixture.api().game_mode(), Some(GameMode::Overworld), "expected a quiet overworld");

    fixture.agent.queue_manual_input([JoypadButton::Start]);
    for _ in 0..MANUAL_INPUT_TICKS_PER_PRESS { fixture.step(); }
    assert_eq!(fixture.api().game_mode(), Some(GameMode::TextBox), "START should have opened the menu");

    // Well inside `TEXT_BOX_ESCAPE_SILENCE`: the hand-over acts as soon as the menu has drawn.
    let mut closed = false;
    for _ in 0..400 {
        fixture.step();
        if fixture.api().game_mode() == Some(GameMode::Overworld) {
            closed = true;
            break;
        }
    }
    assert!(closed, "the agent should have left the START menu rather than pressing A into it");
}

/// A battle turn is put to the policy once, at the main menu, not again from its move list.
#[test]
fn a_battle_turn_is_decided_once_rather_than_twice() {
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::policy::Policy;
    use crate::pokemon::world_graph::WorldGraph;
    use std::sync::{Arc, Mutex};

    /// Where the game was every time the policy was asked for a battle action.
    #[derive(Default)]
    struct Log {
        /// The `battle_menu_state()` at each decision point, as prose.
        asked_at: Vec<String>,
        enemy_hp: Vec<u16>,
        menu: String,
    }

    /// Always the first move, with an LLM's latency bolted on.
    struct Probe { latency: u32, remaining: Option<u32>, log: Arc<Mutex<Log>> }

    impl Policy for Probe {
        fn name(&self) -> &'static str { "scripted" }

        fn service_tools(&mut self, _: &GameState, api: &mut PokemonApi<'_>, _: &WorldGraph) {
            self.log.lock().expect("the log is never poisoned").menu =
                format!("{:?}", api.menu_state().and_then(|m| m.battle_menu_state()));
        }

        fn pick_overworld_action(&mut self, _: &GameState, _: &WorldGraph) -> Option<OverworldAction> { None }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            match self.remaining {
                // The poll that opens a turn: record what the agent thinks is on screen.
                None => {
                    self.remaining = Some(self.latency);
                    let mut log = self.log.lock().expect("the log is never poisoned");
                    let menu = log.menu.clone();
                    log.asked_at.push(menu);
                    log.enemy_hp.push(state.battle.as_ref().map_or(0, |b| b.enemy.current_hp));
                    None
                }
                Some(0) => {
                    self.remaining = None;
                    state.battle.as_ref().and_then(|b| b.player.moves[0])
                        .map(|battle_move| BattleAction::Fight { slot: 0, battle_move })
                }
                Some(n) => { self.remaining = Some(n - 1); None }
            }
        }
    }

    for latency in [1u32, 75] {
        let log = Arc::new(Mutex::new(Log::default()));
        let policy = Probe { latency, remaining: None, log: Arc::clone(&log) };
        let mut fixture = TestFixture::with_policy(
            BATTLE_STATE, Duration::from_secs(120), Box::new(policy));

        let mut ticks = 0;
        while fixture.total_cycles < fixture.max_cycles {
            ticks += 1;
            fixture.step();
            // The fixture opens mid-battle; stop once it is over.
            if ticks > 50 && fixture.try_game_state().map_or(true, |s| s.battle.is_none()) {
                break;
            }
        }

        let log = log.lock().expect("the log is never poisoned");
        let off_menu: Vec<_> = log.asked_at.iter()
            .filter(|menu| menu.as_str() != "Some(Fight)").collect();
        assert!(off_menu.is_empty(),
                "at {latency} ticks of latency the policy was asked for a battle action somewhere \
                 other than the main battle menu: {off_menu:?} (all decision points: {:?})",
                log.asked_at);

        // Every landed move costs the enemy HP, so a decision point that changed nothing was
        // wasted.
        let landed = log.enemy_hp.windows(2).filter(|w| w[1] < w[0]).count();
        assert!(log.asked_at.len() <= landed + 1,
                "at {latency} ticks of latency the policy was asked {} times to land {landed} \
                 moves; enemy HP at each decision point was {:?}",
                log.asked_at.len(), log.enemy_hp);
        assert!(landed >= 2,
                "at {latency} ticks of latency nothing landed, so this run asserts nothing: \
                 enemy HP at each decision point was {:?}", log.enemy_hp);
    }
}

/// A Cut nobody can use never reaches the party menu.
#[test]
fn a_cut_with_no_cut_never_opens_the_party_menu() {
    /// Answers every field-move poll with `CutTree`.
    struct AlwaysCuts;
    impl crate::pokemon::policy::Policy for AlwaysCuts {
        fn name(&self) -> &'static str { "always-cuts" }
        fn pick_overworld_action(&mut self, _state: &GameState, _graph: &crate::pokemon::world_graph::WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction> { None }
        fn pick_battle_action(&mut self, _state: &GameState) -> Option<crate::pokemon::battle::BattleAction> {
            Some(crate::pokemon::battle::BattleAction::Run)
        }
        fn pick_field_move(&mut self, _state: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
            Some(crate::pokemon::policy::FieldMove::CutTree)
        }
    }

    const RUN_FOR: Duration = Duration::from_secs(30);
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/at-vermilion.bin"), RUN_FOR * 2, Box::new(AlwaysCuts),
    );
    assert!(!fixture.game_state().can_use_cut,
            "this fixture reaches Vermilion before the HM, or the test proves nothing");

    let mut refusals = 0;
    let mut worst_silence = Duration::ZERO;
    let budget = MachineCycles::from_duration(RUN_FOR);
    let mut emulated = MachineCycles::ZERO;
    while emulated < budget {
        fixture.step();
        emulated += AGENT_RESOLUTION;
        let state = fixture.agent.state_debug();
        assert!(!state.starts_with("cut"), "the agent entered the Cut driver with no Cut: {state}");
        worst_silence = worst_silence.max(fixture.agent.since_last_policy_poll());
        for event in fixture.agent.drain_events() {
            if let AgentEvent::TextBox { message } = &event
                && message.contains("Cut needs a party member") { refusals += 1 }
        }
    }
    assert!(refusals > 0, "the refusal has to be said out loud, or nothing explains the no-op");
    assert!(worst_silence < Duration::from_secs(10),
            "the agent went {worst_silence:?} without asking anything — a guard that wedges is not a fix");
}

#[test]
fn a_guard_who_turns_you_back_is_quoted_rather_than_swallowed() {
    let mut fixture = TestFixture::new(
        ROUTE22_GATE,
        Duration::from_secs(120),
        vec![PolicyStep::goto(Map::Route23)],
    );

    let mut aborted = false;
    let mut heard: Vec<String> = Vec::new();
    for _ in 0..6000 {
        fixture.step();
        for event in fixture.agent.drain_events() {
            match event {
                AgentEvent::OverworldActionAborted { reason: OverworldActionAbortedReason::Textbox, .. } =>
                    aborted = true,
                AgentEvent::TextBox { message } => heard.push(message),
                _ => {}
            }
        }
        if aborted && !heard.is_empty() { break; }
    }

    // Without the abort the guard was walked past and this proves nothing.
    assert!(aborted, "the walk to Route 23 should have been stopped by the guard; heard {heard:?}");
    assert!(
        heard.iter().any(|line| line.contains("BOULDERBADGE")),
        "the guard says why he will not let you past, and the model has to be told; heard {heard:?}",
    );
}

/// The same fixture, from the other side: the model asking the guard directly.
#[test]
fn talking_to_that_guard_reports_what_he_said() {
    let mut fixture = TestFixture::new(
        ROUTE22_GATE,
        Duration::from_secs(120),
        vec![PolicyStep::Interact(MapSprite::ROUTE22GATE_GUARD)],
    );

    let mut talked = false;
    let mut heard: Vec<String> = Vec::new();
    for _ in 0..6000 {
        fixture.step();
        for event in fixture.agent.drain_events() {
            match event {
                AgentEvent::OverworldInteractionCompleted { .. } => talked = true,
                AgentEvent::TextBox { message } => heard.push(message),
                _ => {}
            }
        }
        if talked && !heard.is_empty() { break; }
    }

    assert!(talked, "the guard should have been reached and spoken to; heard {heard:?}");
    assert!(
        heard.iter().any(|line| line.contains("BOULDERBADGE")),
        "asking him what the problem is has to answer with what he said; heard {heard:?}",
    );
}

#[test]
fn teaching_an_hm_to_a_mon_that_cannot_learn_it_does_not_wedge() {
    /// Asks for the same impossible teach on every poll, as an unguarded `LlmPolicy` would.
    struct AlwaysTeaches;
    impl crate::pokemon::policy::Policy for AlwaysTeaches {
        fn name(&self) -> &'static str { "scripted" }
        fn pick_overworld_action(&mut self, _: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction> { None }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<crate::pokemon::battle::BattleAction> { None }
        fn pick_field_move(&mut self, _: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
            // Slot 0, because that is the one that *cannot* take it.
            Some(crate::pokemon::policy::FieldMove::TeachMove { item: ItemId::Hm01Cut, target_slot: 0 })
        }
    }

    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_secs(240),
        Box::new(AlwaysTeaches),
    );

    let mut worst = Duration::ZERO;
    let mut worst_state = String::new();
    let mut heard: Vec<String> = Vec::new();
    for _ in 0..3000 {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::TextBox { message } = event { heard.push(message); }
        }
        let gap = fixture.agent.since_last_policy_poll();
        if gap > worst { worst = gap; worst_state = fixture.agent.state_debug(); }
    }

    println!("[teach] worst silence {worst:?} in {worst_state}, {} reported", heard.len());

    // Decisions keep coming: a guard that left the agent idle would also never enter
    // `TeachingMove`.
    assert!(
        worst < Duration::from_secs(30),
        "a teach the game will refuse went {worst:?} of game time without reaching a decision \
         point — wedged in {worst_state}. heard {heard:?}",
    );

    let said = heard.first().unwrap_or_else(|| panic!("nothing was reported at all; state {worst_state}"));
    assert!(said.contains("cannot learn Cut"), "it has to name the refusal: {said}");
    assert!(said.contains("slot 1"), "and who in the party can take it instead: {said}");
}

#[test]
fn using_an_item_the_game_will_not_use_does_not_wedge() {
    /// Asks for the same impossible use on every poll, as an unguarded `LlmPolicy` would.
    struct AlwaysUsesTheFossil;
    impl crate::pokemon::policy::Policy for AlwaysUsesTheFossil {
        fn name(&self) -> &'static str { "scripted" }
        fn pick_overworld_action(&mut self, _: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction> { None }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<crate::pokemon::battle::BattleAction> { None }
        fn pick_field_move(&mut self, _: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
            // Sailor 1, at (19, 30) on Vermilion's dock, three steps from the fixture.
            Some(crate::pokemon::policy::FieldMove::UseFieldItem {
                item: ItemId::HelixFossil,
                target: gb::geometry::Point8 { x: 19, y: 30 },
            })
        }
    }

    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_secs(240),
        Box::new(AlwaysUsesTheFossil),
    );

    assert!(
        fixture.game_state().bag.iter().any(|it| it.id == ItemId::HelixFossil),
        "the fixture has to be carrying the fossil, or this is a test about an empty bag",
    );

    let mut worst = Duration::ZERO;
    let mut worst_state = String::new();
    let mut heard: Vec<String> = Vec::new();
    for _ in 0..3000 {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::TextBox { message } = event { heard.push(message); }
        }
        let gap = fixture.agent.since_last_policy_poll();
        if gap > worst { worst = gap; worst_state = fixture.agent.state_debug(); }
    }

    println!("[use-item] worst silence {worst:?} in {worst_state}, {} reported", heard.len());

    assert!(
        worst < Duration::from_secs(30),
        "a use the game will refuse went {worst:?} of game time without reaching a decision point, \
         wedged in {worst_state}. heard {heard:?}",
    );

    let said = heard.first().unwrap_or_else(|| panic!("nothing was reported at all; state {worst_state}"));
    assert!(said.contains("HelixFossil"), "it has to name what was refused: {said}");
    assert!(said.contains("carry"), "and say the item is carried rather than used: {said}");
}

/// An item the ROM table calls usable, refused by where the player stands, is backed out of.
#[test]
fn an_item_the_map_refuses_backs_out_rather_than_mashing_for_a_minute() {
    struct AlwaysRopes;
    impl crate::pokemon::policy::Policy for AlwaysRopes {
        fn name(&self) -> &'static str { "scripted" }
        fn pick_overworld_action(&mut self, _: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction> { None }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<crate::pokemon::battle::BattleAction> { None }
        fn pick_field_move(&mut self, _: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
            Some(crate::pokemon::policy::FieldMove::UseFieldItem {
                item: ItemId::EscapeRope,
                target: gb::geometry::Point8 { x: 19, y: 30 },
            })
        }
    }

    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_secs(240),
        Box::new(AlwaysRopes),
    );
    // `pimp_pokemon` writes `EPIC_BAG`, which carries the rope; the fixture's own bag does not.
    fixture.pimp_pokemon();
    assert!(
        fixture.game_state().bag.iter().any(|it| it.id == ItemId::EscapeRope),
        "the rope has to be in the bag or the driver never reaches the refusal",
    );

    let mut worst = Duration::ZERO;
    let mut worst_state = String::new();
    let mut heard: Vec<String> = Vec::new();
    for _ in 0..3000 {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::TextBox { message } = event { heard.push(message); }
        }
        let gap = fixture.agent.since_last_policy_poll();
        if gap > worst { worst = gap; worst_state = fixture.agent.state_debug(); }
    }

    println!("[rope] worst silence {worst:?} in {worst_state}, {} reported", heard.len());

    assert!(
        worst < Duration::from_secs(30),
        "a rope the map refuses went {worst:?} of game time without reaching a decision point, \
         wedged in {worst_state}. heard {heard:?}",
    );
    assert!(
        heard.iter().any(|line| line.contains("refused to use EscapeRope")),
        "the refusal has to be reported in words rather than as a timeout; heard {heard:?}",
    );
}

/// A pickup that the game refuses looks exactly like one that worked.
#[test]
fn a_pickup_the_game_refuses_says_the_item_is_still_there() {
    // Charmander's ball, not Bulbasaur's.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/oaks-lab-just-got-squirtle.bin"),
        Duration::from_secs(120),
        vec![PolicyStep::Interact(MapSprite::OAKSLAB_CHARMANDER_POKE_BALL)],
    );

    // Not `step_until_exhausted`.
    let mut events = Vec::new();
    for _ in 0..3000 {
        fixture.step();
        events.extend(fixture.agent.drain_events());
        if events.iter().any(|e| matches!(e, AgentEvent::OverworldPickupFailed { .. })) {
            break;
        }
    }

    let said = events.iter().map(|e| format!("{e}")).collect::<Vec<_>>().join("\n");
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::OverworldPickupFailed { target: MetaTile::Sprite("Charmander Poke Ball") },
        )),
        "the ball is still on the floor and nothing said so:\n{said}",
    );
    // Read out of the rendered line, not the source.
    let line = events
        .iter()
        .find(|e| matches!(e, AgentEvent::OverworldPickupFailed { .. }))
        .map(|e| format!("{e}"))
        .expect("asserted above");
    assert_eq!(
        line,
        "✗ nothing was picked up: the Charmander Poke Ball is still lying there. \
         The message above says why.",
        "it has to read as a fact rather than as a malfunction, and with single spaces",
    );
}

/// A pickup that works reports no failure.
#[test]
fn a_pickup_that_works_reports_no_failure() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/viridian-forest.bin"),
        Duration::from_secs(300),
        vec![PolicyStep::Interact(MapSprite::VIRIDIANFOREST_ANTIDOTE)],
    );

    let mut events = Vec::new();
    while !fixture.agent.policy_exhausted() {
        fixture.step();
        events.extend(fixture.agent.drain_events());
    }
    for _ in 0..200 {
        fixture.step();
        events.extend(fixture.agent.drain_events());
    }

    let said = events.iter().map(|e| format!("{e}")).collect::<Vec<_>>().join("\n");
    assert!(
        said.contains("Antidote"),
        "the leg has to actually reach the Antidote or it proves nothing:\n{said}",
    );
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::OverworldPickupFailed { .. })),
        "the Antidote was picked up, so nothing failed:\n{said}",
    );
}

#[test]
fn the_mt_moon_moon_stone_is_not_reported_as_still_lying_there() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/mt-moon.bin"),
        Duration::from_secs(600),
        // Re-issued rather than issued once.
        vec![PolicyStep::Interact(MapSprite::MTMOON1F_MOON_STONE); 30],
    );

    // Not `step_until_exhausted`, and not for the reason the refusal test says.
    let mut events = Vec::new();
    let mut settle = 0;
    for _ in 0..200_000 {
        fixture.step();
        events.extend(fixture.agent.drain_events());
        match events.iter().any(|e| format!("{e}").contains("MOON STONE")) {
            true => settle += 1,
            false => {}
        }
        if settle > 300 {
            break;
        }
    }

    let said = events.iter().map(|e| format!("{e}")).collect::<Vec<_>>().join("\n");
    assert!(said.contains("MOON STONE"), "the leg has to actually reach it or it proves nothing:\n{said}");
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::OverworldPickupFailed { .. })),
        "the Moon Stone was picked up, so nothing failed:\n{said}",
    );
}

use crate::pokemon::battle::BattleAction;

/// Fights with a nominated move slot each turn and records what it asked for.
struct AlternatingMoves {
    asked: std::rc::Rc<std::cell::RefCell<Vec<crate::pokemon::move_name::PokemonMoveName>>>,
    /// Every `pick_move_to_forget` call, so a menu that asks more than once is visible.
    forget_calls: std::rc::Rc<std::cell::RefCell<Vec<crate::pokemon::move_name::PokemonMoveName>>>,
    turn: usize,
}

impl crate::pokemon::policy::Policy for AlternatingMoves {
    fn name(&self) -> &'static str { "alternating" }

    fn pick_overworld_action(
        &mut self,
        state: &GameState,
        _graph: &crate::pokemon::world_graph::WorldGraph,
    ) -> Option<crate::pokemon::actions::OverworldAction> {
        // Stand in the grass so wild battles keep arriving; each one is more turns to sample.
        state.map.actions().into_iter().find(|action| matches!(action.tile, MetaTile::Grass))
    }

    fn pick_move_to_forget(
        &mut self,
        _slot: usize,
        _moves: &[crate::pokemon::move_name::PokemonMove],
        new_move: crate::pokemon::move_name::PokemonMoveName,
    ) -> Option<Option<usize>> {
        self.forget_calls.borrow_mut().push(new_move);
        Some(Some(1))
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let options = crate::pokemon::policy::battle_options(state)?;
        let moves: Vec<&BattleAction> = options.iter()
            .filter(|option| matches!(option, BattleAction::Fight { .. }))
            .collect();
        if moves.is_empty() {
            return options.iter().find(|o| matches!(o, BattleAction::Run)).copied();
        }
        // Highest slot, then lowest: the longest cursor travel, ending every other turn on slot 0.
        let chosen = match self.turn % 2 {
            0 => moves[moves.len() - 1],
            _ => moves[0],
        };
        self.turn += 1;
        if let BattleAction::Fight { battle_move, .. } = chosen {
            self.asked.borrow_mut().push(battle_move.name);
        }
        Some(*chosen)
    }
}

/// The move the agent confirms is the move the policy asked for.
#[test]
fn the_move_the_agent_confirms_is_the_move_the_policy_asked_for() {
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/viridian-forest.bin"),
        Duration::from_secs(1200),
        Box::new(AlternatingMoves {
            asked: std::rc::Rc::clone(&asked),
            forget_calls: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            turn: 0,
        }),
    );

    let mut events = Vec::new();
    // 20 ms a tick against the fixture's own 1200 s game-time budget.
    for _ in 0..55_000 {
        fixture.step();
        events.extend(fixture.agent.drain_events());
    }

    // Pair each published intent with the sentence the cartridge printed next.
    let mut intent: Option<(String, String)> = None;
    let mut matched = 0usize;
    let mut mismatched: Vec<(String, String)> = Vec::new();
    for event in &events {
        let line = format!("{event}");
        match event {
            AgentEvent::BattleActionStarted { actor, action: BattleAction::Fight { battle_move, .. }, .. } =>
                intent = Some((actor.to_string(), battle_move.name.to_string())),
            AgentEvent::TextBox { message } => {
                let Some((actor, wanted)) = intent.clone() else { continue };
                let Some(rest) = message.split(&format!("{actor} used ")).nth(1) else { continue };
                let Some(actual) = rest.split('!').next() else { continue };
                let normalise = |s: &str| s.to_lowercase().replace(['-', ' '], "");
                // Struggle is the cartridge overriding the choice with no PP left, not a wrong
                // cursor.
                if normalise(actual) != "struggle" {
                    match normalise(actual) == normalise(&wanted) {
                        true => matched += 1,
                        false => mismatched.push((wanted, actual.to_string())),
                    }
                }
                intent = None;
            }
            _ => {}
        }
        let _ = line;
    }

    eprintln!("sampled {matched} matched, {} mismatched", mismatched.len());
    // The ceiling is the harness, not the agent.
    assert!(matched >= 10, "only {matched} battle turns were sampled, which proves nothing");
    assert!(
        mismatched.is_empty(),
        "{} of {} turns executed a different move from the one chosen: {mismatched:?}",
        mismatched.len(),
        matched + mismatched.len(),
    );
}

/// Counts `pick_move_to_forget` and otherwise plays the scripted route it wraps.
struct CountingForget {
    inner: crate::pokemon::policy::DeterministicPolicy,
    calls: std::rc::Rc<std::cell::RefCell<usize>>,
}

impl crate::pokemon::policy::Policy for CountingForget {
    fn name(&self) -> &'static str { "counting-forget" }
    fn pick_overworld_action(&mut self, state: &GameState, graph: &crate::pokemon::world_graph::WorldGraph)
        -> Option<crate::pokemon::actions::OverworldAction> { self.inner.pick_overworld_action(state, graph) }
    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        self.inner.pick_battle_action(state)
    }
    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        self.inner.pick_nickname(species)
    }
    fn pick_field_move(&mut self, state: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
        self.inner.pick_field_move(state)
    }
    fn pick_move_to_forget(&mut self, slot: usize, moves: &[crate::pokemon::move_name::PokemonMove],
                           new_move: crate::pokemon::move_name::PokemonMoveName) -> Option<Option<usize>> {
        *self.calls.borrow_mut() += 1;
        self.inner.pick_move_to_forget(slot, moves, new_move)
    }
    fn is_exhausted(&self) -> bool { self.inner.is_exhausted() }
    fn steps_remaining(&self) -> Option<usize> { self.inner.steps_remaining() }
    fn current_step_is_long_running(&self) -> bool { self.inner.current_step_is_long_running() }
}

/// The forget menu asks the policy once, not once a tick.
#[test]
fn the_forget_menu_asks_the_policy_once_rather_than_once_a_tick() {
    let calls = std::rc::Rc::new(std::cell::RefCell::new(0usize));
    // TM34 is in the bag and the Wartortle knows four moves, so the teach opens the forget menu.
    let policy = CountingForget {
        inner: crate::pokemon::policy::DeterministicPolicy::new(42, vec![PolicyStep::TeachMove {
            item: ItemId::Tm34Bide,
            target: crate::pokemon::policy::PartyRef::Slot(0),
        }]),
        calls: std::rc::Rc::clone(&calls),
    };
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/mt-moon.bin"),
        Duration::from_secs(300),
        Box::new(policy),
    );

    // Counted off the screen, not off a text box.
    let mut learns = 0usize;
    let mut showing = false;
    for _ in 0..12_000 {
        fixture.step();
        fixture.agent.drain_events();
        let up = {
            let api = fixture.api();
            use crate::pokemon::PokemonApiTrait;
            api.on_screen_text(true).map_or(false, |t| crate::pokemon::menu::is_forget_move_prompt(&t))
        };
        if up && !showing {
            learns += 1;
        }
        showing = up;
    }

    eprintln!("forget menus: {learns}, policy asked: {}", calls.borrow());
    assert!(learns > 0, "the teach never reached the forget menu, so this proves nothing");
    assert_eq!(
        *calls.borrow(), learns,
        "the policy must be asked once per forget menu, not once per tick of driving it",
    );
}

/// A shop left waiting on the policy is not mashed through: no refusals and no spend.
#[test]
fn a_shop_left_waiting_on_the_policy_is_not_mashed_through() {
    let (money, refusals, before, after) = shop_with_a_policy_that_never_answers(137);

    assert_eq!(refusals, 0, "the game was asked {refusals} times to sell something it cannot");
    assert_eq!(money, 137, "and nothing was spent");
    assert_eq!(after, before, "on a bag nothing was added to");
}

/// With money for the first row, nothing is bought until the policy chooses.
#[test]
fn a_shop_is_not_raided_while_the_policy_is_still_thinking() {
    let (money, _refusals, before, after) = shop_with_a_policy_that_never_answers(1200);

    assert_eq!(money, 1200, "the wallet is untouched until the policy chooses; bag {after:?}");
    assert_eq!(after, before, "and the first row of the stock list is not bought by default");
}

/// Walks to the Viridian Mart under a policy that opens the shop and never answers.
/// Returns the money left, the refusals seen, and the bag before and after.
fn shop_with_a_policy_that_never_answers(money: u32) -> (u32, usize, Vec<(ItemId, u8)>, Vec<(ItemId, u8)>) {
    use crate::pokemon::map_metadata::MapMetadataCache;
    use gb::game_boy::GameBoy;
    use crate::pokemon::policy::{DeterministicPolicy, Policy};
    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::world_graph::WorldGraph;

    /// A `DeterministicPolicy` that walks to the clerk and then thinks about the shop for ever.
    struct StillThinking(DeterministicPolicy);

    impl Policy for StillThinking {
        fn name(&self) -> &'static str { "still-thinking" }

        fn pick_overworld_action(&mut self, state: &GameState, graph: &WorldGraph) -> Option<OverworldAction> {
            self.0.pick_overworld_action(state, graph)
        }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            self.0.pick_battle_action(state)
        }

        // The whole of the fixture: the shop is open and the answer is still coming.
        fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
            None
        }
    }

    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../data/viridian-city-pokemart-shopping.bin")).expect("fixture loads");
    let mut cache = MapMetadataCache::default();
    PokemonApi::with_cache(&mut gb, &mut cache).debug_set_money(money);
    // The fixture carries a Town Map and a Potion, so nothing bought means the bag is unchanged.
    let before: Vec<(ItemId, u8)> = PokemonApi::with_cache(&mut gb, &mut cache)
        .game_state().expect("readable").bag.iter().map(|i| (i.id, i.quantity)).collect();

    let steps = vec![PolicyStep::BuyFromMart {
        map: Map::ViridianMart,
        item: BagItem::new(ItemId::PokeBall, 1),
    }];
    let mut agent = PokemonAgent::new(Box::new(StillThinking(DeterministicPolicy::new(1, steps))));

    // Inside `DRIVER_ESCAPE_SILENCE`, the net under a driver that stops making progress.
    let budget = MachineCycles::from_duration(Duration::from_secs(30));
    let mut emulated = MachineCycles::ZERO;
    // Counted off the screen on a rising edge, not out of `AgentEvent::TextBox`.
    let mut refusals = 0usize;
    let mut refusing = false;
    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let showing = PokemonApi::with_cache(&mut gb, &mut cache)
            .on_screen_text(true)
            .is_some_and(|text| text.contains("don't have enough money"));
        if showing && !refusing {
            refusals += 1;
        }
        refusing = showing;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        agent.drain_events();
    }

    let state = PokemonApi::with_cache(&mut gb, &mut cache).game_state().expect("readable");
    let after = state.bag.iter().map(|i| (i.id, i.quantity)).collect();
    (state.money, refusals, before, after)
}

/// A black-out is not a decision point until its warp has landed.
#[test]
fn a_blackout_is_not_a_decision_point_until_the_warp_has_landed() {
    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::policy::Policy;
    use crate::pokemon::world_graph::WorldGraph;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Log {
        overworld: Vec<(Map, bool, u16)>,
        /// Battle decision points reached.
        battles_fought: usize,
    }

    /// Fights with what it has and never walks, so every overworld decision logged is the agent's.
    struct Probe { log: Arc<Mutex<Log>> }

    impl Policy for Probe {
        fn name(&self) -> &'static str { "blackout-probe" }

        fn pick_overworld_action(&mut self, state: &GameState, _: &WorldGraph) -> Option<OverworldAction> {
            let hp: u16 = state.pokemon.iter().map(|mon| mon.current_hp).sum();
            self.log.lock().expect("the log is never poisoned")
                .overworld.push((state.map.map, state.battle.is_some(), hp));
            None
        }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            self.log.lock().expect("the log is never poisoned").battles_fought += 1;
            state.battle.as_ref().and_then(|b| b.player.moves[0])
                .map(|battle_move| BattleAction::Fight { slot: 0, battle_move })
        }
    }

    let log = Arc::new(Mutex::new(Log::default()));
    let mut fixture = TestFixture::with_policy(
        BATTLE_STATE, Duration::from_secs(180), Box::new(Probe { log: Arc::clone(&log) }));

    // Read before the warp, after which the cartridge answers with the Centre's town.
    let battlefield = fixture.game_state().map.map;

    // Let the battle get going, then take the party out from under it.
    for _ in 0..60 { fixture.step(); }
    fixture.api().debug_faint_party();

    let mut blacked_out = false;
    while fixture.total_cycles < fixture.max_cycles {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::TextBox { message } = &event {
                if crate::llm::battle_report::is_blackout(message) { blacked_out = true; }
            }
        }
        if blacked_out && log.lock().expect("the log is never poisoned").overworld.len() >= 2 {
            break;
        }
    }

    let log = log.lock().expect("the log is never poisoned");
    // A run that never fought and never lost has no gap to ask in.
    assert!(log.battles_fought > 0, "the probe never reached a battle decision, so nothing was lost");
    assert!(blacked_out, "the party was knocked out but the cartridge never said the player blacked out");

    let first = *log.overworld.first()
        .expect("the agent should ask for an overworld action once the black-out warp has landed");
    let (map, had_battle, hp) = first;
    assert_ne!(map, battlefield,
               "the first overworld decision after a black-out was put on {battlefield:?}, the map \
                the fight was on — the warp had not run yet. All of them: {:?}", log.overworld);
    assert!(!had_battle,
            "the first overworld decision after a black-out carried a live battle; `wIsInBattle` \
             was the {:#04x} loss sentinel and was read as a fight in progress. All of them: {:?}",
            crate::pokemon::battle::LOST_BATTLE, log.overworld);
    assert!(hp > 0,
            "the first overworld decision after a black-out showed a party on {hp} HP — the heal \
             inside `ResetStatusAndHalveMoneyOnBlackout` had not run. All of them: {:?}",
            log.overworld);
}

/// The agent's tick is 20 ms of *game* time, not one turn of whatever loop is driving it.
#[test]
fn a_corner_is_turned_at_a_coarse_host_tick() {
    // 250 ms is `host::MAX_CATCHUP`, the coarsest tick a deployment reaches; 60 ms overshoots once.
    for tick_ms in [20u64, 60, 250] {
        let mut fixture = TestFixture::new(
            include_bytes!("../data/post-snorlax.bin"),
            Duration::from_secs(120),
            vec![PolicyStep::EnterMap { to_map: Map::Route11, to_position: None }],
        );
        let tick = MachineCycles::from_duration(Duration::from_millis(tick_ms));
        let mut arrived = false;
        // The fixture's own budget; the walk is thirteen tiles.
        while !arrived {
            fixture.step_coarse(tick);
            arrived = fixture.game_state().map.map == Map::Route11;
        }
        println!("{tick_ms} ms tick: reached Route11 in {:?} of game time", fixture.total_cycles.to_duration());
    }
}

/// A coordinate that underflows a map edge is not a position, so an arrived walk has not failed.
#[test]
fn a_coordinate_that_underflows_a_map_edge_is_not_a_position() {
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use std::sync::Arc;

    let mut fixture = TestFixture::new(
        include_bytes!("../data/viridian-city-north-of-bush.bin"),
        Duration::from_secs(10),
        vec![],
    );
    let metadata = Arc::new(
        PokemonApi::new(&mut fixture.gb)
            .mmu()
            .read_map_metadata(Map::ViridianCity)
            .expect("Viridian City's map data"),
    );
    let dimensions = metadata.dimensions();
    // Viridian connects north and south, so both the underflow and the overflow are reachable.
    assert_eq!((dimensions.north_extra, dimensions.south_extra), (1, 1));

    let at = |y: u8| {
        MetaTileMap::new(&CurrentMap {
            player_position: Point8 { x: 19, y },
            player_direction: PlayerFacingDirection::Down,
            sprites: Vec::new(),
            metadata: Arc::clone(&metadata),
            closed_doors: Vec::new(),
            grass_encounter_rate: 0,
            water_encounter_rate: 0,
            card_key_locked: false,
            header_loaded: true,
            surfing: false,
            sprites_loaded: true,
            script_cancelled_warps: Vec::new(),
            standing_on_warp: true,
        })
    };

    let inside = at(10);
    assert!(inside.position_settled, "an ordinary square in the middle of the map");
    assert_eq!(inside.player_position.y, 10 + dimensions.north_extra as u8);

    // One row past the bottom is the southern connection strip, a real tile walked onto southward.
    let leaving_south = at(dimensions.meta_height as u8);
    assert!(leaving_south.position_settled, "the southern strip is a square, not a transient");

    // …and 255 is not a square at all.
    let leaving_north = at(255);
    assert!(!leaving_north.position_settled, "wYCoord == 255 is a transition, not a position");
    assert_eq!(
        leaving_north.player_position.y,
        (leaving_north.height - 1) as u8,
        "and this is the fiction the flag exists to catch: the clamp puts a player who stepped off \
         the *top* of the map on its bottom row",
    );
}

/// A map the cartridge has not finished loading offers no rows.
#[test]
fn a_map_the_cartridge_has_not_finished_loading_offers_no_rows() {
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use std::sync::Arc;

    let mmu = gb::mmu::MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    let metadata = Arc::new(mmu.read_map_metadata(Map::SafariZoneCenter).unwrap());
    let at = |x: u8, y: u8, header_loaded: bool| {
        MetaTileMap::new(&CurrentMap {
            player_position: Point8 { x, y },
            player_direction: PlayerFacingDirection::Up,
            sprites: Vec::new(),
            metadata: Arc::clone(&metadata),
            closed_doors: Vec::new(),
            grass_encounter_rate: 0,
            water_encounter_rate: 0,
            card_key_locked: false,
            header_loaded,
            surfing: false,
            sprites_loaded: true,
            script_cancelled_warps: Vec::new(),
            standing_on_warp: true,
        })
    };
    let ids = |map: &MetaTileMap| -> Vec<String> {
        map.actions().iter().map(|a| a.id()).collect()
    };

    // The gate's square, before the Centre has loaded.
    let in_flight = at(4, 0, false);
    assert!(!in_flight.position_settled, "wCurMap has changed and the map has not");
    assert!(in_flight.actions().is_empty(), "a route has to start somewhere real: {:?}", ids(&in_flight));

    // The same square with the check told the map is loaded.
    let fiction = at(4, 0, true);
    assert!(ids(&fiction).iter().any(|id| id == "SafariZoneCenter:0,10:Warp"),
        "the fiction this test exists to describe: {:?}", ids(&fiction));

    // Where the player was one map-load later.
    let landed = at(15, 25, true);
    assert!(landed.position_settled);
    assert!(!ids(&landed).iter().any(|id| id == "SafariZoneCenter:0,10:Warp"),
        "the entrance cannot reach the far shore: {:?}", ids(&landed));
    assert!(ids(&landed).iter().any(|id| id == "SafariZoneCenter:29,10:Warp"),
        "and it can reach the east one: {:?}", ids(&landed));
}

/// On a settled save, the ten bytes `LoadMapHeader` copies are `wCurMap`'s header and no other.
#[test]
fn the_header_in_wram_says_which_map_has_actually_been_loaded() {
    use crate::pokemon::map_metadata::map_header_is_loaded;

    let mut fixture = TestFixture::new(
        include_bytes!("../data/viridian-city-north-of-bush.bin"),
        Duration::from_secs(10),
        vec![],
    );
    let standing_on = fixture.game_state().map.map;
    assert_eq!(standing_on, Map::ViridianCity);
    let api = fixture.api();
    let mmu = api.mmu();

    assert!(map_header_is_loaded(mmu, standing_on), "the map the save is standing on");
    for other in [Map::PalletTown, Map::Route1, Map::ViridianMart, Map::SafariZoneCenter] {
        assert!(!map_header_is_loaded(mmu, other),
            "{other:?} is not loaded and the comparison has to say so");
    }
    drop(api);

    // And the field that had to come out of the comparison.
    let mut shopping = TestFixture::new(
        include_bytes!("../data/viridian-city-pokemart-shopping.bin"),
        Duration::from_secs(10),
        vec![],
    );
    assert_eq!(shopping.game_state().map.map, Map::ViridianMart);
    let api = shopping.api();
    assert!(map_header_is_loaded(api.mmu(), Map::ViridianMart),
        "a shop whose script has moved its own text pointer is still a loaded map");
}

/// Every `TestFixture` driver plays at the cartridge's fastest options.
#[test]
fn every_fixture_plays_at_the_fastest_game_options() {
    use crate::pokemon::options::{BattleStyle, GameOptionsReader, TextSpeed};
    use crate::pokemon::options::HEADLESS_OPTIONS;

    assert_eq!(HEADLESS_OPTIONS.text_speed, TextSpeed::Fast);
    assert!(!HEADLESS_OPTIONS.battle_animations_on);
    assert_eq!(HEADLESS_OPTIONS.battle_style, BattleStyle::Set, "SET is the no-switch-prompt one");

    // …and it is live in RAM the moment a fixture exists, before a single tick is run.
    let mut fixture = TestFixture::new(ROUTE1_STATE, Duration::from_secs(10), vec![]);
    let live = PokemonApi::new(&mut fixture.gb).mmu().read_game_options().expect("readable");
    assert_eq!(live, HEADLESS_OPTIONS, "a fresh fixture is already at the fast options");

    // …and it survives the game writing its own back, which is what the per-tick re-apply is for.
    {
        use crate::pokemon::options::{GameOptions, GameOptionsWriter};
        let slow = GameOptions {
            battle_animations_on: true,
            battle_style: BattleStyle::Shift,
            text_speed: TextSpeed::Slow,
        };
        PokemonApi::new(&mut fixture.gb).mmu_mut().write_game_options(&slow).expect("writable");
    }
    fixture.step();
    let live = PokemonApi::new(&mut fixture.gb).mmu().read_game_options().expect("readable");
    assert_eq!(live, HEADLESS_OPTIONS, "a tick puts the fast options back");
}

/// Every committed fixture's sprite table is complete, as `map_sprites_are_loaded` checks per tick.
#[test]
fn every_committed_fixture_has_a_complete_sprite_table() {
    use crate::pokemon::map_metadata::map_sprites_are_loaded;

    let mut files: Vec<_> = std::fs::read_dir("src/pokemon/data").expect("the fixture directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .collect();
    files.sort();
    assert!(files.len() > 100, "expected the whole fixture chain, found {}", files.len());

    let mut checked = 0;
    for path in files {
        // A never-regenerated fixture is a zero-byte placeholder.
        let bytes = std::fs::read(&path).expect("readable");
        if bytes.is_empty() { continue }
        let mut fixture = TestFixture::new(&bytes, Duration::from_secs(10), vec![]);
        let api = fixture.api();
        assert!(map_sprites_are_loaded(api.mmu()),
            "{}: the sprite table disagrees with wNumSprites, so every tick of this state would \
             read as a map still loading and be offered no actions at all", path.display());
        checked += 1;
    }
    println!("{checked} fixtures, every sprite table complete");
}

/// A Seafoam staircase whose warp the map script cancels is not offered as a row.
#[test]
fn a_seafoam_staircase_the_script_cancels_is_not_a_row() {
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection,
                                       script_cancelled_warps};
    use crate::pokemon::symbols::pokered_symbols;
    use crate::pokemon::tile_map::WarpTrigger;
    use gb::ram::RAM;
    use std::sync::Arc;

    // EVENT_SEAFOAM3_BOULDER1/2_DOWN_HOLE = $9C8/$9C9 → byte 313, bits 0 and 1;
    // EVENT_SEAFOAM4_BOULDER1/2_DOWN_HOLE = $9D0/$9D1 → byte 314, bits 0 and 1.
    const SEAFOAM3: (u16, u8) = (313, 0b11);
    const SEAFOAM4: (u16, u8) = (314, 0b11);

    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-articuno.bin"), Duration::from_secs(10), vec![]);
    let base = pokered_symbols::wEventFlags.address;

    // The arithmetic is the risky half and this is what checks it.
    let flags = |f: &TestFixture, off: u16| f.gb.core().mmu().read(base + off);
    assert_eq!(flags(&fixture, SEAFOAM4.0) & SEAFOAM4.1, SEAFOAM4.1,
        "post-articuno pushed B3F's two boulders down their holes, so SEAFOAM4 must be set — if it \
         is not, byte {} is not where SEAFOAM4 lives", SEAFOAM4.0);
    assert_eq!(flags(&fixture, SEAFOAM3.0) & SEAFOAM3.1, 0,
        "post-articuno leaves by Escape Rope *because* SEAFOAM3 is unset (policy.rs, the Articuno \
         leg), so byte {} must be clear", SEAFOAM3.0);

    let staircases = [Point8 { x: 20, y: 17 }, Point8 { x: 21, y: 17 }];
    let b4f = |f: &TestFixture| {
        let mmu = f.gb.core().mmu();
        MetaTileMap::new(&CurrentMap {
            player_position: Point8 { x: 20, y: 15 },
            player_direction: PlayerFacingDirection::Down,
            sprites: Vec::new(),
            metadata: Arc::new(mmu.read_map_metadata(Map::SeafoamIslandsB4F).unwrap()),
            closed_doors: Vec::new(),
            grass_encounter_rate: 0,
            water_encounter_rate: 0,
            card_key_locked: false,
            header_loaded: true,
            surfing: false,
            sprites_loaded: true,
            script_cancelled_warps: script_cancelled_warps(mmu, Map::SeafoamIslandsB4F),
            standing_on_warp: true,
        })
    };

    // Shut, on a save that has finished the islands: the two staircases are not rows.
    let shut = b4f(&fixture);
    for at in staircases {
        assert_eq!(shut.warp_trigger(at), WarpTrigger::Impossible,
            "({}, {}) is cancelled by the map script while SEAFOAM3 is clear", at.x, at.y);
    }
    assert!(!shut.actions().iter().any(|a| staircases.contains(&a.destination)),
        "a warp the script cancels must not be a row either");

    // And set: the script returns early, the warp is an ordinary one again, and the row is back.
    let held = flags(&fixture, SEAFOAM3.0);
    fixture.gb.core_mut().mmu_mut().write(base + SEAFOAM3.0, held | SEAFOAM3.1);
    let open = b4f(&fixture);
    for at in staircases {
        assert_ne!(open.warp_trigger(at), WarpTrigger::Impossible,
            "({}, {}) must come back the moment both boulders are down", at.x, at.y);
    }
}

#[test]
fn a_warp_reached_by_surfing_is_entered_rather_than_leant_on() {
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use crate::pokemon::tile_map::WarpTrigger;
    use std::sync::Arc;

    let mmu = gb::mmu::MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    for (map, to_map) in [
        (Map::SeafoamIslandsB3F, Map::SeafoamIslandsB4F),
        (Map::SeafoamIslandsB4F, Map::SeafoamIslandsB3F),
    ] {
        let metadata = Arc::new(mmu.read_map_metadata(map).unwrap());
        let on_the_warp = Point8 { x: 21, y: 17 };
        let build = |surfing: bool| {
            let mut tm = MetaTileMap::new(&CurrentMap {
                player_position: on_the_warp,
                player_direction: PlayerFacingDirection::Down,
                sprites: Vec::new(),
                metadata: Arc::clone(&metadata),
                closed_doors: Vec::new(),
                grass_encounter_rate: 0,
                water_encounter_rate: 0,
                card_key_locked: false,
                header_loaded: true,
                surfing,
                sprites_loaded: true,
                script_cancelled_warps: Vec::new(),
                standing_on_warp: true,
            });
            tm.can_surf = true;
            tm
        };

        // The entry is the bottom row of the map and water, so only a surfing player stands on it.
        let afloat = build(true);
        assert_eq!(afloat.warp_trigger(on_the_warp), WarpTrigger::HoldDirection(JoypadButton::Down),
            "{map:?} (21, 17) is the map-edge kind");
        assert_eq!(afloat.tile_at(on_the_warp),
            MetaTile::Warp { to_map, to_position: on_the_warp });

        let id = format!("{map:?}:21,17:Warp");
        let route = |tm: &MetaTileMap| -> Vec<JoypadButton> {
            tm.actions().into_iter().find(|a| a.id() == id)
                .unwrap_or_else(|| panic!("{id} should be a row"))
                .route
        };

        assert_eq!(route(&afloat), vec![JoypadButton::Up, JoypadButton::Down],
            "{id}: off the way it came and back the way `IsPlayerFacingEdgeOfMap` wants");

        assert_eq!(route(&build(false)), vec![JoypadButton::Down],
            "{id}: on foot the collision path fires it and no step is needed");
    }
}

#[test]
fn an_impossible_warp_is_one_the_cartridge_really_will_not_open() {
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use crate::pokemon::tile::MetaTile;
    use crate::pokemon::tile_map::WarpTrigger;
    use std::sync::Arc;
    use strum::IntoEnumIterator;

    const KNOWN: &[(Map, u8, u8, &str)] = &[
        (Map::Route7, 19, 9, "Route 7's gate: raw $23, and (19, 10) beside it is the door"),
        (Map::Route8, 2, 9, "Route 8's west gate: raw $39, sibling at (2, 10)"),
        (Map::Route8, 9, 9, "Route 8's east gate: raw $2c, sibling at (9, 10)"),
        (Map::SilphCo1F, 16, 10, "`warp_event 16, 10, SILPH_CO_3F, 7 ; inaccessible` — plain floor"),
    ];

    let mmu = gb::mmu::MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    let mut found: Vec<(Map, u8, u8)> = vec![];
    for map in Map::iter() {
        let Ok(metadata) = mmu.read_map_metadata(map) else { continue };
        let tile_map = MetaTileMap::new(&CurrentMap {
            player_position: Point8 { x: 0, y: 0 },
            player_direction: PlayerFacingDirection::Down,
            sprites: Vec::new(),
            metadata: Arc::new(metadata),
            closed_doors: Vec::new(),
            grass_encounter_rate: 0,
            water_encounter_rate: 0,
            card_key_locked: false,
            header_loaded: true,
            surfing: false,
            sprites_loaded: true,
            script_cancelled_warps: Vec::new(),
            standing_on_warp: true,
        });
        for (i, tile) in tile_map.meta_tiles.iter().enumerate() {
            if !matches!(tile, MetaTile::Warp { .. }) { continue }
            let at = Point8 { x: (i % tile_map.width) as u8, y: (i / tile_map.width) as u8 };
            if tile_map.warp_trigger(at) == WarpTrigger::Impossible {
                found.push((map, at.x, at.y));
            }
        }
    }

    let want: Vec<(Map, u8, u8)> = KNOWN.iter().map(|&(m, x, y, _)| (m, x, y)).collect();
    assert_eq!(found, want,
        "the set of warps the cartridge will not open has changed, and `actions()` drops every one \
         of them.\n  known:\n{}",
        KNOWN.iter().map(|(m, x, y, why)| format!("    {m:?} ({x}, {y}) — {why}\n"))
            .collect::<String>());
}

#[test]
fn a_door_with_somebody_standing_in_it_is_not_a_row_until_they_move() {
    use gb::geometry::Point8;
    const OCCUPIED: Point8 = Point8 { x: 3, y: 7 };
    const BESIDE_IT: Point8 = Point8 { x: 4, y: 7 };

    let mut fixture = TestFixture::new(
        include_bytes!("../data/cerulean-mart-shopper-in-the-doorway.bin"),
        Duration::from_mins(2),
        vec![PolicyStep::EnterMap { to_map: Map::CeruleanCity, to_position: None }]);
    let start = fixture.game_state();
    assert_eq!(start.map.map, Map::CeruleanMart);
    assert_eq!(start.map.tile_at(OCCUPIED), MetaTile::Sprite("Cooltrainer Male"),
        "the state has to be dropped on a tick with somebody actually in the doorway, or it proves \
         nothing");

    let rows: Vec<Point8> = start.map.actions().iter().map(|action| action.destination).collect();
    assert!(!rows.contains(&OCCUPIED),
        "the occupied half of the doormat is a person, not a door: {rows:?}");
    assert!(rows.contains(&BESIDE_IT),
        "the free half of the same doormat is still the way out: {rows:?}");

    let end = fixture.run_until(|state| state.map.map == Map::CeruleanCity);
    println!("left through ({}, {})", end.map.player_position.x, end.map.player_position.y);
}

/// A pacing pair is chosen once and the map moves under it.
#[test]
fn a_pacing_pair_somebody_steps_onto_is_re_picked_rather_than_bumped_into() {
    use gb::geometry::Point8;
    use crate::pokemon::agent::AgentState;
    const PLAYER: Point8 = Point8 { x: 14, y: 6 };
    const BLOCKED: Point8 = Point8 { x: 14, y: 5 };

    let mut fixture = TestFixture::new(
        include_bytes!("../data/route-11-youngster-on-the-pacing-tile.bin"),
        Duration::from_mins(3),
        vec![]);
    let start = fixture.game_state();
    assert_eq!(start.map.map, Map::Route11);
    assert_eq!(start.map.player_position, PLAYER);
    assert_eq!(start.map.tile_at(BLOCKED), MetaTile::Sprite("Youngster 1"),
        "the state has to be dropped with somebody on the square the pace walks into");

    // The pair is installed rather than asked for, and it has to be.
    fixture.agent.set_state(AgentState::PacingForEncounters {
        destination: MetaTile::Grass,
        map: Map::Route11,
        tile_a: PLAYER,
        tile_b: BLOCKED,
        heading_to_b: true,
        stalled: 0,
        paced: 0,
    });

    // Bumping for `STALL_TICKS` from the starting square ends in `Unknown`.
    let mut stalled = false;
    let mut paced = false;
    for _ in 0..9000 {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::OverworldActionAborted {
                reason: OverworldActionAbortedReason::Unknown, .. } = &event {
                stalled = true;
            }
        }
        let now = fixture.game_state();
        if now.mode == GameMode::WildBattle
            || (now.map.map == Map::Route11 && now.map.player_position != PLAYER) {
            paced = true;
        }
        if paced || stalled { break }
    }
    assert!(!stalled, "the pace reported a malfunction instead of pacing somewhere else");
    assert!(paced, "the pace neither moved nor turned anything up");
}

#[test]
fn a_water_crossing_is_a_row_of_its_own_beside_the_bridge_to_the_same_map() {
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use crate::pokemon::tile::MetaTile;
    use std::sync::Arc;

    let mmu = gb::mmu::MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    let metadata = Arc::new(mmu.read_map_metadata(Map::Route24).expect("Route 24's header"));
    let route_24 = |can_surf: bool| {
        let mut map = MetaTileMap::new(&CurrentMap {
            // The south end of Route 24, where the footbridge and the river seam are neighbours.
            player_position: Point8 { x: 6, y: 30 },
            player_direction: PlayerFacingDirection::Down,
            sprites: Vec::new(),
            metadata: Arc::clone(&metadata),
            closed_doors: Vec::new(),
            grass_encounter_rate: 0,
            water_encounter_rate: 0,
            card_key_locked: false,
            header_loaded: true,
            surfing: false,
            sprites_loaded: true,
            script_cancelled_warps: Vec::new(),
            standing_on_warp: false,
        });
        // `game_state()` sets `can_surf` from the party; the map builder does not.
        map.can_surf = can_surf;
        map.actions().into_iter().map(|action| action.tile).collect::<Vec<_>>()
    };

    let on_foot = route_24(false);
    assert!(on_foot.iter().any(|t| matches!(t, MetaTile::Connection { to_map: Map::CeruleanCity, .. })),
        "the footbridge into Cerulean is a row whatever the party knows: {on_foot:?}");
    assert!(!on_foot.iter().any(|t| matches!(t, MetaTile::ConnectionWater(Map::CeruleanCity))),
        "a water edge nothing can mount is scenery, not a way out: {on_foot:?}");

    let surfing = route_24(true);
    assert!(surfing.iter().any(|t| matches!(t, MetaTile::Connection { to_map: Map::CeruleanCity, .. })),
        "the bridge does not go away when the party can Surf: {surfing:?}");
    assert!(surfing.iter().any(|t| matches!(t, MetaTile::ConnectionWater(Map::CeruleanCity))),
        "the river seam is the only way to the Cerulean Cave side, and it has to be its own row: \
         {surfing:?}");
}

#[test]
fn a_duplicate_map_is_not_a_coverage_gap() {
    use crate::pokemon::map_metadata::MapMetadataReader;
    use strum::IntoEnumIterator;

    let mmu = gb::mmu::MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    let mut targeted: std::collections::BTreeSet<Map> = Default::default();
    let mut readable: Vec<Map> = vec![];
    for map in Map::iter() {
        let Ok(metadata) = mmu.read_map_metadata(map) else { continue };
        readable.push(map);
        for warp in &metadata.warp_events {
            targeted.insert(warp.destination_map);
        }
    }

    const DUPLICATES: [Map; 4] = [
        Map::CeruleanTrashedHouseCopy,
        Map::CinnabarMartCopy,
        Map::UndergroundPathRoute6Copy,
        Map::UndergroundPathRoute7Copy,
    ];
    for map in DUPLICATES {
        assert!(!targeted.contains(&map),
            "{map:?} is warped to after all, so it is not an unreachable duplicate");
    }

    // And nothing else that the reader can see is orphaned.
    let orphans: Vec<Map> = readable.into_iter()
        .filter(|map| !targeted.contains(map))
        .filter(|map| !format!("{map:?}").starts_with("UnusedMap"))
        .filter(|map| !matches!(map, Map::Colosseum | Map::TradeCenter))
        .filter(|map| !map.is_overworld())
        .collect();
    println!("interiors no warp anywhere targets: {orphans:?}");
    assert!(orphans.iter().all(|map| DUPLICATES.contains(map)),
        "an interior nothing warps to that is not one of the known duplicates: {orphans:?} — \
         coverage::UNREACHABLE_DUPLICATES and the coverage report's map counts are derived from \
         that list");
}
