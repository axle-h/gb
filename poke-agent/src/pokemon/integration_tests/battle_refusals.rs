use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::pokemon::integration_tests::llm_harness::{Brain, Call, LlmRun, Reply, TurnRequest};
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::options::GameOptions;

/// How long a test waits on the wall clock; a deadline, so a passing run spends none of it.
const PATIENCE: Duration = Duration::from_secs(120);

/// A wild battle on Route 6: Ivysaur lv29 against Oddish lv13, five Poké Balls and a bag of things
/// the game refuses.
const WILD: &[u8] = include_bytes!("../data/stall-battle-key-item.bin");

/// A trainer battle in Viridian Forest: Articuno against a Bug Catcher's Weedle lv7, eight Great
/// Balls in the bag.
const TRAINER: &[u8] = include_bytes!("../data/stall-battle-key-item-trainer.bin");

/// A Safari Zone battle against a Rhyhorn, with the menu cursor left on BAIT.
const SAFARI: &[u8] = include_bytes!("../data/stall-safari-menu.bin");

const VIRIDIAN: &[u8] = include_bytes!("../data/postgame-sold.bin");

// ── The brain ──

/// Everything a refusal test reads back out of a run, all strings the model was sent.
#[derive(Default)]
struct Seen {
    /// One entry per request that was a battle turn, in order: the rendered situation.
    battle_turns: Vec<String>,
    /// Every overworld turn's situation, in order.
    overworld_turns: Vec<String>,
    /// Every distinct `tool`-role message the endpoint has been sent, in the order first seen.
    tool_results: Vec<String>,
    /// Whether the watchdog ever asked for a nudge.
    was_stuck: bool,
}

impl Seen {
    /// Whether any battle turn's situation contains `needle`.
    fn battle_said(&self, needle: &str) -> bool {
        self.battle_turns.iter().any(|turn| turn.contains(needle))
    }

    /// Whether any tool result contains `needle`.
    fn told(&self, needle: &str) -> bool {
        self.tool_results.iter().any(|result| result.contains(needle))
    }

