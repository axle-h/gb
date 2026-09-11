use super::super::*;
use crate::pokemon::encoding::GameMode;

/// Drive `post-hall-of-fame.bin` until the player is in a playable overworld again, and return that
/// state.
pub fn drive_out_of_hall_of_fame(fixture: &mut TestFixture) -> GameState {
    let mut tick = 0u32;
    loop {
        let (mode, map) = {
            let api = fixture.api();
            (api.game_mode(), api.mmu().read_pointer(&pokered_symbols::wCurMap))
        };
        if mode == Some(GameMode::Overworld) && map != Map::HallOfFame as u8 {
            fixture.api().release_all_buttons();
            return fixture.game_state();
        }
        // Mash: press one tick, release the next, so every input is a fresh rising edge.
        {
            let mut api = fixture.api();
            if tick % 2 == 0 { api.press_button(JoypadButton::A); } else { api.release_all_buttons(); }
        }
        fixture.step();
        tick += 1;
    }
}

/// The end of the game is noticed, once, at the frame the ceremony starts.
#[test]
fn the_hall_of_fame_is_announced_once_when_the_ceremony_starts() {
    let mut fixture = TestFixture::new(
        include_bytes!("../../data/post-hall-of-fame.bin"),
        Duration::from_mins(20),
        vec![],
    );
    assert_eq!(
        fixture.api().mmu().read_pointer(&pokered_symbols::wNumHoFTeams), 0,
        "the fixture is captured on arrival, three script stages before AnimateHallOfFame",
    );

    // A-mash exactly as `drive_out_of_hall_of_fame` does, but stop at the announcement.
    let mut wins = Vec::new();
    let mut tick = 0u32;
    while wins.is_empty() {
        {
            let mut api = fixture.api();
            if tick % 2 == 0 { api.press_button(JoypadButton::A); } else { api.release_all_buttons(); }
        }
        fixture.step();
        tick += 1;
        wins.extend(
            fixture.agent.drain_events().into_iter()
                .filter(|event| matches!(event, AgentEvent::HallOfFame { .. })),
        );
        assert!(tick < 60_000, "the ceremony never started");
    }

    let AgentEvent::HallOfFame { teams, playtime, playtime_seconds, badges, party } = &wins[0] else {
        unreachable!("filtered above")
    };
    assert_eq!(*teams, 1, "a first championship");
    assert_eq!(*badges, 0xFF, "all eight badges, read at the moment of victory");
    assert!(!party.is_empty(), "the winning party is carried on the event, not looked up later");
    assert_eq!(
        *playtime_seconds,
        crate::pokemon::observe::playtime_seconds(&fixture.api()),
        "the two readings of the cartridge's clock agree",
    );
    assert!(playtime.len() == 8, "HH:MM:SS, got {playtime}");
    assert_eq!(
        fixture.api().mmu().read_pointer(&pokered_symbols::wCurMap), Map::HallOfFame as u8,
        "it fires at the start of the ceremony — the credits and the soft reset are still to come",
    );
    println!("the game was won at {playtime} after {tick} agent ticks in the ceremony");

    // Once, however long the ceremony runs.
    for _ in 0..2_000 {
        fixture.step();
        assert!(
            !fixture.agent.drain_events().iter().any(|e| matches!(e, AgentEvent::HallOfFame { .. })),
            "the counter is monotonic, so the edge happens exactly once",
        );
    }

    // And a fresh agent seeded from a state that has already won is silent.
    let won = fixture.gb.save_state().expect("a state past the increment");
    let mut resumed = TestFixture::new(&won, Duration::from_mins(1), vec![]);
    for _ in 0..500 {
        resumed.step();
        assert!(
            !resumed.agent.drain_events().iter().any(|e| matches!(e, AgentEvent::HallOfFame { .. })),
            "resuming a finished run must not re-announce a victory from another process",
        );
    }
}

/// Winning the game does not hand the world back, and the agent must stop playing it.
#[test]
fn the_agent_stops_playing_a_world_the_cartridge_has_reset() {
    // `RandomPolicy` answers every overworld turn, so a quiet tick is the agent declining to ask.
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../../data/post-hall-of-fame.bin"),
        Duration::from_mins(20),
        Box::new(crate::pokemon::policy::RandomPolicy::seeded(7)),
    );

    let (mut won, mut reset) = (false, false);
    let (mut before, mut after) = (0usize, 0usize);
    let mut tick = 0u32;
    // Until a real overworld on another map comes back.
    loop {
        let (mode, map) = {
            let api = fixture.api();
            (api.game_mode(), api.mmu().read_pointer(&pokered_symbols::wCurMap))
        };
        if won && mode == Some(GameMode::Overworld) && map != Map::HallOfFame as u8 { break }
        assert!(fixture.total_cycles.to_duration() < Duration::from_secs(600),
                "the ending never finished");
        {
            let mut api = fixture.api();
            if tick % 2 == 0 { api.press_button(JoypadButton::A); } else { api.release_all_buttons(); }
        }
        tick += 1;
        // The window opens at the announcement and closes once the cartridge has reset and a game
        // is loaded again.
        let loaded = fixture.api().a_game_is_loaded();
        if won && !loaded { reset = true }
        let in_the_window = won && !(reset && loaded);
        fixture.step();
        for event in fixture.agent.drain_events() {
            match event {
                AgentEvent::HallOfFame { .. } => won = true,
                AgentEvent::StartedOverworldAction { .. } if in_the_window => after += 1,
                AgentEvent::StartedOverworldAction { .. } => before += 1,
                _ => {}
            }
        }
    }

    assert!(won, "the ceremony never started, so this test proved nothing");
    // Without this the test passes on a run that sat in the Hall of Fame.
    assert!(reset, "the cartridge never reached `jp Init`, so the window under test never opened");
    assert!(before <= 2, "{before} walks before the announcement — the room is small");
    assert_eq!(after, 0, "the agent started {after} walks across a room the player had left");

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::PalletTown, "the reset lands outside the player's own front door");
    let mut played = 0usize;
    for _ in 0..600 {
        fixture.step();
        played += fixture.agent.drain_events().iter()
            .filter(|e| matches!(e, AgentEvent::StartedOverworldAction { .. })).count();
    }
    assert!(played > 0, "the agent never started playing again after the world came back");
}

