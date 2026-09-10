//! **Step 7 of `docs/coverage-plan.md`** — the battle refusals, through `LlmPolicy` and the worker.
//!
//! The coverage walk scores overworld ids only: it takes rows off `MetaTileMap::actions()` and never
//! reaches a battle decision at all. So the battle matrix was audited by hand instead (the audit is
//! in git at `7343616`, `docs/coverage-plan.md` §6.0 of that draft), and what it found was that
//! seven cells existed nowhere in the suite and **every one of them is a refusal** — a ball that
//! fails, a run that fails, a run the cartridge will not allow, a ball thrown at a trainer, an item
//! the bag has run out of, the Safari game ending around a battle, and the old man's tutorial.
//!
//! That is where the findings in a battle were always going to be. Every deployed-run defect this
//! repo has fixed in the *overworld* was the same shape — the game refuses, the refusal is reported
//! as something else, and the model concludes the emulator is broken — and nothing had ever put that
//! question to a battle turn.
//!
//! ⚠️ **Through the whole stack, and that is the point of the file.** Every cell below is already
//! reachable under `DeterministicPolicy` or against a hand-built `GameState`; what was untested is
//! what a *model* is offered in each — the id, the row, whether the tool refuses it, and whether the
//! cartridge's own sentence reaches the next turn. So these run on [`LlmRun`]: a real socket, the
//! real worker, the real `LlmPolicy`, the real agent and the real emulator, with only the model
//! standing in.
//!
//! ⚠️ **A [`Brain`] is handed strings and nothing else**, so every assertion below about what the
//! model was told is read out of the rendered turn. If a refusal is not in there, a real model
//! cannot see it either.
//!
//! # Where the states come from
//!
//! All five fixtures are already committed, and four of them are `soak` jam states being read a
//! second time — a mid-battle save is expensive to cut and these are exactly the battles wanted:
//!
//! | Fixture | The battle | Cells |
//! |---|---|---|
//! | `stall-battle-key-item.bin` | wild Oddish, five Poké Balls in the bag | a ball that fails, an item run out of |
//! | `stall-battle-key-item-trainer.bin` | a Bug Catcher's Weedle, eight Great Balls | a run refused, a ball at a trainer |
//! | `stall-safari-menu.bin` | a Safari Rhyhorn | the Safari game ending around a battle |
//! | `viridian-city-north-of-bush.bin` | none — Viridian City, the old man awake | the old man's tutorial |
//!
//! ⚠️ **Two of them need a `debug_` write to make the outcome a fact rather than a roll**, and both
//! are argued on the primitive: [`PokemonApi::debug_set_catch_rate`] and
//! [`PokemonApi::debug_set_battle_speeds`]. Gen 1 has no state in which a ball certainly fails or an
//! escape certainly does, so the residue is named in the test that carries it.
//!
//! [`PokemonApi::debug_set_catch_rate`]: crate::pokemon::PokemonApi::debug_set_catch_rate
//! [`PokemonApi::debug_set_battle_speeds`]: crate::pokemon::PokemonApi::debug_set_battle_speeds

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::pokemon::integration_tests::llm_harness::{Brain, Call, LlmRun, Reply, TurnRequest};
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;

/// How long a test will wait on the wall clock. The same figure `llm.rs` uses and for the same
/// reason: the worker is a real thread on a real socket and a loaded machine is not a failure.
const PATIENCE: Duration = Duration::from_secs(30);

/// A wild battle on Route 6: Ivysaur lv29 against an Oddish lv13, five Poké Balls and a bag full of
/// things the game will refuse. Cut by `soak` seed 1 from `at-vermilion` at 372 s, where the S.S.
/// Ticket wedged the bag list — see `stalls::a_key_item_used_in_battle_does_not_trap_the_bag`.
const WILD: &[u8] = include_bytes!("../data/stall-battle-key-item.bin");

/// A trainer battle in Viridian Forest: an Articuno against a Bug Catcher's Weedle lv7, with eight
/// Great Balls in the bag. `soak` seed 1 from `postgame-pc-box` at 554 s — see
/// `stalls::a_key_item_used_against_a_trainer_does_not_trap_the_bag`.
///
/// ⚠️ **A trainer battle is the whole point of it**, so it must not be swapped for a wild one that
/// happens to be tidier: `battle_options` offers `Run` in one and not the other, which is the entire
/// subject of two of the tests below.
const TRAINER: &[u8] = include_bytes!("../data/stall-battle-key-item-trainer.bin");