    /// The menu ids of the `nth` battle turn, or an empty list if it never came.
    fn battle_menu(&self, nth: usize) -> Vec<String> {
        self.battle_turns
            .get(nth)
            .map(|turn| {
                turn.lines()
                    .filter_map(|line| line.strip_prefix("- `"))
                    .filter_map(|line| line.split_once('`'))
                    .map(|(id, _)| id.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

struct Refuser {
    plan: VecDeque<Call>,
    seen: Arc<Mutex<Seen>>,
}

impl Brain for Refuser {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        {
            let mut seen = self.seen.lock().expect("not poisoned");
            for message in &request.messages {
                if message.role == "tool" && !seen.tool_results.contains(&message.text) {
                    seen.tool_results.push(message.text.clone());
                }
            }
            if request.is_battle() {
                seen.battle_turns.push(request.situation().to_string());
            } else if request.has_tool("choose_action") {
                seen.overworld_turns.push(request.situation().to_string());
            }
            if request.is_stuck() {
                seen.was_stuck = true;
            }
        }

        // A compaction has no tools, so a tool call here would hang it rather than fail it.
        if request.is_summary() {
            return Reply::Content("I am testing what happens when the game says no.".to_string());
        }
        if request.is_stuck() {
            return Reply::call(
                "press_buttons",
                serde_json::json!({ "buttons": ["b"], "why": "the agent is wedged" }),
            );
        }
        if request.has_tool("set_nickname") {
            return Reply::call("set_nickname", serde_json::json!({ "summary": "keeping the species name" }));
        }
        if request.is_battle() {
            if let Some(mut call) = self.plan.pop_front() {
                // `kind:*` is the first row of that kind, for a cell that does not care which.
                if let Some(kind) = call.arguments["id"].as_str().and_then(|id| id.strip_suffix('*')) {
                    if let Some(id) = request.menu_ids().into_iter().find(|id| id.starts_with(kind)) {
                        call.arguments["id"] = serde_json::json!(id);
                    }
                }
                return Reply::Calls(vec![call]);
            }
        }
        Reply::Calls(vec![Call::wait(30)])
    }
}

/// A terminal battle call, with the `summary` every one of them requires.
fn choose(id: &str) -> Call {
    Call::new(
        "choose_battle_action",
        serde_json::json!({ "id": id, "summary": format!("taking `{id}` to see what the game says") }),
    )
}

/// The assembled run, with the brain's log.
fn run_on(fixture: &'static [u8], name: &'static str, plan: Vec<Call>, options: GameOptions) -> (LlmRun, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = Refuser { plan: plan.into(), seen: Arc::clone(&seen) };
    let run = LlmRun::builder(fixture)
        .named(name)
        // Fifteen game-minutes for a battle that takes seconds.
        .game_time(Duration::from_secs(15 * 60))
        .options(options)
        .start(Box::new(brain));
    (run, seen)
}

/// How much of `item` the bag holds, by the same reader the turn's menu is built from.
fn held(run: &mut LlmRun, item: ItemId) -> u8 {
    run.fixture()
        .game_state()
        .bag
        .iter()
        .find(|entry| entry.id == item)
        .map_or(0, |entry| entry.quantity)
}

/// The cartridge's words for a ball that did not catch, `ItemUseBallText00` to `04`.
const BALL_FAILED: [&str; 5] =
    ["You missed", "Darn! The", "Aww! It", "Shoot! It", "It dodged"];

// ── The cells ──

in_both_animation_modes!(
    a_poke_ball_that_fails_hands_the_battle_back_rather_than_ending_it,
    a_run_the_game_refuses_hands_the_turn_back_rather_than_ending_the_battle,
    run_is_withheld_from_a_trainer_battle_and_asking_for_it_is_refused_in_the_turn,
    a_ball_thrown_at_a_trainer_is_blocked_and_the_ball_is_spent_saying_so,
    an_item_the_bag_has_run_out_of_leaves_the_menu_and_is_refused_by_name,
    a_key_item_the_game_refuses_in_a_battle_says_why_rather_than_going_quiet,
    the_last_safari_ball_ends_the_game_around_the_battle_it_was_thrown_in,
    a_ball_that_catches_ends_the_battle_with_the_catch_in_the_party,
    a_potion_in_battle_heals_the_active_pokemon,
    a_switch_puts_the_chosen_pokemon_out,
    a_faint_brings_the_next_pokemon_out,
    a_trainers_last_pokemon_ends_the_battle,
    a_run_that_works_ends_the_battle,
);

fn a_poke_ball_that_fails_hands_the_battle_back_rather_than_ending_it(options: GameOptions) {
    let (mut run, seen) = run_on(WILD, "refusal-ball", vec![choose("item:PokeBall")], options);
    run.fixture().api().debug_set_catch_rate(0);
    let before = held(&mut run, ItemId::PokeBall);
    assert_eq!(before, 5, "the committed fixture carries five Poké Balls");

    // Two battle turns: the throw, and the one that proves the battle is still on.
    let asked_twice = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns.len() >= 2
    });

    let seen = seen.lock().expect("not poisoned");
    assert!(
        asked_twice,
        "the model was asked {} battle turns after throwing a ball that failed; it should have been \
         asked again, because the battle is still there.\n  tool results: {:?}",
        seen.battle_turns.len(),
        seen.tool_results,
    );
    assert!(!seen.was_stuck, "the watchdog fired: a failed catch left the agent with nothing to do");
    assert_eq!(
        held(&mut run, ItemId::PokeBall), before - 1,
        "a thrown ball is spent whether it catches or not, and the bag has to say so",
    );
    assert!(
        BALL_FAILED.iter().any(|words| seen.battle_said(words)),
        "the model was never told the catch failed. `ItemUseBallText01`–`04` are the cartridge's own \
         four sentences for it and not one of them reached the turn.\n  turn 2 was:\n{}",
        seen.battle_turns.get(1).map_or("<never asked>", String::as_str),
    );
    // Still the same fight, so the sentence is actionable.
    assert!(seen.battle_said("Wild battle"), "the second turn was not a wild battle any more");
    assert!(seen.battle_said("Oddish"), "the second turn was about a different Pokémon");
    // The bag must not leak into it, as
    // `llm::what_the_enemy_did_is_reported_rather_than_only_what_we_did` guards the move list.
    for turn in seen.battle_turns.iter() {
        let quoted: Vec<&str> = turn.lines().filter(|line| line.starts_with("- Text:")).collect();
        for line in quoted {
            let listed = ["TOWN MAP", "HELIX FOSSIL", "S.S.TICKET", "NUGGET"]
                .iter().filter(|name| line.contains(**name)).count();
            assert!(listed < 2, "the battle bag list leaked into a quoted message: {line:?}");
        }
    }
}

/// A run the cartridge refuses hands the turn back, and the escape is never certain.
fn a_run_the_game_refuses_hands_the_turn_back_rather_than_ending_the_battle(options: GameOptions) {
    // Even at 1 against 255 the first try escapes on one random byte, so a lucky escape is re-run.
    for _ in 0..3 {
        let (mut run, seen) = run_on(WILD, "refusal-run", vec![choose("run"), choose("run")], options);
        let asked_twice = run.tick_until(PATIENCE, |run| {
            run.drain_events();
            // Held every tick: the cartridge recomputes the speeds after the fixture was cut.
            if run.fixture().game_state().battle.is_some() {
                run.fixture().api().debug_set_battle_speeds(1, 255);
            }
            seen.lock().expect("not poisoned").battle_turns.len() >= 2
                || run.fixture().game_state().battle.is_none()
        });
        if run.fixture().game_state().battle.is_none() {
            continue;
        }

        let seen = seen.lock().expect("not poisoned");
        assert!(
            asked_twice,
            "the model was asked {} battle turns after a failed escape; the fight is still on, so it \
             should have been asked again.\n  tool results: {:?}",
            seen.battle_turns.len(),
            seen.tool_results,
        );
        assert!(!seen.was_stuck, "the watchdog fired: a failed escape left the agent with nothing to do");
        assert!(
            seen.battle_said("Can't escape"),
            "the model was never told the escape failed — `CantEscapeText` is the cartridge's own \
             sentence and it did not reach the turn.\n  turn 2 was:\n{}",
            seen.battle_turns.get(1).map_or("<never asked>", String::as_str),
        );
        assert!(seen.battle_said("Oddish"), "the second turn was about a different Pokémon");
        return;
    }
    panic!("three escapes at 1 speed against 255 all worked, which the odds do not allow");
}

/// `run` is not on a trainer battle's menu, and asking for it is answered inside the turn.
fn run_is_withheld_from_a_trainer_battle_and_asking_for_it_is_refused_in_the_turn(options: GameOptions) {
    let (mut run, seen) =
        run_on(TRAINER, "refusal-trainer-run", vec![choose("run"), choose("fight:Ice Beam")], options);

    let decided = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        !run.decisions().is_empty()
    });