/// Cuts `postgame-post-credits.bin` by walking out of the Hall of Fame with nothing lost.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_walk_out_of_the_hall_of_fame() {
    let mut fixture = TestFixture::new(
        include_bytes!("../../data/post-hall-of-fame.bin"),
        Duration::from_mins(20),
        vec![],
    );

    let state = drive_out_of_hall_of_fame(&mut fixture);
    println!("post-credits: {} @ {}", state.map.map, state.map.player_position);

    // The special warp lands the player outside their own front door in Pallet Town.
    assert_eq!(state.map.map, Map::PalletTown);
    assert_eq!(state.mode, GameMode::Overworld);
    // A save/reload, not a new game: nothing lost.
    assert_eq!(state.badges.bits(), 255, "badges lost across the reset");
    assert_eq!(state.pokemon.len(), 4, "party lost across the reset");
    assert!(state.money > 0, "money lost across the reset");

    fixture.save_state_named("src/pokemon/data/postgame-post-credits.bin").unwrap();
}

/// The options the harness writes survive the Hall of Fame's soft reset.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn options_survive_the_hall_of_fame_reset() {
    use crate::pokemon::postgame::debug::FAST_FIXTURE_OPTIONS;

    let mut fixture = TestFixture::new(
        include_bytes!("../../data/post-hall-of-fame.bin"),
        Duration::from_mins(20),
        vec![],
    );

    assert_eq!(fixture.api().read_game_options().unwrap(), FAST_FIXTURE_OPTIONS,
        "J2 should have applied the options at load");

    let state = drive_out_of_hall_of_fame(&mut fixture);
    assert_eq!(state.map.map, Map::PalletTown);

    assert!(fixture.options_drifts > 0,
        "the credits' save/soft-reset/CONTINUE should have restored the cartridge's own wOptions at \
         least once — if this ever reads 0, the per-tick re-apply in TestFixture::step is dead code \
         and J3's whole premise is wrong");
    assert_eq!(fixture.api().read_game_options().unwrap(), FAST_FIXTURE_OPTIONS,
        "…and the re-apply should have put them back");
    println!("wOptions drifted {} times across the credits and was re-applied each time",
        fixture.options_drifts);
}

/// The postgame root: Pallet Town after the credits, cut by `can_walk_out_of_the_hall_of_fame`.
const POST_CREDITS: &[u8] = include_bytes!("../../data/postgame-post-credits.bin");

/// Stand at a Pokémon Center PC and open it.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_open_the_pokemon_center_pc() {
    // Viridian is the nearest centre to where the credits drop the player.
    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(20), vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::UsePc { map: Map::ViridianPokecenter },
    ]);

    // `UsePc` pops once it issues the walk, so drain the queue and watch the screen.
    fixture.step_until_exhausted();

    // Log every distinct screen for the next 30 s of game time.
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..(30 * 50) {
        fixture.step();
        if let Some(text) = fixture.api().on_screen_text(false) {
            if seen.last() != Some(&text) && !text.is_empty() {
                println!("  screen: {text}");
                seen.push(text);
            }
        }
    }

    // The parent menu, post-Champion, in full.
    let main_menu = seen.iter()
        .filter(|t| t.contains("LOG OFF") && !t.contains("Acc"))
        .max_by_key(|t| t.len())
        .unwrap_or_else(|| panic!("PC main menu never appeared; screens seen: {seen:#?}"));
    println!("PC main menu: {main_menu}");
    for entry in ["BILL's PC", "PROF.OAK's PC", "LEAGUE", "LOG OFF"] {
        assert!(main_menu.contains(entry), "{entry:?} missing from PC menu {main_menu:?}");
    }

    assert!(
        !seen.iter().any(|t| t.contains("CHANGE BOX") && t.contains("SEE YA!")),
        "the agent walked into Bill's PC instead of logging off; screens seen: {seen:#?}"
    );
    assert!(!fixture.api().in_pc_menu(), "the agent never got back out of the PC");
    assert_eq!(fixture.game_state().map.map, Map::ViridianPokecenter,
               "it should have logged off and be standing in front of the PC again");
}

