//! What a model can do that it could not before, each through `LlmPolicy`, the worker and the wire.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::pokemon::integration_tests::llm_harness::{Call, LlmRun, Reply, TurnRequest};

/// How long a test waits on the wall clock; a deadline, so a passing run spends none of it.
const PATIENCE: Duration = Duration::from_secs(120);

/// What a brain has been shown, in order.
#[derive(Default)]
struct Seen {
    overworld_turns: Vec<String>,
    /// Every distinct `tool`-role message the endpoint has been sent.
    tool_results: Vec<String>,
    battle_turns: usize,
    was_stuck: bool,
}

/// A brain that chooses the first row whose id ends in `suffix`, and runs from any battle.
fn choosing(suffix: &'static str, seen: Arc<Mutex<Seen>>) -> impl FnMut(&TurnRequest) -> Reply + Send {
    move |request: &TurnRequest| {
        let mut log = seen.lock().expect("not poisoned");
        if request.is_summary() {
            return Reply::Content("Walking for wild Pokémon.".to_string());
        }
        if request.is_stuck() {
            log.was_stuck = true;
            return Reply::call("choose_action", serde_json::json!({ "id": "press:b", "summary": "wedged" }));
        }
        if request.is_battle() {
            log.battle_turns += 1;
            return Reply::call("choose_action", serde_json::json!({ "id": "run", "summary": "not now" }));
        }
        if request.is_nickname() {
            return Reply::choose("NONE");
        }
        if !request.is_overworld() {
            return Reply::Calls(vec![Call::wait(10)]);
        }
        log.overworld_turns.push(request.situation().to_string());
        match request.menu_ids().into_iter().find(|id| id.ends_with(suffix)) {
            Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "pacing" })),
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    }
}

/// Choose the `suffix` row from `fixture` and wait for the wild battle it is for.
fn paces_into_a_battle(fixture: &'static [u8], name: &'static str, suffix: &'static str) -> LlmRun {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut run = LlmRun::builder(fixture)
        .named(name)
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(choosing(suffix, Arc::clone(&seen))));

    let fought = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns > 0
    });
    let seen = seen.lock().expect("not poisoned");
    let first = seen.overworld_turns.first().map_or("<never asked>", String::as_str);
    assert!(first.contains(suffix), "the first turn offered no `{suffix}` row:\n{first}");
    assert!(fought, "no wild battle came of pacing: {} overworld turns", seen.overworld_turns.len());
    assert!(!seen.was_stuck, "the watchdog fired while pacing");
    drop(seen);
    run
}

/// A cave has no grass, and every floor square rolls an encounter.
#[test]
fn a_cave_floor_is_paced_for_wild_pokemon() {
    let mut run = paces_into_a_battle(include_bytes!("../data/soak-mt-moon.bin"), "pace-cave", ":Pace");
    assert!(!run.fixture().game_state().map.surfing, "a cave floor is walked, not surfed");
}

/// Water rolls its own rate while surfing.
#[test]
fn water_is_surfed_for_wild_pokemon() {
    let mut run = paces_into_a_battle(include_bytes!("../data/route21-islands.bin"), "pace-water", ":PaceOnWater");
    assert!(run.fixture().game_state().map.surfing, "the battle came from somewhere other than the water");
}

/// From dry land the row mounts Surf on the way, as a water crossing does: grass on one of Route
/// 21's islands first, then the water around it.
#[test]
fn water_is_surfed_for_wild_pokemon_from_dry_land() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let seen = Arc::new(Mutex::new(Seen::default()));
    let ashore = Arc::new(AtomicBool::new(false));
    let mut grass = choosing(":Grass", Arc::clone(&seen));
    let mut water = choosing(":PaceOnWater", Arc::clone(&seen));
    let on_land = Arc::clone(&ashore);
    let brain = move |request: &TurnRequest| match on_land.load(Ordering::Relaxed) {
        true => water(request),
        false => grass(request),
    };
    let mut run = LlmRun::builder(include_bytes!("../data/route21-islands.bin"))
        .named("pace-mount")
        .game_time(Duration::from_secs(30 * 60))
        .start(Box::new(brain));

    let mut landed_at = None;
    let surfed = run.tick_until(PATIENCE, |run| {
        let state = run.fixture().game_state();
        if landed_at.is_none() && !state.map.surfing && state.battle.is_none() && state.map.position_settled {
            landed_at = Some(state.map.player_position);
            ashore.store(true, Ordering::Relaxed);
        }
        landed_at.is_some() && state.battle.is_some() && state.map.surfing
    });
    assert!(landed_at.is_some(), "the walk to the island's grass never came ashore");
    // Ashore, then surfing, and only the water row was chosen in between: the row mounted Surf.
    assert!(surfed, "no battle came of surfing from the island");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired");
}

/// A brain that walks into each map of `path` in turn by its rows, then makes each of `calls` in
/// the last, one a turn.
fn walk_in_then(
    path: &'static [&'static str],
    calls: Vec<Call>,
    seen: Arc<Mutex<Seen>>,
) -> impl FnMut(&TurnRequest) -> Reply + Send {
    use crate::pokemon::integration_tests::scripted_brain::Intent;
    let mut calls: std::collections::VecDeque<Call> = calls.into();
    let mut leg = 0;
    move |request: &TurnRequest| {
        let mut log = seen.lock().expect("not poisoned");
        for message in request.messages.iter().filter(|message| message.role == "tool") {
            if !log.tool_results.contains(&message.text) {
                log.tool_results.push(message.text.clone());
            }
        }
        if request.is_summary() {
            return Reply::Content("Visiting a building.".to_string());
        }
        if request.is_stuck() {
            log.was_stuck = true;
            return Reply::call("choose_action", serde_json::json!({ "id": "press:b", "summary": "wedged" }));
        }
        if request.is_battle() {
            log.battle_turns += 1;
            return Reply::call("choose_action", serde_json::json!({ "id": "run", "summary": "not now" }));
        }
        if request.is_nickname() {
            return Reply::choose("NONE");
        }
        if !request.is_overworld() {
            return Reply::Calls(vec![Call::wait(10)]);
        }
        log.overworld_turns.push(request.situation().to_string());
        while leg + 1 < path.len() && Intent::Enter(path[leg]).satisfied_by(request) {
            leg += 1;
        }
        let map = path[leg];
        if !Intent::Enter(map).satisfied_by(request) {
            return match Intent::Enter(map).resolve(request) {
                Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "on my way" })),
                None => Reply::Calls(vec![Call::wait(30)]),
            };
        }
        match calls.pop_front() {
            Some(call) => Reply::Calls(vec![call]),
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    }
}