    let seen = seen.lock().expect("not poisoned");
    let menu = seen.battle_menu(0);
    assert!(!menu.is_empty(), "the model was never asked a battle turn at all");
    assert!(
        !menu.contains(&"run".to_string()),
        "`run` was offered in a trainer battle. The cartridge refuses it with a message and drops \
         back to the same menu, which is a closed loop under A.\n  menu: {menu:?}",
    );
    assert!(
        seen.told("`run`"),
        "asking for `run` was not answered with anything naming it, so the model has no way to \
         learn the id was never on offer.\n  tool results: {:?}",
        seen.tool_results,
    );
    assert!(
        seen.told("no running from a trainer battle"),
        "the refusal did not say why. A model that can see RUN on the screen and is told only that \
         the id is unavailable has been handed a contradiction, which is how the ViridianGym and \
         Route 22 issue reports happened.\n  tool results: {:?}",
        seen.tool_results,
    );
    assert!(
        seen.battle_turns.len() >= 2,
        "the refusal ended the turn instead of handing it back — the model was asked {} times and a \
         rejected id has to be followed by another go at the same question",
        seen.battle_turns.len(),
    );
    assert!(decided, "the turn never reached a decision after the refusal");
    assert!(!seen.was_stuck, "the watchdog fired after a refused battle id");
}

/// A ball thrown at a trainer's Pokémon is blocked, costs the ball, and the model is told both.
fn a_ball_thrown_at_a_trainer_is_blocked_and_the_ball_is_spent_saying_so(options: GameOptions) {
    let (mut run, seen) =
        run_on(TRAINER, "refusal-trainer-ball", vec![choose("item:GreatBall")], options);
    let before = held(&mut run, ItemId::GreatBall);
    assert_eq!(before, 8, "the committed fixture carries eight Great Balls");

    let asked_twice = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns.len() >= 2
    });

    let seen = seen.lock().expect("not poisoned");
    assert!(
        asked_twice,
        "the model was asked {} battle turns after a blocked ball; the trainer fight is still on.\n  \
         tool results: {:?}",
        seen.battle_turns.len(),
        seen.tool_results,
    );
    assert!(!seen.was_stuck, "the watchdog fired: a blocked ball left the agent with nothing to do");
    assert_eq!(
        held(&mut run, ItemId::GreatBall), before - 1,
        "`ThrowBallAtTrainerMon` falls through to `RemoveUsedItem`, so the ball is spent and the \
         bag has to say so — a refusal that charges for itself is one a model must be able to see",
    );
    assert!(
        seen.battle_said("The trainer blocked the") || seen.battle_said("thief"),
        "the model was never told the trainer blocked it. Both of the cartridge's sentences are \
         missing from the turn.\n  turn 2 was:\n{}",
        seen.battle_turns.get(1).map_or("<never asked>", String::as_str),
    );
}