/// A Safari Zone battle against a Rhyhorn, with the menu cursor left on BAIT. `soak` seed 1 — see
/// `stalls::a_safari_menu_cursor_left_on_bait_does_not_repeat_itself`.
const SAFARI: &[u8] = include_bytes!("../data/stall-safari-menu.bin");

/// Viridian City with the Pokédex long since collected, which is the state the old man is *awake*
/// in: `EVENT_GOT_POKEDEX` hides the "Old Man Sleepy" sprite that blocks the road north and shows
/// the one at (18, 6) who offers the catching tutorial. ⚠️ **A pre-Pokédex Viridian fixture is the
/// wrong one and looks right** — `viridian-city-north-of-bush.bin` has him lying there saying "You
/// can't go through here! This is private property!", which is a different script with no battle
/// anywhere in it.
const VIRIDIAN: &[u8] = include_bytes!("../data/postgame-sold.bin");

// ── The brain ────────────────────────────────────────────────────────────────────────────────────

/// Everything a refusal test reads back out of a run, all of it strings the model was actually sent.
#[derive(Default)]
struct Seen {
    /// One entry per request that was a battle turn, in order: the rendered situation.
    ///
    /// ⚠️ **Per request, not per turn.** A refused id is answered *inside* the turn with a tool
    /// result and another request, so the count here is what the model was asked rather than what it
    /// decided — which is exactly the distinction two of these tests are about.
    battle_turns: Vec<String>,
    /// Every overworld turn's situation, same rule.
    overworld_turns: Vec<String>,
    /// Every distinct `tool`-role message the endpoint has been sent, in the order first seen. A
    /// rejection lands here, which is the only place the model ever reads one.
    tool_results: Vec<String>,
    /// Whether the watchdog ever asked for a nudge. A refusal that wedges the agent shows up here
    /// and nowhere else, so every test asserts it stayed false.
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

/// Answers each battle turn with the next call in `plan`, and keeps a copy of everything it was
/// shown. When the plan runs out it stops deciding — `wait` rather than a fallback action, because a
/// test that has got what it came for should not go on playing the game.
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

