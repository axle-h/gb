//! Stalls the fuzzer found, each frozen into a save state and re-run in a second.

use super::*;
use crate::pokemon::policy::RandomPolicy;

/// Game time each case is given to reach a decision point.
const ESCAPE_BUDGET: Duration = Duration::from_secs(120);

/// The longest silence a case may show before it counts as still stuck.
const QUIET_LIMIT: Duration = Duration::from_secs(90);

/// Replay `state` against a fresh agent and return the longest it went without reaching a
/// decision point, plus where it ended up.
fn longest_silence(state: &[u8], seed: u64) -> (Duration, String, String) {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).expect("a committed stall fixture should load");
    let mut cache = MapMetadataCache::default();
    let mut agent = PokemonAgent::new(Box::new(RandomPolicy::seeded(seed)));

    let budget = MachineCycles::from_duration(ESCAPE_BUDGET);
    let mut emulated = MachineCycles::ZERO;
    let mut worst = Duration::ZERO;
    let mut worst_state = String::new();

    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        agent.drain_events();

        let gap = agent.since_last_policy_poll();
        if gap > worst {
            worst = gap;
            worst_state = agent.state_debug();
        }
    }

    let where_it_is = PokemonApi::with_cache(&mut gb, &mut cache)
        .game_state()
        .map_or_else(|_| "unreadable".into(), |s| format!("{} at {}", s.map.map, s.map.player_position));
    (worst, worst_state, where_it_is)
}

/// Assert a fixture is no longer a stall, reporting what it did if it still is.
fn assert_escapes(name: &str, state: &[u8]) {
    // Three seeds, because escaping must not depend on what the policy picks once it is free.
    for seed in [1, 2, 3] {
        let (worst, worst_state, where_it_is) = longest_silence(state, seed);
        assert!(
            worst < QUIET_LIMIT,
            "{name} (seed {seed}): the agent went {worst:?} of game time without reaching a decision \
             point — still stuck.\n  state: {worst_state}\n  where: {where_it_is}",
        );
        println!("[stall] {name} (seed {seed}): out in {worst:?}, {where_it_is}");
    }
}

/// `soak` seed 1, 3600 s in — a Bulbasaur out of PP against a Weedle in Viridian Forest.
#[test]
fn a_move_with_no_pp_left_does_not_trap_the_battle() {
    assert_escapes("no-pp-move", include_bytes!("../data/stall-no-pp-move.bin"));
}

/// `soak` seed 1, `at-vermilion`, 372 s in — the S.S.
#[test]
fn a_key_item_used_in_battle_does_not_trap_the_bag() {
    assert_escapes("battle-key-item", include_bytes!("../data/stall-battle-key-item.bin"));
}

/// `can_get_rainbow_badge`, Erika's Vileplume — a party with no PP anywhere.
#[test]
fn a_party_with_no_pp_anywhere_still_gets_an_answer() {
    use crate::pokemon::policy::DeterministicPolicy;
    let state = include_bytes!("../data/stall-no-pp-trainer-battle.bin");
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).expect("a committed stall fixture should load");
    let mut cache = MapMetadataCache::default();
    // An empty queue: `pick_battle_action` does not read it, and what is under test is the answer
    // it gives when the active mon has nothing left, not the route it is on.
    let mut agent = PokemonAgent::new(Box::new(DeterministicPolicy::new(1, [])));

    // Count *actions*, not silence, because the watchdog cannot see this one.
    let budget = MachineCycles::from_duration(ESCAPE_BUDGET);
    let mut emulated = MachineCycles::ZERO;
    let mut actions = 0usize;
    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        actions += agent.drain_events().iter()
            .filter(|e| matches!(e, AgentEvent::BattleActionStarted { .. })).count();
    }
    assert!(actions > 0,
        "the scripted policy took no battle action in {ESCAPE_BUDGET:?} of game time against a fight \
         it cannot win and cannot leave — it is waiting at the menu for ever.\n  state: {}",
        agent.state_debug());
    println!("[stall] no-pp-trainer-battle: {actions} battle actions taken");
}

/// `soak` seed 1, `postgame-pc-box`, 554 s in — the Lift Key against a Bug Catcher's Weedle.
#[test]
fn a_key_item_used_against_a_trainer_does_not_trap_the_bag() {
    assert_escapes("battle-key-item-trainer", include_bytes!("../data/stall-battle-key-item-trainer.bin"));
}