/// An item the bag has run out of stops being a row, and asking for it is refused by name.
fn an_item_the_bag_has_run_out_of_leaves_the_menu_and_is_refused_by_name(options: GameOptions) {
    let (mut run, seen) = run_on(
        WILD,
        "refusal-item-gone",
        vec![choose("item:PokeBall"), choose("item:PokeBall"), choose("run")],
        options,
    );
    run.fixture().api().debug_set_catch_rate(0);
    // One ball, so the throw below is the last.
    run.fixture().api().debug_take_item(ItemId::PokeBall).expect("the fixture holds Poké Balls");
    run.fixture().api().debug_give_item(ItemId::PokeBall, 1).expect("a bag with a slot free");

    let asked = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns.len() >= 3
    });

    let seen = seen.lock().expect("not poisoned");
    assert!(asked, "the model was asked {} battle turns", seen.battle_turns.len());
    assert!(!seen.was_stuck, "the watchdog fired after the bag ran out of Poké Balls");
    assert_eq!(held(&mut run, ItemId::PokeBall), 0, "the last ball should have been thrown");
    let second = seen.battle_menu(1);
    assert!(!second.is_empty(), "the second battle turn carried no menu");
    assert!(
        !second.contains(&"item:PokeBall".to_string()),
        "`item:PokeBall` was still on the menu with none left in the bag.\n  menu: {second:?}",
    );
    assert!(
        seen.told("`item:PokeBall`"),
        "asking for the ball that had run out was not answered with anything naming it.\n  tool \
         results: {:?}",
        seen.tool_results,
    );
    assert!(
        seen.told("not in the bag"),
        "the refusal did not say the bag had run out — which is the one thing that changed between \
         the two turns.\n  tool results: {:?}",
        seen.tool_results,
    );
    assert!(
        !seen.told("ids are minted for the map"),
        "the refusal talked about map ids in a battle.\n  tool results: {:?}",
        seen.tool_results,
    );
}

/// A key item the game refuses in a battle says so, rather than costing a turn in silence.
fn a_key_item_the_game_refuses_in_a_battle_says_why_rather_than_going_quiet(options: GameOptions) {
    let (mut run, seen) = run_on(WILD, "refusal-key-item", vec![choose("item:SSTicket")], options);
    let before = held(&mut run, ItemId::SSTicket);
    assert_eq!(before, 1, "the committed fixture carries the S.S. Ticket");

    let asked_twice = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns.len() >= 2
    });

    let seen = seen.lock().expect("not poisoned");
    assert!(
        asked_twice,
        "the model was asked {} battle turns after an item the game refused.\n  tool results: {:?}",
        seen.battle_turns.len(),
        seen.tool_results,
    );
    assert!(!seen.was_stuck, "the watchdog fired: the refused key item wedged the bag list");
    assert_eq!(held(&mut run, ItemId::SSTicket), before, "`ItemUseNotTime` consumes nothing");
    assert!(
        seen.battle_said("This isn't the ti"),
        "the model was never told why the S.S. Ticket did nothing. `ItemUseNotTime` says it out \\
         loud, and a turn spent on an action with no account of it is the silence this file is \\
         about.\\n  turn 2 was:\\n{}",
        seen.battle_turns.get(1).map_or("<never asked>", String::as_str),
    );
}

/// Ticks `run` until `seen` holds an overworld turn, the proof that a battle handed the run back.
fn played_on(run: &mut LlmRun, seen: &Arc<Mutex<Seen>>) -> bool {
    run.tick_until(PATIENCE, |run| {
        run.drain_events();
        !seen.lock().expect("not poisoned").overworld_turns.is_empty()
    })
}