fn row(id: &str) -> Call {
    Call::new("choose_action", serde_json::json!({ "id": id, "summary": "doing the thing" }))
}

/// A Pokémon is boarded at the Day Care and collected again, each by a call a model can make.
#[test]
fn a_pokemon_is_boarded_at_the_day_care_and_collected() {
    use crate::pokemon::move_name::PokemonMoveName as Move;
    const DAYCARE: &[u8] = include_bytes!("../data/postgame-daycare.bin");
    let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(DAYCARE).expect("the fixture loads");
    let party = crate::pokemon::PokemonApiTrait::game_state(&crate::pokemon::PokemonApi::new(&mut gb))
        .expect("a readable state").pokemon;
    let knows_an_hm = |slot: usize| party[slot].moves.iter().flatten()
        .any(|m| matches!(m.name, Move::Cut | Move::Fly | Move::Surf | Move::Strength | Move::Flash));
    let refused = (1..party.len()).find(|&slot| knows_an_hm(slot)).expect("a member that knows an HM");
    let free = (1..party.len()).find(|&slot| !knows_an_hm(slot)).expect("a member that knows none");

    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = walk_in_then(&["Daycare"], vec![
        // A slot the row does not name, which the gentleman refuses.
        row(&format!("daycare:deposit@{refused}")),
        row(&format!("daycare:deposit@{free}")),
        row("daycare:withdraw"),
    ], Arc::clone(&seen));
    let mut run = LlmRun::builder(include_bytes!("../data/postgame-daycare.bin"))
        .named("day-care")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    let before = run.fixture().game_state();
    let boarded = before.pokemon[free].species;

    let deposited = run.tick_until(PATIENCE, |run| run.fixture().game_state().day_care_in_use);
    assert!(deposited, "the Day Care never took a Pokémon");
    let during = run.fixture().game_state();
    assert_eq!(during.pokemon.len(), before.pokemon.len() - 1, "the party did not shrink");
    assert!(!during.pokemon.iter().any(|mon| mon.species == boarded) || before.pokemon.iter()
        .filter(|mon| mon.species == boarded).count() > 1, "the wrong Pokémon was boarded");

    let collected = run.tick_until(PATIENCE, |run| {
        let state = run.fixture().game_state();
        !state.day_care_in_use && state.pokemon.len() == before.pokemon.len()
    });
    assert!(collected, "the boarded Pokémon was never collected");
    let after = run.fixture().game_state();
    assert_eq!(after.pokemon.len(), before.pokemon.len(), "the party did not get it back");
    assert!(after.money < before.money, "the Day Care was not paid");
    // The row names only the slots the gentleman takes.
    let turns = seen.lock().expect("not poisoned").overworld_turns.clone();
    let offer = turns.iter().flat_map(|turn| turn.lines())
        .find(|line| line.starts_with("- `daycare:deposit@<slot>`")).expect("a deposit row");
    assert!(offer.contains(&crate::llm::prompt::named(&party[free])), "{offer}");
    assert!(!offer.contains(&format!("{refused} {}", crate::llm::prompt::named(&party[refused]))), "{offer}");
    // A row resolves once the turn has ended, so the refusal opens the next one.
    let told = turns.iter().any(|turn| turn.contains("knows an HM move"));
    assert!(told, "boarding a Pokémon that knows an HM was not refused with the gentleman's reason");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired at the Day Care");
}

/// A Rare Candy's level-up can start an evolution, and a model can let it run or stop it as B does.
#[test]
fn a_rare_candy_evolution_runs_or_is_stopped_as_the_model_says() {
    use crate::pokemon::species::PokemonSpecies;
    const SAFARI: &[u8] = include_bytes!("../data/completion-safari.bin");
    for (evolve, becomes) in [(false, PokemonSpecies::NidoranMale), (true, PokemonSpecies::Nidorino)] {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let id = match evolve { true => "item:RareCandy@4", false => "item:RareCandy@4:stay" };
        let brain = walk_in_then(&["Route19"], vec![row(id)], Arc::clone(&seen));
        let mut run = LlmRun::builder(SAFARI)
            .named("rare-candy")
            .game_time(Duration::from_secs(10 * 60))
            .start(Box::new(brain));
        // The fixture's party is the three the tour keeps, the seat its collecting fills, and the
        // two Nidoran the Safari Zone caught for the trades ahead.
        let before = run.fixture().game_state().pokemon[4].clone();
        assert_eq!((before.species, before.level), (PokemonSpecies::NidoranMale, 22),
            "slot 4 should be the Safari Zone's Nidoran, a level short of what evolves it");

        let fed = run.tick_until(PATIENCE, |run| {
            let state = run.fixture().game_state();
            state.mode == crate::pokemon::encoding::GameMode::Overworld && state.pokemon[4].level > before.level
        });
        assert!(fed, "the Rare Candy was never used (evolve: {evolve})");
        assert_eq!(run.fixture().game_state().pokemon[4].species, becomes, "evolve: {evolve}");
        assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired feeding a Rare Candy");
    }
}