        // A compaction asks the same endpoint with no tools at all; a tool call here would hang it
        // rather than fail it.
        if request.is_summary() {
            return Reply::Content("I am testing what happens when the game says no.".to_string());
        }
        if request.is_stuck() {
            return Reply::call(
                "press_buttons",
                serde_json::json!({ "buttons": ["b"], "why": "the agent is wedged" }),
            );
        }
        if request.is_battle() {
            if let Some(call) = self.plan.pop_front() {
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

/// The assembled run, with the brain's log. The caller does its `debug_` seeding on
/// `run.fixture().api()` before the first tick — nothing has been emulated yet at this point, so a
/// write here is a write to the committed state rather than to a game part-way through a turn.
fn run_on(fixture: &'static [u8], name: &'static str, plan: Vec<Call>) -> (LlmRun, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let brain = Refuser { plan: plan.into(), seen: Arc::clone(&seen) };
    let run = LlmRun::builder(fixture)
        .named(name)
        // Enough emulated time for a battle with a refusal in it and no more. A test that runs out
        // of game time reports whatever it has, which is what makes the assertions below readable.
        .game_time(Duration::from_secs(180))
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

/// The cartridge's own words for a ball that did not catch — `ItemUseBallText00`–`04`, whichever the
/// shake calculation lands on. Asserted as a set because which one it is depends on a roll the test
/// deliberately does not pin; that it is *one of them* is the whole claim.
///
/// ⚠️ **The opening of each sentence, not a phrase from the middle of one.** The reader samples the
/// screen once per agent tick and the driver dismisses the box a tick or two later, so a quoted line
/// arrives a character or two short of its end — measured over eight runs of the trainer-ball test as
/// "The trainer blocked the BA" and "…BAL", never the closing "BALL!". Matching from the front is
/// what makes the assertion about the sentence rather than about the sampling.
const BALL_FAILED: [&str; 5] =
    ["You missed", "Darn! The", "Aww! It", "Shoot! It", "It dodged"];

// ── The cells ────────────────────────────────────────────────────────────────────────────────────

/// **A Poké Ball that fails leaves the battle exactly where it was, and says so.**
///
/// The first of the seven cells the audit found nowhere. Every ball in the suite until now was one
/// that *worked* — `postgame::fishing::can_catch_a_magikarp_on_the_old_rod`, the three legendaries —
/// and a successful catch ends the battle, so the path where the model has to be asked again had
/// never been walked. What it has to hold: the throw is taken, the ball is spent, the battle is
/// still there, the model is asked a *second* battle turn, and the cartridge's own sentence about
/// what happened reaches that turn. The last of those is the one that matters — a model that reads
/// only "you used a Poké Ball" and then finds the same Oddish in front of it has no way to tell a
/// failed catch from a lost turn.
///
/// ⚠️ **One throw in about 720 catches anyway** and this test then fails on its second assertion
/// with the count intact; [`crate::pokemon::PokemonApi::debug_set_catch_rate`] has the arithmetic.
/// It is the smallest residue Gen 1 allows without an uncatchable fixture (the ghost Marowak on
/// Pokémon Tower 6F is the only truly deterministic one, and it costs a mid-battle cut of a scripted
/// fight to reach).
#[test]
fn a_poke_ball_that_fails_hands_the_battle_back_rather_than_ending_it() {
    let (mut run, seen) = run_on(WILD, "refusal-ball", vec![choose("item:PokeBall")]);
    run.fixture().api().debug_set_catch_rate(0);
    let before = held(&mut run, ItemId::PokeBall);
    assert_eq!(before, 5, "the committed fixture carries five Poké Balls");

    // Two battle turns: the one that throws, and the one that proves there is still a battle to be
    // asked about.
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
    // Still the same fight, which is what makes the sentence above actionable rather than alarming.
    assert!(seen.battle_said("Wild battle"), "the second turn was not a wild battle any more");
    assert!(seen.battle_said("Oddish"), "the second turn was about a different Pokémon");
    // ⚠️ **And the bag must not leak into it**, the same guard
    // `llm::what_the_enemy_did_is_reported_rather_than_only_what_we_did` puts on the move list. An
    // in-battle bag list is drawn in the very rows a `message_box_only` reader reads, so the
    // accumulate in `BattleState::UsingItem` is gated on the game actually talking; without that
    // gate every quoted line would open with the player's own inventory.
    for turn in seen.battle_turns.iter() {
        let quoted: Vec<&str> = turn.lines().filter(|line| line.starts_with("- Text:")).collect();
        for line in quoted {
            let listed = ["TOWN MAP", "HELIX FOSSIL", "S.S.TICKET", "NUGGET"]
                .iter().filter(|name| line.contains(**name)).count();
            assert!(listed < 2, "the battle bag list leaked into a quoted message: {line:?}");
        }
    }
}

/// **A run the cartridge refuses hands the turn back, and the escape is never certain.**
///
/// `TryRunningFromBattle` prints "Can't escape!" and **takes the player's turn** — the enemy attacks
/// and the menu comes back. Nothing in the suite had ever seen it, because every committed
/// mid-battle fixture has the player faster than the wild Pokémon and Gen 1 lets a faster player
/// leave unconditionally, so `run` had only ever been the action that *worked*.
///
/// ⚠️ **This is not the trainer refusal**, which is a different mechanism entirely and is the test
/// below: there the row is never offered. Here the row is offered, correctly, and the cartridge
/// declines anyway — so a model that reads "run" on the menu and takes it has to be told what
/// happened rather than left to infer it from finding itself still in the fight.
///
/// ⚠️ **One first attempt in 256 escapes** — see
/// [`crate::pokemon::PokemonApi::debug_set_battle_speeds`], and it is the cartridge making sure a
/// player can never be trapped rather than anything about this test.
#[test]
fn a_run_the_game_refuses_hands_the_turn_back_rather_than_ending_the_battle() {
    let (mut run, seen) = run_on(WILD, "refusal-run", vec![choose("run"), choose("run")]);
    run.fixture().api().debug_set_battle_speeds(1, 255);

    let asked_twice = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        seen.lock().expect("not poisoned").battle_turns.len() >= 2
    });

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
    assert!(seen.battle_said("Oddish"), "the battle ended after all: the escape worked");
}

/// **`run` is not on a trainer battle's menu, and asking for it is answered inside the turn.**
///
/// The cell the audit found covered only in the *sandbox*
/// (`battle_script::running_from_a_trainer_is_refused_with_the_reason`), which proves what a script
/// is told and nothing about what a model is. Two halves, and the second is the one worth having:
/// `battle_options` withholds the row — the standing rule that an action the game would refuse is
/// not offered at all — and a model that asks for it anyway gets a tool result and *another go at
/// the same turn*, rather than a wasted decision or a joypad press the game ignores.
///
/// ⚠️ **This is the last remaining way for the model to reach `NoRunningText`, and it must stay
/// unreachable.** The cartridge would answer a RUN here by printing "No! There's no running from a
/// trainer battle!" and dropping back to the same menu with the cursor where it was — the
/// closed-loop-under-A shape that has wedged this agent four separate times. The refusal above the
/// joypad is what keeps the agent away from it.
#[test]
fn run_is_withheld_from_a_trainer_battle_and_asking_for_it_is_refused_in_the_turn() {
    let (mut run, seen) =
        run_on(TRAINER, "refusal-trainer-run", vec![choose("run"), choose("fight:Ice Beam")]);

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
    // ⭐ **And it says *why*, which is the half step 7 added.** The turn's own `### On screen` line
    // reads `FIGHT Pokémon ITEM RUN`, and the system prompt forbids the model from filling the gap
    // out of what it knows about Pokémon Red — so "that id is not on the menu" against a screen
    // showing RUN is a contradiction with no way out of it. See `tools::battle_rule_behind`.
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

/// **A ball thrown at a trainer's Pokémon is blocked, costs the ball, and the model is told both.**
///
/// `ThrowBallAtTrainerMon` prints "The trainer blocked the BALL!" and "Don't be a thief!" and then
/// falls straight into `RemoveUsedItem` — so the throw is refused *and* the ball is gone, which is
/// the one refusal in the game that charges for itself. Nothing in the suite had ever thrown one.
///
/// ⚠️ **The row is offered and that is correct.** `battle_options` cannot withhold it: a ball is an
/// ordinary bag item, the bag list is what the game itself shows in a trainer fight, and the rule
/// that an action the game would refuse is not offered applies to actions the game refuses
/// *silently* — this one says what it is doing, out loud, twice. What has to hold is that the
/// sentence reaches the model, because eight Great Balls thrown at eight trainers is a real way for
/// a run to go quietly broke.
#[test]
fn a_ball_thrown_at_a_trainer_is_blocked_and_the_ball_is_spent_saying_so() {
    let (mut run, seen) =
        run_on(TRAINER, "refusal-trainer-ball", vec![choose("item:GreatBall")]);
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

/// **An item the bag has run out of stops being a row, and asking for it is refused by name.**
///
/// The bag is the one part of a battle menu that *changes underneath the model*: a ball thrown is a
/// row gone, and a model reading its own last turn back sees the id it just used and no reason to
/// think it has expired. So the last Poké Ball is thrown here, and the next turn is asked for the
/// same id.
///
/// ⚠️ **Zero-quantity entries are not the case being tested, because the cartridge cannot produce
/// one.** Gen 1 removes a bag entry outright when the last of it is used (`RemoveUsedItem`), so
/// `battle_options`' lack of a `quantity > 0` filter is not a live hole — what a model actually
/// meets is an id that has *left the menu*, which is this.
#[test]
fn an_item_the_bag_has_run_out_of_leaves_the_menu_and_is_refused_by_name() {
    let (mut run, seen) = run_on(
        WILD,
        "refusal-item-gone",
        vec![choose("item:PokeBall"), choose("item:PokeBall"), choose("run")],
    );
    run.fixture().api().debug_set_catch_rate(0);
    // One ball, so the throw below is the last of them. Taking the entry out and putting a single
    // one back is the only way to set a quantity: `debug_give_item` tops a stack up, never down.
    run.fixture().api().debug_take_item(ItemId::PokeBall).expect("the fixture holds Poké Balls");
    run.fixture().api().debug_give_item(ItemId::PokeBall, 1).expect("a bag with a slot free");

    // ⚠️ **Three requests, not two.** A rejection is answered *inside* the turn, so the count that
    // proves the model was told anything is the request after the one that was refused.
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
    // ⛔ **The sentence this cell was found by.** `not_on_the_menu` split the battle id on its colon
    // and reported `item` as a map and `fight` as the map the player was standing on: three false
    // statements about maps, on the commonest refusal there is in a battle. See the ⚠️ in
    // `tools::not_on_the_menu`.
    assert!(
        !seen.told("ids are minted for the map"),
        "the refusal talked about map ids in a battle.\n  tool results: {:?}",
        seen.tool_results,
    );
}

/// **A key item the game refuses in a battle says so, rather than costing a turn in silence.**
///
/// ⚠️ **Not one of the audit's seven, and here because the fix for two of them changed it.** This
/// cell was already covered — `stalls::a_key_item_used_in_battle_does_not_trap_the_bag` is the jam
/// this very fixture was cut in — but only as an *escape*: the agent got out of the bag list, and
/// nothing anywhere asked whether the model was ever told why its action did nothing. It was not.
/// `ItemUseNotTime` prints "OAK: <PLAYER>! This isn't the time to use that!" while
/// `BattleState::UsingItem` is in charge, and the refusal net dismissed it with a B and built a
/// fresh reader — so the sentence went the same way the ball's did.
///
/// It is worth its own test because it is the refusal a deployed run meets most: **every** bag in
/// the game holds something the cartridge will decline, and eleven of `soak`'s thirteen starting
/// states wedged on this one within six minutes of game time each.
#[test]
fn a_key_item_the_game_refuses_in_a_battle_says_why_rather_than_going_quiet() {
    let (mut run, seen) = run_on(WILD, "refusal-key-item", vec![choose("item:SSTicket")]);
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

/// **The last Safari Ball ends the game, and the run comes out of it at the gate.**
///
/// ⚠️ **The cell the audit asked for does not exist, and this is what does.** It wanted "the Safari
/// step counter running out mid-battle", and the counter cannot: `SafariZoneCheckSteps`
/// (`engine/events/hidden_events/safari_game.asm`) is called from the overworld's step block
/// *above* the `wIsInBattle` test and warps the player out on the very step that exhausts it —
/// before the encounter roll that step would otherwise make. So a battle and an expired step counter
/// never coexist.
///
/// What does end a Safari game from inside a fight is the **balls**: each throw decrements
/// `wNumSafariBalls` in `ItemUseBall`, and `SafariZoneCheck` at the top of `OverworldLoop` ends the
/// game the moment it sees zero. So the throw happens in a battle, the game-over happens on the way
/// out of it, and the player is warped to `SafariZoneGate` with a script waiting — which is a
/// terminus arriving *while the agent thinks it is in a battle*, and the only cell in this file that
/// takes the map out from under the model.
///
/// `postgame::safari::runs_the_step_budget_down_and_is_ejected` covers the ordinary ejection, from
/// the overworld, under `DeterministicPolicy`.
#[test]
fn the_last_safari_ball_ends_the_game_around_the_battle_it_was_thrown_in() {
    let (mut run, seen) = run_on(SAFARI, "refusal-safari", vec![choose("ball"), choose("ball")]);
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

    // And the run carries on: a terminus that leaves the model with nothing to do is the failure
    // this whole file is about. ⚠️ The lock is taken and released inside the predicate rather than
    // held across the loop — the brain is writing to it from the worker thread.
    let playing_on = run.tick_until(PATIENCE, |run| {
        run.drain_events();
        !seen.lock().expect("not poisoned").overworld_turns.is_empty()
    });
    assert!(playing_on, "no overworld turn followed the ejection from the Safari Zone");
}

/// **The old man's catching tutorial cannot be reached, and this is the reason.**
///
/// ⚠️ **The one cell in the audit that is answered by a row rather than by a test of it**, and the
/// row is checkable, which is why it is here. `wBattleType == 1` is a battle in which
/// `DisplayBattleMenu` (`engine/battle/core.asm`) **does not read the joypad at all** — the
/// cartridge walks its own cursor to ITEM and throws its own ball — and
/// [`crate::pokemon::battle::read_battle_state`] reports it as `BattleType::Wild`, so the agent
/// would reach a decision point and the model would be asked a question its answer could not
/// possibly affect.
///
/// It cannot happen. `ViridianCityOldManText` asks "Are you in a hurry?" and starts the tutorial on
/// **NO**; the agent answers every yes/no box in the game with A, which is YES. So the old man says
/// "Time is money... Go along then." and there is no battle. That is what this pins: the model picks
/// the old man off its own action menu, the conversation happens, and `wBattleType` never leaves 0.
///
/// ⚠️ **The claim is about the agent's yes/no rule, so it is the thing that would break it.** A
/// change that let the agent answer NO — to a nickname prompt, to "use next #MON?", to anything —
/// re-opens this cell, and then the reader has to name the tutorial and the agent has to refuse to
/// treat it as a decision point, in the same shape `battle::LOST_BATTLE` already has. The comment on
/// `open_menu_on_screen` is where the yes/no rule is argued; this is where it is paid for.
#[test]
fn the_old_mans_tutorial_is_a_battle_the_agent_cannot_walk_into() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = Arc::clone(&seen);
    // The overworld brain this cell needs is not the `Refuser`: it has to find the old man in the
    // action menu and talk to him, which is a row rather than a battle id.
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
        // ⚠️ **The id, not the prose.** "Old Man Sleepy" is a *different* sprite on this same map
        // and its row says "Old Man" too; a substring match on the description would talk to
        // whichever of them the fixture happens to be showing.
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
        .game_time(Duration::from_secs(180))
        .start(Box::new(brain));

    // ⚠️ **Read off the model's own turn rather than off the notices.** `LlmRun::said` reads
    // `Notice` events and an agent text box is not one; what settles this cell is that the *turn*
    // carries the old man's answer, which is the same string the assertions below are about.
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