/// A ball that catches ends the battle, and the catch joins the party.
fn a_ball_that_catches_ends_the_battle_with_the_catch_in_the_party(options: GameOptions) {
    let (mut run, seen) = run_on(WILD, "battle-catch", vec![choose("item:MasterBall")], options);
    run.fixture().api().debug_give_item(ItemId::MasterBall, 1).expect("a bag with a slot free");

    assert!(played_on(&mut run, &seen), "no overworld turn followed the catch");
    let state = run.fixture().game_state();
    assert!(state.battle.is_none(), "still in the battle");
    assert_eq!(state.pokemon.len(), 3, "the Oddish did not join the party");
    assert!(!seen.lock().expect("not poisoned").was_stuck, "the watchdog fired around a catch");
}

/// A Potion in battle heals the Pokémon that is out and costs the turn, not the battle.
fn a_potion_in_battle_heals_the_active_pokemon(options: GameOptions) {
    let (mut run, seen) = run_on(WILD, "battle-potion", vec![choose("item:Potion")], options);
    run.fixture().api().debug_give_item(ItemId::Potion, 1).expect("a bag with a slot free");
    // Out of reach, so the enemy's reply cannot undo the heal before it is read.
    run.fixture().api().debug_set_hp(0, 20);

    let asked_twice = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns.len() >= 2
    });
    assert!(asked_twice, "no second battle turn after the Potion");
    assert_eq!(held(&mut run, ItemId::Potion), 0, "the Potion was not used");
    // A resisted Absorb from a lv13 Oddish cannot take back what a Potion gives.
    assert!(run.fixture().game_state().pokemon[0].current_hp > 20, "the Ivysaur was not healed");
}

/// A switch puts the chosen Pokémon out and asks again.
fn a_switch_puts_the_chosen_pokemon_out(options: GameOptions) {
    let (mut run, seen) = run_on(WILD, "battle-switch", vec![choose("switch:1")], options);

    let asked_twice = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns.len() >= 2
    });
    assert!(asked_twice, "no second battle turn after the switch");
    let out = run.fixture().game_state().battle.map(|battle| battle.active_party_slot);
    assert_eq!(out, Some(1), "the Pidgey is not the one out");
}

/// A Pokémon that faints is replaced by the next one standing, and the battle goes on.
fn a_faint_brings_the_next_pokemon_out(options: GameOptions) {
    let (mut run, seen) = run_on(WILD, "battle-faint", vec![choose("fight:*")], options);
    run.fixture().api().debug_set_hp(0, 1);

    let asked_twice = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        // The Oddish moves first, and one hit of anything is the faint.
        if run.fixture().game_state().pokemon[0].current_hp > 0 {
            run.fixture().api().debug_set_battle_speeds(1, 255);
        }
        seen.lock().expect("not poisoned").battle_turns.len() >= 2
    });
    let seen = seen.lock().expect("not poisoned");
    assert!(asked_twice, "no battle turn after the faint.\n  tool results: {:?}", seen.tool_results);
    let state = run.fixture().game_state();
    assert_eq!(state.pokemon[0].current_hp, 0, "the Ivysaur never fainted, so this proves nothing");
    assert_eq!(state.battle.map(|battle| battle.active_party_slot), Some(1), "the Pidgey was not sent out");
    assert!(!seen.was_stuck, "the watchdog fired around a forced switch");
}

/// A trainer's last Pokémon fainting ends the battle and hands the run back.
fn a_trainers_last_pokemon_ends_the_battle(options: GameOptions) {
    let (mut run, seen) = run_on(TRAINER, "battle-trainer-won", vec![choose("fight:Ice Beam"); 6], options);

    assert!(played_on(&mut run, &seen), "no overworld turn followed the trainer battle");
    let seen = seen.lock().expect("not poisoned");
    assert!(seen.battle_turns.len() >= 2, "one battle turn for a trainer with more than one Pokémon");
    assert!(run.fixture().game_state().battle.is_none(), "still in the trainer battle");
    assert!(!seen.was_stuck, "the watchdog fired around the end of a trainer battle");
}

/// SHIFT, which only a human or an old save leaves set: the agent declines every switch offer.
#[test]
fn a_trainer_battle_on_shift_declines_the_switch_it_is_offered() {
    use crate::pokemon::options::{BattleStyle, SERVED_OPTIONS};
    a_trainers_last_pokemon_ends_the_battle(GameOptions { battle_style: BattleStyle::Shift, ..SERVED_OPTIONS });
}