/// TM34 Bide, a spare TM that `item.rs` calls "the bag's most useless item".
const SPARE_TM: ItemId = ItemId::Tm34Bide;

/// Deposit an item into PC storage.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_deposit_an_item() {
    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(20), vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::deposit_item(SPARE_TM, 1, Map::ViridianPokecenter),
    ]);

    let before = bag_count(&mut fixture);
    assert_eq!(before, 20, "the postgame root should still have a full bag");
    assert_eq!(fixture.api().bag_item_quantity(SPARE_TM), 1);

    fixture.step_until_exhausted();
    // The step pops when the driver takes over, so wait on the effect.
    run_until_bag(&mut fixture, before - 1);

    let api = fixture.api();
    assert_eq!(api.bag_item_quantity(SPARE_TM), 0, "{SPARE_TM:?} should have left the bag");
    assert_eq!(api.pc_box_item_quantity(SPARE_TM), 1, "{SPARE_TM:?} should be in PC storage");
    println!("bag {before} → {}, PC storage now holds {SPARE_TM:?}", bag_count(&mut fixture));
}

/// An item deposited and withdrawn again.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn item_round_trips_through_pc_storage() {
    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(30), vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::deposit_item(SPARE_TM, 1, Map::ViridianPokecenter),
        PolicyStep::withdraw_item(SPARE_TM, 1, Map::ViridianPokecenter),
    ]);

    let before = bag_count(&mut fixture);
    fixture.step_until_exhausted();

    // Out…
    run_until_bag(&mut fixture, before - 1);
    assert_eq!(fixture.api().pc_box_item_quantity(SPARE_TM), 1, "should be banked");
    println!("deposited: bag {before} → {}", before - 1);

    // …and back.
    run_until_bag(&mut fixture, before);
    let api = fixture.api();
    assert_eq!(api.bag_item_quantity(SPARE_TM), 1, "{SPARE_TM:?} should be back in the bag");
    assert_eq!(api.pc_box_item_quantity(SPARE_TM), 0, "PC storage should be empty again");
    println!("withdrew: bag back to {before}");
}

/// The six spare TMs the save arrives carrying.
const SPARE_TMS: [ItemId; 6] = [
    ItemId::Tm06Toxic, ItemId::Tm11Bubblebeam, ItemId::Tm21MegaDrain,
    ItemId::Tm24Thunderbolt, ItemId::Tm27Fissure, ItemId::Tm34Bide,
];

/// Cuts `postgame-phase0.bin`: six spare TMs banked, the key items kept, the party healed.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_ship_the_phase0_entry_fixture() {
    let mut steps = vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
    ];
    steps.extend(SPARE_TMS.map(|tm| PolicyStep::deposit_item(tm, 1, Map::ViridianPokecenter)));
    // Heal last: the deposits leave the player standing at the PC, one tile from the nurse.
    steps.push(PolicyStep::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE));

    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(40), steps);
    let before = bag_count(&mut fixture);
    assert_eq!(before, 20);

    fixture.step_until_exhausted();
    run_until_bag(&mut fixture, before - SPARE_TMS.len() as u8);

    // `Interact` pops when it issues the walk, so gate on the party being at full health.
    let state = fixture.run_until(|s| {
        s.mode == GameMode::Overworld && s.pokemon.iter().all(|p| p.current_hp == p.stats.hp)
    });

    let api = fixture.api();
    for tm in SPARE_TMS {
        assert_eq!(api.bag_item_quantity(tm), 0, "{tm:?} should be banked");
        assert_eq!(api.pc_box_item_quantity(tm), 1, "{tm:?} should be in PC storage");
    }
    for keep in [ItemId::Hm01Cut, ItemId::Hm03Surf, ItemId::Hm04Strength, ItemId::PokeFlute,
                 ItemId::SilphScope, ItemId::CardKey, ItemId::SecretKey, ItemId::HelixFossil,
                 ItemId::TownMap, ItemId::SSTicket, ItemId::LiftKey] {
        assert!(api.bag_item_quantity(keep) > 0, "{keep:?} must stay in the bag");
    }
    assert_eq!(state.badges.bits(), 255);

    let after = bag_count(&mut fixture);
    println!("phase 0 entry fixture: bag {before} → {after}, party healed, at {} @ {}",
        state.map.map, state.map.player_position);
    assert!(after <= 14, "bag should be well under 20, is {after}");

    fixture.save_state_named("src/pokemon/data/postgame-phase0.bin").unwrap();
}

/// Raw `wNumBagItems`, since `GameState::bag` drops ids `ItemId` cannot name and under-reports
/// against the 20-slot ceiling.
fn bag_count(fixture: &mut TestFixture) -> u8 {
    fixture.api().mmu().read_pointer(&pokered_symbols::wNumBagItems)
}

/// Drive until the bag holds exactly `target` items.
fn run_until_bag(fixture: &mut TestFixture, target: u8) {
    while bag_count(fixture) != target {
        fixture.step();
    }
}