/// A prize is bought with coins, by a call a model can make.
#[test]
fn a_prize_is_bought_at_the_game_corner() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = walk_in_then(&["GameCornerPrizeRoom"], vec![
        row("prize:Abra"),
    ], Arc::clone(&seen));
    let mut run = LlmRun::builder(include_bytes!("../data/postgame-game-corner.bin"))
        .named("prize")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    run.fixture().api().debug_set_coins(500);
    // `try_`: a Pokémon being written into the party reads as garbage for a tick.
    let owned = |run: &mut LlmRun| {
        let Ok(state) = run.fixture().try_game_state() else { return 0 };
        state.pokemon.iter().filter(|mon| mon.species == crate::pokemon::species::PokemonSpecies::Abra).count()
            + state.boxed_pokemon.iter().filter(|mon| mon.species == crate::pokemon::species::PokemonSpecies::Abra).count()
    };
    let abras = owned(&mut run);

    let bought = run.tick_until(PATIENCE, |run| run.fixture().try_game_state().is_ok_and(|state| state.coins < 500));
    assert!(bought, "no coins were spent in the prize room");
    run.tick_until(PATIENCE, |run| owned(run) > abras);
    assert_eq!(run.fixture().game_state().coins, 500 - 180, "Abra costs 180 coins");
    assert!(owned(&mut run) > abras, "the Abra went nowhere");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired in the prize room");
}

/// A prize further down a counter's menu is the one bought, not the first.
#[test]
fn a_prize_below_the_first_row_is_bought() {
    use crate::pokemon::item::ItemId;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = walk_in_then(&["GameCornerPrizeRoom"], vec![
        row("prize:Tm50Substitute"),
    ], Arc::clone(&seen));
    let mut run = LlmRun::builder(include_bytes!("../data/postgame-game-corner.bin"))
        .named("prize-tm")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    run.fixture().api().debug_set_coins(9000);
    let bought = run.tick_until(PATIENCE, |run| run.fixture().try_game_state()
        .is_ok_and(|state| state.bag.contains(&ItemId::Tm50Substitute)));
    let turns = seen.lock().expect("not poisoned").overworld_turns.clone();
    assert!(bought, "no TM50 was bought; the last turn:\n{}", turns.last().map_or("", String::as_str));
    assert_eq!(run.fixture().game_state().coins, 9000 - 7700, "TM50 costs 7700 coins");
}

/// A machine prize with no room in the bag is not offered, and asking for it costs no walk.
#[test]
fn a_machine_prize_is_not_offered_to_a_full_bag() {
    use crate::pokemon::item::ItemId;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = walk_in_then(&["GameCornerPrizeRoom"], vec![row("prize:Tm50Substitute")], Arc::clone(&seen));
    let mut run = LlmRun::builder(include_bytes!("../data/postgame-game-corner.bin"))
        .named("prize-full-bag")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    run.fixture().api().debug_set_coins(9000);
    // Every kind the bag has room for, so the machine would be a twenty-first.
    let fillers = (1..=255u8).filter_map(ItemId::from_repr)
        .filter(|id| !id.is_key_item() && !id.is_hm() && !id.is_floor() && *id != ItemId::Tm50Substitute);
    for filler in fillers {
        if run.fixture().game_state().bag.len() >= crate::pokemon::bag::Bag::MAX_ITEMS { break }
        if !run.fixture().game_state().bag.contains(&filler) {
            run.fixture().api().debug_give_item(filler, 1).ok();
        }
    }
    let refused = run.tick_until(PATIENCE, |_| seen.lock().expect("not poisoned").tool_results.iter()
        .any(|result| result.contains("`prize:Tm50Substitute` is not one of this turn's actions")));
    assert!(refused, "the machine the bag has no room for was not refused");
    let turns = seen.lock().expect("not poisoned").overworld_turns.clone();
    let here = turns.last().expect("a turn in the prize room");
    assert!(here.contains("`prize:Abra`") && !here.contains("`prize:Tm50Substitute`"), "{here}");
    assert_eq!(run.fixture().game_state().coins, 9000, "coins were spent on a prize that could not be held");
}

/// Every floor a lift serves can be picked, not only the first: the Rocket Hideout's B4F is the
/// third row of its panel and the one way into Giovanni's half of that floor.
#[test]
fn a_lift_goes_to_every_floor_its_panel_lists() {
    use crate::pokemon::map::Map;
    let seen = Arc::new(Mutex::new(Seen::default()));
    // Down to B1F, the panel's first floor, and back up to B4F, its third.
    let brain = walk_in_then(&["RocketHideoutElevator"], vec![
        row("lift:RocketHideoutB1F"),
        row("lift:RocketHideoutB4F"),
    ], Arc::clone(&seen));
    let mut run = LlmRun::builder(include_bytes!("../data/post-silph-scope.bin"))
        .named("lift-third-floor")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));

    let down = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        run.fixture().try_game_state().is_ok_and(|state| state.map.map == Map::RocketHideoutB1F)
    });
    assert!(down, "the lift never reached B1F");
    let up = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        run.fixture().try_game_state().is_ok_and(|state| state.map.map == Map::RocketHideoutB4F)
    });
    let turns = seen.lock().expect("not poisoned").overworld_turns.clone();
    assert!(up, "the lift never came back to B4F; the last turn:\n{}", turns.last().map_or("", String::as_str));
}

/// The Name Rater's party menu is declined, and the conversation hands the run back unchanged.
#[test]
fn talking_to_the_name_rater_changes_nothing() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = walk_in_then(&["NameRatersHouse"], vec![
        Call::new("choose_action", serde_json::json!({ "id": "NameRatersHouse:NameRater", "summary": "names" })),
    ], Arc::clone(&seen));
    let mut run = LlmRun::builder(include_bytes!("../data/at-lavender.bin"))
        .named("name-rater")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    let names = |run: &mut LlmRun| run.fixture().game_state().pokemon.iter()
        .map(|mon| mon.nickname.to_default_string()).collect::<Vec<_>>();
    let before = names(&mut run);

    let inside = |seen: &Arc<Mutex<Seen>>| seen.lock().expect("not poisoned").overworld_turns.iter()
        .filter(|turn| turn.contains("Location: NameRatersHouse")).count();
    let talked_and_back = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        inside(&seen) >= 2
    });
    assert!(talked_and_back, "the run never came back to a turn in the Name Rater's house");
    assert_eq!(names(&mut run), before, "a name changed that nobody chose");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired at the Name Rater");
}

/// The gate flags the quiz machines open, `EVENT_CINNABAR_GYM_GATE0_UNLOCKED` onward.
fn gates_open(run: &mut LlmRun) -> u8 {
    use crate::pokemon::symbols::{pokered_events::EVENT_CINNABAR_GYM_GATE0_UNLOCKED as FIRST, pokered_symbols};
    use gb::ram::ROM;
    let mmu = run.fixture().gb.core().mmu();
    (0..7).filter(|gate| {
        let flag = FIRST + gate;
        mmu.read(pokered_symbols::wEventFlags.address + flag / 8) & (1 << (flag % 8)) != 0
    }).count() as u8
}