/// A run the cartridge allows ends the battle.
fn a_run_that_works_ends_the_battle(options: GameOptions) {
    let (mut run, seen) = run_on(WILD, "battle-run", vec![choose("run")], options);

    let escaped = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        // Held every tick, as in the refused escape above.
        if run.fixture().game_state().battle.is_some() {
            run.fixture().api().debug_set_battle_speeds(255, 1);
        }
        !seen.lock().expect("not poisoned").overworld_turns.is_empty()
    });
    assert!(escaped, "no overworld turn followed the escape");
    assert!(run.fixture().game_state().battle.is_none(), "still in the battle");
}

/// The last Safari Ball ends the game, and the run comes out of it at the gate.
fn the_last_safari_ball_ends_the_game_around_the_battle_it_was_thrown_in(options: GameOptions) {
    let (mut run, seen) = run_on(SAFARI, "refusal-safari", vec![choose("ball"), choose("ball")], options);
    run.fixture().api().debug_set_safari_balls(1);

    let ejected = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        run.map() == Map::SafariZoneGate
    });

    {
        let log = seen.lock().expect("not poisoned");
        assert!(
            ejected,
            "the Safari game did not end after its last ball was thrown.\n  battle turns: {}\n  \
             tool results: {:?}",
            log.battle_turns.len(),
            log.tool_results,
        );
        assert!(!log.was_stuck, "the watchdog fired on the way out of the Safari Zone");
        assert!(
            log.battle_menu(0).contains(&"ball".to_string()),
            "the Safari menu was never put to the model: {:?}",
            log.battle_menu(0),
        );
    }

    // The run carries on: a terminus that leaves the model nothing to do is the failure.
    let playing_on = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        !seen.lock().expect("not poisoned").overworld_turns.is_empty()
    });
    assert!(playing_on, "no overworld turn followed the ejection from the Safari Zone");
}

/// The old man's catching tutorial is a battle the agent cannot walk into.
#[test]
fn the_old_mans_tutorial_is_a_battle_the_agent_cannot_walk_into() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = Arc::clone(&seen);
    let brain = move |request: &TurnRequest| -> Reply {
        {
            let mut seen = log.lock().expect("not poisoned");
            if request.is_battle() {
                seen.battle_turns.push(request.situation().to_string());
            } else if request.has_tool("choose_action") {
                seen.overworld_turns.push(request.situation().to_string());
            }
            if request.is_stuck() {
                seen.was_stuck = true;
            }
        }
        if request.is_summary() {
            return Reply::Content("I am talking to an old man in Viridian City.".to_string());
        }
        if request.is_stuck() {
            return Reply::call(
                "press_buttons",
                serde_json::json!({ "buttons": ["b"], "why": "the agent is wedged" }),
            );
        }
        // The id, not the prose.
        match request.menu_ids().into_iter().find(|id| id == "ViridianCity:OldMan") {
            Some(id) => Reply::call(
                "choose_action",
                serde_json::json!({ "id": id, "summary": "asking the old man about catching" }),
            ),
            None => Reply::Calls(vec![Call::wait(30)]),
        }
    };

    let mut run = LlmRun::builder(VIRIDIAN)
        .named("refusal-old-man")
        // Same margin as [`run_on`]'s.
        .game_time(Duration::from_secs(15 * 60))
        .start(Box::new(brain));

    // Read off the model's own turn rather than off the notices.
    let talked = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock()
            .expect("not poisoned")
            .overworld_turns
            .iter()
            .any(|turn| turn.contains("Time is money"))
    });

    let seen = seen.lock().expect("not poisoned");
    assert!(
        talked,
        "the old man was never talked to, so this test proves nothing. The rows on offer were:\n{}",
        seen.overworld_turns.last().map_or("<no overworld turn>", String::as_str),
    );
    assert!(
        seen.battle_turns.is_empty(),
        "a battle turn was put to the model in Viridian City. If that is the old man's tutorial, \
         `read_battle_state` is calling it `Wild` and the model is being asked a question it cannot \
         answer — see this test's doc comment.\n  first turn:\n{}",
        seen.battle_turns[0],
    );
    assert!(
        run.fixture().game_state().battle.is_none(),
        "the old man started a battle: the agent answered NO to \"Are you in a hurry?\"",
    );
    assert!(!seen.was_stuck, "the watchdog fired while talking to the old man");
}