/// `soak` seed 1, `at-vermilion`, 1681 s in — the man in the Cerulean badge house.
#[test]
fn a_list_that_only_b_leaves_does_not_trap_a_conversation() {
    assert_escapes("badge-house-list", include_bytes!("../data/stall-badge-house-list.bin"));
}

/// `soak` seed 1, `at-cinnabar`, 2258 s in — SURF chosen in Saffron City.
#[test]
fn a_field_move_the_game_refuses_does_not_trap_the_party_menu() {
    assert_escapes("field-move-refused", include_bytes!("../data/stall-field-move-refused.bin"));
}

/// `soak` seeds 28/38/50/62, 1801 s in — a fainted Pokémon chosen from the battle party menu.
#[test]
fn a_fainted_pokemon_chosen_in_battle_does_not_trap_the_party_menu() {
    assert_escapes("fainted-switch", include_bytes!("../data/stall-fainted-switch.bin"));
}

/// `soak` seeds 8/30/36/44, 1082 s in — a Card Key door on Silph Co 2F, with no Card Key.
#[test]
fn a_card_key_door_that_will_not_open_is_only_tried_once() {
    assert_escapes("card-key-door", include_bytes!("../data/stall-card-key-door.bin"));
}

/// `soak` seed 119, `postgame-aides`, 223 s in — a Ditto on Route 15, one frame, for ever.
#[test]
fn a_battle_message_over_the_party_list_is_cleared_first() {
    assert_escapes("battle-message-over-party", include_bytes!("../data/stall-battle-message-over-party.bin"));
}

/// `soak` seed 11, `at-cinnabar`, 2301 s in — the water current on Seafoam Islands B4F.
#[test]
fn a_walk_the_current_keeps_interrupting_gives_up() {
    assert_escapes("seafoam-current", include_bytes!("../data/stall-seafoam-current.bin"));
}

/// `soak` seeds 76…120, eleven of them — Bill's own PC, in his house on Route 25.
#[test]
fn a_menu_offering_cancel_does_not_trap_a_conversation() {
    assert_escapes("bills-pc-list", include_bytes!("../data/stall-bills-pc-list.bin"));
}

/// `soak` seed 70, 1500 s in — BAIT thrown at the same Rhyhorn for ever.
#[test]
fn a_safari_menu_cursor_left_on_bait_does_not_repeat_itself() {
    assert_escapes("safari-menu", include_bytes!("../data/stall-safari-menu.bin"));
}

#[test]
fn a_ghost_battle_is_left_rather_than_fought_for_ever() {
    use crate::pokemon::policy::DeterministicPolicy;
    let state = include_bytes!("../data/stall-ghost-battle.bin");
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).expect("a committed stall fixture should load");
    let mut cache = MapMetadataCache::default();
    // An empty queue, as in the no-PP case: what is under test is the answer `pick_battle_action`
    // gives to a fight that cannot be won, not the route it happens to be on.
    let mut agent = PokemonAgent::new(Box::new(DeterministicPolicy::new(1, [])));

    let budget = MachineCycles::from_duration(ESCAPE_BUDGET);
    let mut emulated = MachineCycles::ZERO;
    let mut left_at = None;
    let mut turns = 0usize;
    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        turns += agent.drain_events().iter()
            .filter(|e| matches!(e, AgentEvent::BattleActionStarted { .. })).count();
        if api.game_state().is_ok_and(|s| s.battle.is_none()) {
            left_at = Some(emulated.to_duration());
            break;
        }
    }

    let left_at = left_at.unwrap_or_else(|| panic!(
        "still in the ghost battle after {ESCAPE_BUDGET:?} of game time and {turns} battle actions \
         — every one of them a move the cartridge refuses to execute.\n  state: {}",
        agent.state_debug()));
    println!("[stall] ghost-battle: out in {left_at:?} after {turns} battle actions");
}

/// The Route 8 gate doorstep, and it is a stall rather than a wasted turn.
#[test]
fn the_route_8_gate_can_be_re_entered_from_its_own_doorstep() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-lavender.bin"),
        Duration::from_secs(600),
        vec![
            PolicyStep::enter(Map::Route8),
            PolicyStep::enter(Map::Route8Gate),
            PolicyStep::enter(Map::Route8),
            PolicyStep::enter(Map::Route8Gate),
        ],
    );
    fixture.try_run_until(|state| state.map.map == Map::Route8Gate).expect("into the gate");
    // Wait for the landing square, not merely for the map.
    let out = fixture.try_run_until(|state| state.map.map == Map::Route8 && matches!(
        (state.map.player_position.x, state.map.player_position.y), (2 | 9, 9 | 10)))
        .expect("out of the gate again");
    // And assert *which* door, or the test passes on a run that left by the west one and never
    // stood on the square this is about.
    assert_eq!(out.map.player_position.x, 9,
               "the east door puts the player on the entry the cartridge will not open");

    fixture.try_run_until(|state| state.map.map == Map::Route8Gate)
        .expect("and straight back in from the doorstep, which is what never used to happen");
}