/// A quiz machine answered from its row: right opens a gate, wrong is the cartridge's battle.
fn answers_the_first_quiz_machine(answer: &'static str) -> (LlmRun, Arc<Mutex<Seen>>, u8) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = walk_in_then(&["CinnabarGym"], vec![
        Call::new("choose_action", serde_json::json!({ "id": format!("CinnabarGym:15,7:{answer}"), "summary": "quiz" })),
    ], Arc::clone(&seen));
    let mut run = LlmRun::builder(include_bytes!("../data/at-cinnabar.bin"))
        .named("cinnabar-quiz")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    // The door wants the Secret Key, which is the Mansion's business, not this test's.
    run.fixture().api().debug_take_item(crate::pokemon::item::ItemId::Tm46Psywave).expect("the fixture's junk TM");
    run.fixture().api().debug_give_item(crate::pokemon::item::ItemId::SecretKey, 1).expect("room in the bag");
    let before = gates_open(&mut run);
    (run, seen, before)
}

#[test]
fn a_quiz_machine_answered_right_opens_its_gate() {
    let (mut run, seen, before) = answers_the_first_quiz_machine("QuizYes1");
    let opened = run.tick_until(PATIENCE, |run| gates_open(run) > before);
    let seen = seen.lock().expect("not poisoned");
    let asked = seen.overworld_turns.iter().find(|turn| turn.contains("QuizYes1")).map_or("<never asked>", String::as_str);
    assert!(asked.contains("CATERPIE evolves into BUTTERFREE?"), "the row does not quote its question:\n{asked}");
    assert!(opened, "the right answer opened no gate");
    assert_eq!(seen.battle_turns, 0, "a right answer started a battle");
}

#[test]
fn a_quiz_machine_answered_wrong_is_a_trainer_battle() {
    let (mut run, seen, before) = answers_the_first_quiz_machine("QuizNo2");
    let fought = run.tick_until(PATIENCE, |_| seen.lock().expect("not poisoned").battle_turns > 0);
    assert!(fought, "a wrong answer started no battle");
    assert_eq!(gates_open(&mut run), before, "a wrong answer opened a gate without the battle");
}

/// Seafoam's boulder puzzle as a model plays it: each boulder named by its row, down the hole it
/// fills, and on to Articuno.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "a Strength floor; run with --features slow-tests")]
fn seafoams_boulders_are_pushed_by_their_rows_down_to_articuno() {
    use crate::pokemon::integration_tests::scripted_brain::{Intent, ScriptedBrain};
    use crate::pokemon::species::PokemonSpecies;
    let brain = ScriptedBrain::new(vec![
        // One row per hole, naming the one boulder that can reach it.
        Intent::Repeat("hole at (3, 16)"),
        Intent::Repeat("hole at (6, 16)"),
        // Down the hole just filled, into the west lake: the stairs land on the wrong side.
        Intent::Says("SeafoamIslandsB4F, arriving at (5, 14)"),
        Intent::Row("Articuno"),
    ]);
    let stuck = Arc::clone(&brain.stuck);
    let mut run = LlmRun::builder(include_bytes!("../data/seafoam-b3f.bin"))
        .named("seafoam-boulders")
        .game_time(Duration::from_mins(60))
        .start(Box::new(brain));

    let met = run.tick_until(Duration::from_secs(600), |run| {
        stuck.lock().expect("not poisoned").is_some()
            || run.fixture().try_game_state().is_ok_and(|state| state.battle.as_ref()
                .is_some_and(|battle| battle.enemy.species == PokemonSpecies::Articuno))
    });
    if let Some(why) = stuck.lock().expect("not poisoned").clone() {
        panic!("the Seafoam puzzle could not be played from its rows:\n{why}");
    }
    assert!(met, "never reached Articuno");
    // Each boulder that fell is hidden on B3F: (3, 15) is its second object, (8, 14) its third.
    use crate::pokemon::symbols::{pokered_symbols, pokered_toggles};
    use gb::ram::ROM;
    let mmu = run.fixture().gb.core().mmu();
    for (toggle, at) in [(pokered_toggles::TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_2, "(3, 15)"),
                         (pokered_toggles::TOGGLE_SEAFOAM_ISLANDS_B3F_BOULDER_3, "(8, 14)")] {
        let hidden = mmu.read(pokered_symbols::wToggleableObjectFlags.address + toggle / 8) & (1 << (toggle % 8)) != 0;
        assert!(hidden, "the boulder at {at} never went down its hole");
    }
}

/// A Seafoam hole with the current below it still strong says where the current carries the player,
/// and the next turn is asked only once it has.
#[test]
fn a_seafoam_hole_says_where_the_current_below_leaves_the_player() {
    let turns: Arc<Mutex<Vec<(String, Vec<(String, String)>)>>> = Arc::default();
    let log = Arc::clone(&turns);
    let brain = move |request: &TurnRequest| {
        if request.is_battle() {
            return Reply::call("choose_action", serde_json::json!({ "id": "run", "summary": "not now" }));
        }
        if !request.is_overworld() {
            return Reply::Calls(vec![Call::wait(10)]);
        }
        let rows = request.menu_rows();
        let here = request.situation().lines()
            .find_map(|line| line.strip_prefix("Location: ")).unwrap_or_default().to_string();
        let hole = rows.iter().find(|(id, _)| id == "SeafoamIslandsB3F:3,16:Warp").map(|(id, _)| id.clone());
        log.lock().expect("not poisoned").push((here, rows));
        match hole {
            Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "down the hole" })),
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    };
    let mut run = LlmRun::builder(include_bytes!("../data/seafoam-b3f.bin"))
        .named("seafoam-current")
        .game_time(Duration::from_mins(10))
        .start(Box::new(brain));

    let below = run.tick_until(PATIENCE, |_| {
        turns.lock().expect("not poisoned").iter().any(|(here, _)| here.starts_with("SeafoamIslandsB4F"))
    });
    let turns = turns.lock().expect("not poisoned");
    let (_, first) = turns.first().expect("no overworld turn was asked");
    let row = first.iter().find(|(id, _)| id == "SeafoamIslandsB3F:3,16:Warp")
        .unwrap_or_else(|| panic!("the hole is not a row: {first:?}"));
    assert!(row.1.contains("SeafoamIslandsB4F, arriving at (7, 10)"), "the hole's row: {}", row.1);
    assert!(below, "never asked a turn on B4F: {:?}", turns.iter().map(|(here, _)| here).collect::<Vec<_>>());
    let landed = turns.iter().map(|(here, _)| here).find(|here| !here.starts_with("SeafoamIslandsB3F"));
    assert!(landed.is_some_and(|here| here.starts_with("SeafoamIslandsB4F at (7, 10)")),
        "the first turn off B3F must be asked where the current stops, not {landed:?}");
}

/// An item ball with tall grass on every side is still a row, and a model can pick it up.
#[test]
fn an_item_ringed_by_tall_grass_can_be_picked_up() {
    use crate::pokemon::item::ItemId;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut run = LlmRun::builder(include_bytes!("../data/viridian-forest.bin"))
        .named("grass-ringed-ball")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(choosing(":PokeBall", Arc::clone(&seen))));
    let balls = |run: &mut LlmRun| run.fixture().try_game_state().ok()
        .and_then(|state| state.bag.iter().find(|item| item.id == ItemId::PokeBall).map(|item| item.quantity))
        .unwrap_or(0);
    let before = balls(&mut run);
    let picked = run.tick_until(PATIENCE, |run| balls(run) > before);
    let first = seen.lock().expect("not poisoned").overworld_turns.first().cloned().unwrap_or_default();
    assert!(first.contains("ViridianForest:PokeBall"), "the ball was not offered:\n{first}");
    assert!(picked, "the ball was never picked up");
}

/// Teach Strength to a Mewtwo that knows four moves, answering the move to forget with `forget`.
fn teaches_strength(forget: Option<u8>, name: &'static str) -> (LlmRun, Arc<Mutex<Seen>>) {
    use crate::pokemon::item::ItemId;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut walk = walk_in_then(&["CeruleanCity"], vec![
        row("teach:Hm04Strength@0"),
    ], Arc::clone(&seen));
    let brain = move |request: &TurnRequest| match request.is_forget_move() {
        true => Reply::choose(&forget.map_or("keep".to_string(), |slot| format!("forget:{slot}"))),
        false => walk(request),
    };
    let mut run = LlmRun::builder(include_bytes!("../data/completion-bill.bin"))
        .named(name)
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    run.fixture().api().debug_give_item(ItemId::Hm04Strength, 1).expect("room in the bag");
    (run, seen)
}

fn knows_strength(run: &mut LlmRun) -> bool {
    use crate::pokemon::move_name::PokemonMoveName as Move;
    run.fixture().try_game_state().is_ok_and(|state|
        state.pokemon.iter().next().is_some_and(|mon| mon.moves.iter().flatten().any(|m| m.name == Move::Strength)))
}

/// A Pokémon with four moves forgets the one chosen and learns the HM.
#[test]
fn an_hm_replaces_the_move_chosen_to_forget() {
    let (mut run, seen) = teaches_strength(Some(0), "teach-forget");
    let taught = run.tick_until(PATIENCE, knows_strength);
    assert!(taught, "Strength was never learned");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired while teaching");
}

/// Keeping all four moves ends the teach, rather than asking again for as long as the run lasts.
#[test]
fn declining_the_move_to_forget_ends_the_teach() {
    let (mut run, seen) = teaches_strength(None, "teach-decline");
    let told = run.tick_until(PATIENCE, |_| seen.lock().expect("not poisoned").overworld_turns.iter()
        .any(|turn| turn.contains("did not learn the move")));
    assert!(told, "no overworld turn said the move was not learned");
    assert!(!knows_strength(&mut run), "Strength was learned though nothing was forgotten");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired while teaching");
}

/// A battle reloads the map and every cut tree grows back, however the battle ends: here with a
/// catch, whose naming screen stands between the battle and the overworld.
#[test]
fn a_tree_grows_back_after_a_battle_that_ends_in_a_catch() {
    use crate::pokemon::item::ItemId;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = Arc::clone(&seen);
    let (mut cut, mut caught) = (false, false);
    let brain = move |request: &TurnRequest| {
        if request.is_summary() {
            return Reply::Content("Cutting and catching.".to_string());
        }
        if request.is_battle() {
            return Reply::call("choose_action", serde_json::json!({ "id": "item:MasterBall", "summary": "catch" }));
        }
        if request.is_nickname() {
            caught = true;
            return Reply::choose("NONE");
        }
        if !request.is_overworld() {
            return Reply::Calls(vec![Call::wait(10)]);
        }
        log.lock().expect("not poisoned").overworld_turns.push(request.situation().to_string());
        let rows = request.menu_rows();
        let row = |what: &str| rows.iter().find(|(_, description)| description.starts_with(what)).map(|(id, _)| id.clone());
        let id = if !cut {
            cut = true;
            row("cut down the tree at (30, 12)")
        } else if !caught {
            row("pace back and forth in the nearest tall grass")
        } else {
            // Out west: through the gate if the way is open, or the tree again.
            row("take the warp to Route8Gate").or_else(|| row("cut down the tree at (30, 12)"))
        };
        match id {
            Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "on my way" })),
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    };
    let mut run = LlmRun::builder(include_bytes!("../data/route8-cut-trees.bin"))
        .named("regrown-tree")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    run.fixture().api().debug_give_item(ItemId::MasterBall, 5).expect("room in the bag");

    let through = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        run.fixture().try_game_state().is_ok_and(|state| state.map.map == crate::pokemon::map::Map::Route8Gate)
    });
    let turns = seen.lock().expect("not poisoned").overworld_turns.clone();
    assert!(through, "never reached the gate; the last turn:\n{}", turns.last().map_or("", String::as_str));
    assert!(!turns.iter().any(|turn| turn.contains("gave up on the warp to Route8Gate")),
            "the gate was offered through a tree that had grown back");
}