/// Route 14's north-east pocket, walled in by a trainer, whose only way out is back over the
/// Route 13 border. From where that crossing lands, the nearest way into Route 14 is the pocket's
/// own, and a route on to Route 15 has to take another one.
#[test]
fn a_route_out_of_a_pocket_does_not_walk_back_into_it() {
    // Raw landings on Route 14, and the pocket's in the tile map's own coordinates.
    const MAIN: Point8 = Point8 { x: 19, y: 8 };
    const POCKET: Point8 = Point8 { x: 19, y: 6 };
    const IN_THE_POCKET: Point8 = Point8 { x: 20, y: 6 };
    let mut fixture = TestFixture::new(
        include_bytes!("../data/pocket-route14.bin"),
        Duration::from_secs(300),
        vec![
            // Both sections first, so the graph has seen the pocket and the way on.
            PolicyStep::enter(Map::Route13),
            PolicyStep::EnterMap { to_map: Map::Route14, to_position: Some(MAIN) },
            PolicyStep::enter(Map::Route13),
            PolicyStep::EnterMap { to_map: Map::Route14, to_position: Some(POCKET) },
            PolicyStep::Goto { map: Map::Route15, strict: true },
        ],
    );
    for map in [Map::Route13, Map::Route14, Map::Route13] {
        fixture.try_run_until(|state| state.map.map == map).unwrap_or_else(|| panic!("onto {map}"));
    }
    let pocket = fixture.try_run_until(|state| state.map.map == Map::Route14 && state.map.position_settled)
        .expect("back into the pocket");
    assert_eq!(pocket.map.player_position, IN_THE_POCKET, "the fixture no longer stands where this is about");

    // Out, across the border lower down, and on: three crossings.
    let crossings = std::cell::Cell::new(0);
    let last = std::cell::Cell::new(Map::Route14);
    let end = fixture.try_run_until(|state| {
        if state.map.map != last.get() {
            last.set(state.map.map);
            crossings.set(crossings.get() + 1);
        }
        state.map.map == Map::Route15 || crossings.get() > 6
    });
    assert_eq!(end.map(|state| state.map.map), Some(Map::Route15),
               "never reached Route 15; {} crossings, walking back into the pocket", crossings.get());
    assert_eq!(crossings.get(), 3);
}

#[test]
fn a_water_route_does_not_climb_out_onto_route_21s_islands() {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../data/route21-islands.bin")).expect("the Route 21 fixture loads");
    let mut cache = MapMetadataCache::default();
    let map = PokemonApi::with_cache(&mut gb, &mut cache).game_state().expect("a map to route on").map;
    assert_eq!(map.map, Map::Route21);
    assert!(map.can_surf, "the fixture is mid-crossing, so the search must believe it can surf");

    let north = map.water_connection_action(Map::PalletTown)
        .expect("the way back to Pallet Town is a water crossing and it is reachable");
    // Walk the route and collect every square it puts the player on.
    let mut at = map.player_position;
    let ashore: Vec<(Point8, Option<MetaTile>)> = north.route[..north.route.len() - 1].iter()
        .map(|b| {
            at = match b {
                JoypadButton::Up    => Point8 { x: at.x, y: at.y - 1 },
                JoypadButton::Down  => Point8 { x: at.x, y: at.y + 1 },
                JoypadButton::Left  => Point8 { x: at.x - 1, y: at.y },
                _                   => Point8 { x: at.x + 1, y: at.y },
            };
            (at, map.tile_at_checked(at))
        })
        .filter(|(_, t)| !matches!(t, Some(MetaTile::Water) | Some(MetaTile::ConnectionWater(_))))
        .collect();
    assert!(ashore.is_empty(),
            "the crossing steps ashore at {ashore:?}, which is a Surf mount apiece to leave again — \
             the islands at y = 25/26 are what the search's mount price is for");
}