/// Each vending row buys the drink it names, not the cheapest: the roof girl trades a different TM
/// for each of the three.
#[test]
fn a_vending_row_buys_the_drink_it_names() {
    use crate::pokemon::item::ItemId;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut walk = walk_in_then(&["CeladonMart1F", "CeladonMart2F", "CeladonMart3F", "CeladonMart4F",
                                  "CeladonMart5F", "CeladonMartRoof"], vec![], Arc::clone(&seen));
    let mut bought = false;
    let brain = move |request: &TurnRequest| {
        if !bought && request.location().as_deref() == Some("CeladonMartRoof") && request.is_overworld()
            && let Some((id, _)) = request.menu_rows().into_iter().find(|(_, what)| what.starts_with("buy a SODA POP"))
        {
            bought = true;
            return Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "a drink" }));
        }
        walk(request)
    };
    let mut run = LlmRun::builder(include_bytes!("../data/at-celadon.bin"))
        .named("vending")
        .game_time(Duration::from_secs(20 * 60))
        .start(Box::new(brain));
    let held = |run: &mut LlmRun, drink| run.fixture().try_game_state().is_ok_and(|state| state.bag.contains(&drink));

    let got = run.tick_until(PATIENCE, |run| held(run, ItemId::SodaPop));
    assert!(got, "no Soda Pop came out of the machine");
    assert!(!held(&mut run, ItemId::FreshWater), "the cheapest drink was bought instead, or as well");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired at the machine");
}

/// Print the first overworld turns a dropped save produces, as the model would see them.
/// Set `GB_PROBE_STATE` to the save.
#[test]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_policy_menu() {
    let path = std::env::var("GB_PROBE_STATE").expect("GB_PROBE_STATE");
    let bytes: &'static [u8] = Box::leak(std::fs::read(&path).expect("state").into_boxed_slice());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = Arc::clone(&seen);
    let brain = move |request: &TurnRequest| {
        if request.is_summary() {
            return Reply::Content("Looking around.".to_string());
        }
        if request.is_overworld() {
            log.lock().expect("not poisoned").overworld_turns.push(request.situation().to_string());
        }
        Reply::Calls(vec![Call::wait(20)])
    };
    let mut run = LlmRun::builder(bytes)
        .named("probe-menu")
        .game_time(Duration::from_secs(10 * 60))
        .start(Box::new(brain));
    run.tick_until(Duration::from_secs(30), |_| seen.lock().expect("not poisoned").overworld_turns.len() >= 3);
    for (n, turn) in seen.lock().expect("not poisoned").overworld_turns.iter().enumerate() {
        let rows: Vec<&str> = turn.lines().filter(|line| line.starts_with("- `")).collect();
        println!("turn {n}: {} rows", rows.len());
        for row in rows { println!("   {row}"); }
    }
}

/// A one-shove row reports how it went even when its shove runs long enough to read as a script,
/// as every shove on Seafoam B3F does.
#[test]
fn a_one_shove_row_says_it_landed_after_a_long_shove() {
    const ROW: &str = "SeafoamIslandsB3F:5,14:PushBoulderLeft";
    let told: Arc<Mutex<Option<String>>> = Arc::default();
    let log = Arc::clone(&told);
    let brain = move |request: &TurnRequest| {
        if request.is_battle() {
            return Reply::call("choose_action", serde_json::json!({ "id": "run", "summary": "not now" }));
        }
        if !request.is_overworld() {
            return Reply::Calls(vec![Call::wait(10)]);
        }
        let outcome = request.situation().lines()
            .find(|line| line.contains("the boulder at (5, 14)") && (line.starts_with("- ✓") || line.starts_with("- ✗")))
            .map(str::to_string);
        if outcome.is_some() {
            *log.lock().expect("not poisoned") = outcome;
            return Reply::Calls(vec![Call::wait(30)]);
        }
        match request.menu_ids().into_iter().find(|id| id == ROW) {
            Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "one shove" })),
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    };
    let mut run = LlmRun::builder(include_bytes!("../data/seafoam-b3f.bin"))
        .named("one-shove")
        .game_time(Duration::from_mins(10))
        .start(Box::new(brain));
    run.tick_until(PATIENCE, |_| told.lock().expect("not poisoned").is_some());
    let told = told.lock().expect("not poisoned").clone();
    assert!(told.as_deref().is_some_and(|line| line.contains("✓ reached")), "{ROW}: the model was told {told:?}");
}

/// A second vending machine straight after a first: the first purchase leaves the menu's geometry
/// in RAM, and the second must still read its greeting before it picks.
#[test]
fn a_second_vending_machine_sells_its_own_drink() {
    use crate::pokemon::integration_tests::scripted_brain::Intent;
    const PATH: &[&str] = &["CeladonMart1F", "CeladonMart2F", "CeladonMart3F", "CeladonMart4F", "CeladonMart5F", "CeladonMartRoof"];
    let told = Arc::new(Mutex::new(false));
    let log = Arc::clone(&told);
    let mut leg = 0;
    let mut picks = vec!["CeladonMartRoof:11,1:VendingMachine2", "CeladonMartRoof:10,1:VendingMachine1"];
    let brain = move |request: &TurnRequest| {
        if request.is_battle() {
            return Reply::call("choose_action", serde_json::json!({ "id": "run", "summary": "not now" }));
        }
        if !request.is_overworld() {
            return Reply::Calls(vec![Call::wait(10)]);
        }
        if request.situation().contains("SODA POP popped out") {
            *log.lock().expect("not poisoned") = true;
        }
        while leg + 1 < PATH.len() && Intent::Enter(PATH[leg]).satisfied_by(request) { leg += 1; }
        if !Intent::Enter(PATH[leg]).satisfied_by(request) {
            return match Intent::Enter(PATH[leg]).resolve(request) {
                Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "up" })),
                None => Reply::Calls(vec![Call::wait(30)]),
            };
        }
        match picks.pop() {
            Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "a drink" })),
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    };
    let mut run = LlmRun::builder(include_bytes!("../data/postgame-game-corner.bin"))
        .named("two-vending-machines")
        .game_time(Duration::from_mins(20))
        .start(Box::new(brain));
    let sold = run.tick_until(PATIENCE, |_| *told.lock().expect("not poisoned"));
    assert!(sold, "the second machine never sold its SODA POP");
}

/// The slots are one row the model chooses by its id, and the next turn says what came of it.
#[test]
fn a_slots_row_is_chosen_by_its_id_and_the_next_turn_says_what_it_won() {
    use crate::pokemon::integration_tests::scripted_brain::Intent;
    let told = Arc::new(Mutex::new(None));
    let log = Arc::clone(&told);
    let mut chosen = false;
    let brain = move |request: &TurnRequest| {
        if request.is_battle() {
            return Reply::call("choose_action", serde_json::json!({ "id": "run", "summary": "not now" }));
        }
        if !request.is_overworld() {
            return Reply::Calls(vec![Call::wait(10)]);
        }
        if let Some(line) = request.situation().lines().find(|line| line.contains("spins at the slot machine")) {
            *log.lock().expect("not poisoned") = Some(line.to_string());
            return Reply::Calls(vec![Call::wait(30)]);
        }
        if !Intent::Enter("GameCorner").satisfied_by(request) {
            return match Intent::Enter("GameCorner").resolve(request) {
                Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "in" })),
                None => Reply::Calls(vec![Call::wait(30)]),
            };
        }
        match request.menu_ids().into_iter().find(|id| id == "GameCorner:Slots").filter(|_| !chosen) {
            Some(id) => {
                chosen = true;
                Reply::call("choose_action", serde_json::json!({ "id": id, "summary": "a flutter" }))
            }
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    };
    let mut run = LlmRun::builder(include_bytes!("../data/postgame-coins.bin"))
        .named("slots")
        .game_time(Duration::from_mins(10))
        .start(Box::new(brain));
    run.tick_until(PATIENCE, |_| told.lock().expect("not poisoned").is_some());
    let told = told.lock().expect("not poisoned").clone();
    assert!(told.as_deref().is_some_and(|line| line.contains("played 10 spins at the slot machine: 200 coins became")),
            "the model was told {told:?}");
}

/// Pallet Town, standing outside, with the starter.
const PALLET: &[u8] = include_bytes!("../data/pallet-town-state.bin");

/// An Ether's overworld row restores the one move its id names.
#[test]
fn an_ether_row_restores_the_move_it_names() {
    use crate::pokemon::item::ItemId;
    use crate::pokemon::postgame::items::{bag_quantity, move_pp};
    use crate::pokemon::integration_tests::llm_harness::takes_once_offered;
    let situations = Arc::new(Mutex::new(Vec::new()));
    let brain = takes_once_offered("item:Ether@<slot>:<move>", &["item:Ether@0:1"], Arc::clone(&situations));
    let mut run = LlmRun::builder(PALLET).named("ether-row").game_time(Duration::from_secs(10 * 60)).start(Box::new(brain));
    // Debug tier: a spent move, and an Ether to restore it with.
    let mut party = run.fixture().game_state().pokemon.clone();
    let untouched = party[0].moves[0];
    party[0].moves[1].as_mut().expect("the starter knows two moves").pp = 0;
    run.fixture().api().debug_set_party(&party).expect("a party can be installed");
    run.fixture().api().debug_give_item(ItemId::Ether, 1).expect("room for an Ether");

    let restored = run.tick_until(PATIENCE, |run| run.fixture().try_game_state().is_ok_and(|state|
        bag_quantity(&state, ItemId::Ether) == 0 && state.mode == crate::pokemon::encoding::GameMode::Overworld));
    let situations = situations.lock().expect("not poisoned").clone();
    let offer = situations.iter().flat_map(|turn| turn.lines()).find(|line| line.starts_with("- `item:Ether@<slot>:<move>`"))
        .unwrap_or_else(|| panic!("no Ether row: {situations:#?}"));
    assert!(offer.ends_with(&format!("restores 10 PP to one move: 0:1 {}'s {} 0/{}",
        crate::llm::prompt::named(&party[0]), party[0].moves[1].unwrap().name, party[0].moves[1].unwrap().name.metadata().pp)), "{offer}");
    assert!(restored, "the Ether was never used: {situations:#?}");
    let state = run.fixture().game_state();
    assert_eq!(move_pp(&state.pokemon[0].moves[1].unwrap()), 10, "the move the id names gets 10 PP");
    assert_eq!(state.pokemon[0].moves[0], untouched, "and no other");
}