/// The other half of the Route 21 crossing: a mount must not end the walk it is part of.
#[test]
fn a_surf_mount_hands_the_walk_back_to_itself_rather_than_to_the_policy() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/postgame-fishing.bin"),
        Duration::from_secs(300),
        vec![PolicyStep::enter(Map::Route21)],
    );
    assert_eq!(fixture.game_state().map.map, Map::PalletTown);

    // The distinct agent states from the decision to the landing, in order.
    let mut seen: Vec<String> = vec![];
    let mut arrived = false;
    for _ in 0..12_000 {
        fixture.step();
        let state = fixture.agent.state_debug();
        if seen.last() != Some(&state) { seen.push(state); }
        if fixture.try_game_state().map(|g| g.map.map) == Ok(Map::Route21) { arrived = true; break }
    }
    assert!(arrived, "the crossing has to finish, or the sequence below is about nothing: {seen:?}");
    assert!(seen.iter().any(|s| s == "surf"),
            "the walk had to mount Surf to leave Pallet Town, or this proves nothing: {seen:?}");
    assert_eq!(seen.first().map(String::as_str), Some("wait"),
               "the crossing opens on the one decision that starts it: {seen:?}");
    assert!(!seen[1..].iter().any(|s| s.starts_with("wait")),
            "the whole crossing is one decision: a second `wait` is the walk being thrown away and \
             the identical question put back to the policy, which is a paid request. {seen:?}");
}

/// Cinnabar Island's gym doorstep, and the general rule it is the test case for.
#[test]
fn a_square_the_game_walks_you_back_off_is_learned_and_routed_around() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-cinnabar.bin"),
        Duration::from_secs(300),
        vec![PolicyStep::enter(Map::CinnabarPokecenter),
             PolicyStep::enter(Map::CinnabarIsland),
             PolicyStep::enter(Map::PokemonMansion1F)],
    );
    // The fixture is before the Mansion, so the key cannot be in the bag — if it ever is, the gym
    // doorstep is ordinary floor and this test is about nothing.
    assert!(!fixture.game_state().bag.iter()
                .any(|i| i.id == crate::pokemon::item::ItemId::SecretKey),
            "the Secret Key is inside the Mansion, so a fixture standing outside it must not hold one");

    let mut learned = vec![];
    for _ in 0..40_000 {
        fixture.step();
        learned.extend(fixture.agent.drain_events().into_iter().map(|e| format!("{e}"))
                           .filter(|e| e.contains("walked you back")));
        if fixture.agent.policy_steps_remaining() == Some(0) { break }
    }
    assert_eq!(fixture.try_game_state().map(|g| g.map.map), Ok(Map::PokemonMansion1F),
               "the walk to the Mansion has to arrive; over the doorstep it is stopped by \
                \"The door is locked...\" and re-planned identically for ever");
    // Arrival alone would also pass if the router simply never chose that square — which is a
    // tie-break away from being true again.
    assert!(learned.iter().any(|e| e.contains("(18, 5)")),
            "the doorstep has to be recognised from the shove rather than avoided by luck: {learned:?}");
}

/// The mount must not be handed to `RunningScript`, and whether it *is* depends on the map's
/// NPCs.
#[test]
fn a_surf_mount_is_not_taken_over_by_the_script_handler() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-cinnabar.bin"),
        Duration::from_secs(300),
        // Cinnabar's east shore *is* the seam into Route 20, so the mount's own step crosses it —
        // there is no walk left to resume, which is the other half of why this is a separate
        // case.
        vec![PolicyStep::enter(Map::Route20)],
    );
    assert_eq!(fixture.game_state().map.map, Map::CinnabarIsland);

    let mut seen: Vec<String> = vec![];
    let mut arrived = false;
    for _ in 0..12_000 {
        fixture.step();
        let state = fixture.agent.state_debug();
        if seen.last() != Some(&state) { seen.push(state); }
        if fixture.try_game_state().map(|g| g.map.map) == Ok(Map::Route20) { arrived = true; break }
    }
    assert!(arrived, "the crossing has to finish: {seen:?}");
    let mount = seen.iter().position(|s| s == "surf")
        .unwrap_or_else(|| panic!("it has to mount Surf to get there: {seen:?}"));
    // From the mount onward, not from the decision.
    assert!(!seen[mount..].iter().any(|s| s == "script" || s == "text"),
            "the mount drives its own menus and ends in its own scripted step, so nothing between \
             it and the landing may be `script` or `text`: {seen:?}");
}