/// Softboiled: not offered while nobody else is hurt or the giver has a fifth of its max HP or
/// less; a fainted target refused in the game's words; a hurt one healed, and the menus left.
#[test]
fn a_softboiled_row_heals_a_hurt_member_and_only_when_the_game_would() {
    use crate::pokemon::integration_tests::llm_harness::takes_once_offered;
    use crate::pokemon::move_name::PokemonMoveName as M;
    use crate::pokemon::pokemon::Pokemon;
    use crate::pokemon::species::PokemonSpecies;
    let situations = Arc::new(Mutex::new(Vec::<String>::new()));
    let brain = takes_once_offered("softboiled@<slot>", &["softboiled@2", "softboiled@0"], Arc::clone(&situations));
    let mut run = LlmRun::builder(PALLET).named("softboiled-row").game_time(Duration::from_secs(10 * 60)).start(Box::new(brain));
    // Debug tier: the starter, a Chansey that knows Softboiled, and a fainted Pidgey.
    let state = run.fixture().game_state();
    let mut party = crate::pokemon::party::PokemonParty::default();
    party.push(state.pokemon[0].clone()).expect("room");
    let chansey = Pokemon::maxed(PokemonSpecies::Chansey, "CHANSEY", [M::Pound, M::Softboiled, M::Growl, M::Minimize],
                                 state.name.clone(), state.player_id);
    let pidgey = Pokemon::maxed(PokemonSpecies::Pidgey, "PIDGEY", [M::Gust, M::SandAttack, M::QuickAttack, M::Tackle],
                                state.name.clone(), state.player_id);
    let (max, share, starter_max) = (chansey.stats.hp, chansey.stats.hp / 5, party[0].stats.hp);
    party.push(chansey).expect("room");
    party.push(pidgey).expect("room");
    run.fixture().api().debug_set_party(&party).expect("a party can be installed");
    run.fixture().api().debug_set_hp(2, 0);

    let turn_showing = |run: &mut LlmRun, needles: &[String]| -> String {
        let from = situations.lock().expect("not poisoned").len();
        let shown = |turns: &[String]| turns[from..].iter().find(|turn| needles.iter().all(|n| turn.contains(n.as_str()))).cloned();
        run.tick_until(PATIENCE, |_| shown(&situations.lock().expect("not poisoned")).is_some());
        shown(&situations.lock().expect("not poisoned")).unwrap_or_else(|| panic!("no turn showed {needles:?}"))
    };
    let nobody_hurt = turn_showing(&mut run, &[format!("CHANSEY Lv100 (Normal) — {max}/{max} HP")]);
    assert!(!nobody_hurt.contains("`softboiled@"), "{nobody_hurt}");

    run.fixture().api().debug_set_hp(0, 1);
    run.fixture().api().debug_set_hp(1, share);
    let weak = turn_showing(&mut run, &[format!("— 1/{starter_max} HP"), format!("— {share}/{max} HP")]);
    assert!(!weak.contains("`softboiled@"), "a fifth of the max HP is not enough to give: {weak}");

    run.fixture().api().debug_set_hp(1, max);
    let healed = (1 + share).min(starter_max);
    turn_showing(&mut run, &[format!("— {healed}/{starter_max} HP")]);
    let situations = situations.lock().expect("not poisoned").clone();
    let offer = situations.iter().flat_map(|turn| turn.lines()).find(|line| line.starts_with("- `softboiled@<slot>`"))
        .unwrap_or_else(|| panic!("no Softboiled row: {situations:#?}"));
    assert!(offer.ends_with(&format!("CHANSEY ({max}/{max} HP) gives {share} of its HP to another party member with \
        Softboiled: 0 {} 1/{starter_max} HP", crate::llm::prompt::named(&party[0]))), "only the hurt starter: {offer}");
    assert!(situations.iter().any(|turn| turn.contains("cannot revive")), "the fainted Pidgey was not refused");
    let state = run.fixture().game_state();
    assert_eq!(state.pokemon[1].current_hp, max - share, "the giver pays a fifth of its max HP");
    assert_eq!(state.pokemon[2].current_hp, 0, "the fainted Pidgey stays fainted");
}

/// Softboiled with a walk through a door chained behind it in one reply: the walk is taken once the
/// heal lands, and dropped, and named in the next turn's note, when the game refuses the heal. The
/// refusal is made live, after the row was resolved: the target faints under the driver, and the
/// cartridge will not heal a fainted Pokémon.
fn softboiled_then_a_door(refuse: bool, name: &'static str) -> (LlmRun, Vec<String>, String) {
    use crate::pokemon::integration_tests::llm_harness::chains_a_door_behind;
    use crate::pokemon::move_name::PokemonMoveName as M;
    use crate::pokemon::pokemon::Pokemon;
    use crate::pokemon::species::PokemonSpecies;
    let situations = Arc::new(Mutex::new(Vec::<String>::new()));
    let door = Arc::new(Mutex::new(None));
    let brain = chains_a_door_behind("softboiled@<slot>", "softboiled@0", Arc::clone(&situations), Arc::clone(&door));
    let mut run = LlmRun::builder(PALLET).named(name).game_time(Duration::from_secs(10 * 60)).start(Box::new(brain));
    // Debug tier: the starter at 1 HP, and a Chansey that knows Softboiled.
    let state = run.fixture().game_state();
    let mut party = crate::pokemon::party::PokemonParty::default();
    party.push(state.pokemon[0].clone()).expect("room");
    party.push(Pokemon::maxed(PokemonSpecies::Chansey, "CHANSEY", [M::Pound, M::Softboiled, M::Growl, M::Minimize],
                              state.name.clone(), state.player_id)).expect("room");
    run.fixture().api().debug_set_party(&party).expect("a party can be installed");
    run.fixture().api().debug_set_hp(0, 1);

    let mut fainted = false;
    let settled = run.tick_until(PATIENCE, |run| {
        if refuse && !fainted && run.fixture().agent.state_debug().starts_with("softboiled") {
            run.fixture().api().debug_set_hp(0, 0);
            fainted = true;
        }
        let asked_again = situations.lock().expect("not poisoned").len() > 1;
        let state = run.fixture().try_game_state();
        asked_again || state.is_ok_and(|state| state.map.map != crate::pokemon::map::Map::PalletTown && state.map.position_settled)
    });
    assert!(settled, "neither the door nor a second turn came of the chain");
    assert_eq!(fainted, refuse, "the driver was never seen running");
    let situations = situations.lock().expect("not poisoned").clone();
    let door = door.lock().expect("not poisoned").clone().unwrap_or_else(|| panic!("no chain was sent: {situations:#?}"));
    (run, situations, door)
}

#[test]
fn a_chain_stops_at_a_field_move_the_game_refuses() {
    let (mut run, situations, door) = softboiled_then_a_door(true, "chain-refused");
    assert_eq!(run.fixture().game_state().map.map, crate::pokemon::map::Map::PalletTown, "the door was walked through");
    let next = situations.get(1).unwrap_or_else(|| panic!("no turn after the refusal: {situations:#?}"));
    assert!(next.contains("`softboiled@0` was not done: Softboiled from party slot 1 healed nobody"), "{next}");
    assert!(next.contains(&format!("So `{door}`, which you had chained behind it, was not taken.")), "{next}");
}

#[test]
fn a_chain_goes_on_past_a_field_move_the_game_takes() {
    let (mut run, situations, _) = softboiled_then_a_door(false, "chain-taken");
    let state = run.fixture().game_state();
    assert_ne!(state.map.map, crate::pokemon::map::Map::PalletTown, "the door behind the heal was not walked: {situations:#?}");
    assert!(state.pokemon[0].current_hp > 1, "the heal never landed");
    assert_eq!(situations.len(), 1, "the chain asked again before reaching the door: {situations:#?}");
}
