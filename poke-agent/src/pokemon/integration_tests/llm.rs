use std::sync::Arc;
use std::time::Duration;

use crate::llm::map_image;
use crate::llm::screenshot::SCALE;
use crate::pokemon::integration_tests::llm_harness::{
    Always, Brain, Call, Fault, FaultThen, LlmRun, Reply, TurnRequest,
};
use crate::pokemon::map::Map;
use crate::pokemon::integration_tests::TestFixture;

/// Pallet Town, standing outside.
const FIXTURE: &[u8] = include_bytes!("../data/pallet-town-state.bin");

/// How long a default-tier test waits on the wall clock.
const PATIENCE: Duration = Duration::from_secs(30);

/// The stand-in model.
fn plays_the_menu(request: &TurnRequest) -> Reply {
    // Compaction asks the same endpoint, with no tools and an instruction.
    if request.is_summary() {
        return Reply::Content("I am in Pallet Town and I am trying to leave it.".to_string());
    }
    if request.is_stuck() {
        // `why` is required and read back out of its record by
        // `the_watchdog_asks_the_model_for_a_nudge_and_delivers_it`.
        return Reply::call(
            "press_buttons",
            serde_json::json!({ "buttons": ["a"], "why": "the agent is wedged" }),
        );
    }
    let ids = request.menu_ids();
    if request.is_battle() {
        // A wild encounter is never impossible, and a brain that only ran would hang in a trainer
        // fight.
        let id = ids.iter().find(|id| id.starts_with("fight:")).cloned().unwrap_or_else(|| "run".into());
        return Reply::call("choose_battle_action", serde_json::json!({ "id": id }));
    }
    if request.seen == 0 {
        // Both in one message: the read goes to the emulator thread and the screenshot to the
        // worker.
        return Reply::Calls(vec![
            Call::new("read_map", serde_json::json!({})),
            Call::new("screenshot", serde_json::json!({})),
        ]);
    }
    match ids.iter().find(|id| id.ends_with(":Warp") || id.ends_with(":Connection")) {
        Some(id) => Reply::call("choose_action", serde_json::json!({ "id": id })),
        None => Reply::Calls(vec![Call::wait(1)]),
    }
}

/// The same, with a name, so a test can pass it as a `Box<dyn Brain>`.
struct Menu;

impl Brain for Menu {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        plays_the_menu(request)
    }
}

#[test]
fn the_llm_plays_from_a_fixture() {
    let mut run = LlmRun::builder(FIXTURE).named("llm-plays").start(Box::new(Menu));
    let left = run.tick_until(PATIENCE, |run| run.map() != Map::PalletTown);
    assert!(left, "the player never left Pallet Town — still at {}", run.fixture().game_state().map.player_position);

    // Having used a tool on the way.
    let read = run
        .endpoint
        .requests()
        .iter()
        .flat_map(|request| request.messages.clone())
        .find(|message| message.role == "tool" && message.text.contains("\"warps\""))
        .map(|message| message.text)
        .expect("`read_map` was never answered — the tool round trip did not complete");
    assert!(read.contains("\"PalletTown\""), "read_map answered from the wrong state: {read:.200}");
    assert!(read.contains("\"is_dark\""), "read_map lost its shape: {read:.200}");
    // The picture replaces the grid and legend, so the map is not read twice in two coordinate
    // systems.
    assert!(!read.contains("\"grid\"") && !read.contains("\"legend\""),
            "read_map is still shipping the ASCII grid: {read:.200}");

    // Both pictures came back, encoded by the worker in the multi-part content form.
    use image::GenericImageView;
    let mut pictures: Vec<(String, String)> =
        run.endpoint.requests().iter().flat_map(TurnRequest::images).collect();
    pictures.dedup();
    assert!(!pictures.is_empty(), "no picture reached the endpoint as an image part");
    let decoded: Vec<_> = pictures
        .iter()
        .map(|(url, detail)| {
            let payload = url
                .strip_prefix("data:image/png;base64,")
                .unwrap_or_else(|| panic!("not a PNG data URL: {url:.60}"));
            let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
                .expect("the worker encoded this");
            (image::load_from_memory(&png).expect("a PNG the worker produced").dimensions(),
             detail.clone())
        })
        .collect();

    let screen = ((gb::ppu::LCD_WIDTH * SCALE) as u32, (gb::ppu::LCD_HEIGHT * SCALE) as u32);
    assert!(decoded.iter().any(|(size, detail)| *size == screen && detail == "low"),
            "no `detail: low` screenshot at {screen:?} among {decoded:?}");

    // Pallet Town at one pixel per game pixel, plus the ruler.
    let map = (
        (map_image::RULER_LEFT + 10 * 2 * map_image::CELL_PX) as u32,
        (map_image::RULER_TOP + (9 * 2 + 2) * map_image::CELL_PX) as u32,
    );
    assert!(decoded.iter().any(|(size, detail)| *size == map && detail == "high"),
            "no `detail: high` map at {map:?} among {decoded:?}");
}

/// The run directory is written as it is in deployment, and a restart resumes on it.
#[test]
fn a_restart_resumes_the_conversation_as_well_as_the_game() {
    let mut run = LlmRun::builder(FIXTURE).named("llm-restart").start(Box::new(Menu));
    assert!(run.tick_until_requests(3, PATIENCE), "the run never got going");

    let before = run.saved_history();
    let stored = before["messages"].as_array().expect("history.json holds messages").len();
    assert!(stored > 0, "a turn completed and nothing was written down: {before}");
    // The system prompt is never stored but re-minted from the running build.
    assert!(
        !before["messages"].as_array().unwrap().iter().any(|m| m["role"] == "system"),
        "message 0 must not be stored: {before}",
    );
    assert!(run.run_dir.join(crate::run::files::CONVERSATION).exists(), "no conversation log");

    let directory = run.run_dir.clone();
    let served = run.endpoint.requests_served();
    run.restart();
    assert_eq!(run.run_dir, directory, "a restart must resume the run in place, not mint a new one");

    assert!(run.tick_until_requests(served + 1, PATIENCE), "the second process never asked anything");
    let resumed = run
        .endpoint
        .requests()
        .last()
        .cloned()
        .expect("a request after the restart");
    // The second process's first request carries the first's messages, plus a note that the game
    // may be behind them.
    assert!(
        resumed.messages.len() > stored,
        "the restarted process started from {} messages, not the {stored} it had written",
        resumed.messages.len(),
    );
    assert!(
        resumed.messages.iter().any(|m| m.text.contains(crate::llm::prompt::RESUMED_NOTE)),
        "a resumed conversation has to say so, once: {:?}",
        resumed.messages.iter().map(|m| m.role.clone()).collect::<Vec<_>>(),
    );
    assert_eq!(run.processes, 2);
}

#[test]
fn the_watchdog_asks_the_model_for_a_nudge_and_delivers_it() {
    use crate::pokemon::agent::AgentEvent;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Watch(Arc<Mutex<Option<(String, Vec<String>)>>>);

    impl Brain for Watch {
        fn respond(&mut self, request: &TurnRequest) -> Reply {
            if request.is_stuck() {
                let mut seen = self.0.lock().expect("not poisoned");
                if seen.is_none() {
                    let mut terminals: Vec<String> = request
                        .tool_names()
                        .into_iter()
                        .filter(|name| name.starts_with("press") || *name == "wait")
                        .map(str::to_string)
                        .collect();
                    terminals.sort();
                    *seen = Some((request.situation().to_string(), terminals));
                }
            }
            plays_the_menu(request)
        }
    }

    let seen = Arc::new(Mutex::new(None));
    let mut run = LlmRun::builder(FIXTURE)
        .named("watchdog")
        .stuck_timeout(Duration::from_secs(1))
        .start(Box::new(Watch(Arc::clone(&seen))));

    let mut reported = false;
    let mut delivered = false;
    run.tick_until(PATIENCE, |run| {
        reported |= run
            .fixture()
            .agent
            .drain_events()
            .iter()
            .any(|event| matches!(event, AgentEvent::WatchdogFired { .. }));
        delivered |= run.fixture().agent.manual_input_pending() > 0;
        reported && delivered && seen.lock().expect("not poisoned").is_some()
    });

    let stuck = seen.lock().expect("not poisoned").clone();
    let (situation, terminals) = stuck.expect("no stuck turn ever reached the endpoint");

    assert_eq!(terminals, vec!["press_buttons".to_string(), "wait".to_string()]);

    // The turn names the agent's own state and says the fault is the agent's, not a puzzle.
    assert!(situation.contains("## Decision: the game is stuck"), "{situation:.300}");
    assert!(situation.contains("bug in the agent"), "{situation:.600}");

    assert!(reported, "a firing has to be reported: every one of them is a bug report");
    assert!(delivered, "the model's press never reached the joypad");

    // The press left a record: the model's reason, the screen and the conversation.
    let records = run.run_dir.join(crate::run::files::PRESS_BUTTONS);
    let record = std::fs::read_dir(&records)
        .unwrap_or_else(|e| panic!("no records in {records:?}: {e}"))
        .next()
        .expect("a press was delivered, so a record must exist")
        .expect("a readable entry")
        .path();
    assert!(record.join("screen.png").exists(), "a record without its screen is half a record");
    let json = std::fs::read_to_string(record.join("incident.json")).expect("incident.json");
    assert!(json.contains("the agent is wedged"), "the model's reason is the point of it: {json:.400}");
    assert!(json.contains("\"kind\": \"stuck\""), "a sanctioned press has to be tellable apart");
    assert!(json.contains("## Decision: the game is stuck"), "the turn that asked is in the slice");
}

#[test]
fn an_undated_hard_failure_does_not_ratchet_the_history() {
    const FAILURES: usize = 25;

    let brain = FaultThen::new(
        Fault::Http { status: 402, message: "Prompt tokens limit exceeded: add more credits".into() },
        FAILURES,
        Reply::Calls(vec![Call::wait(1)]),
    );
    let served = Arc::clone(&brain.served);
    let mut run = LlmRun::builder(FIXTURE)
        .named("hard-402")
        .game_time(Duration::from_secs(600))
        .start(Box::new(brain));

    assert!(
        run.tick_until(PATIENCE, |_run| served.load(std::sync::atomic::Ordering::SeqCst) >= FAILURES),
        "only {} of {FAILURES} failures were served",
        served.load(std::sync::atomic::Ordering::SeqCst),
    );

    // A 402 is not retryable, so each request is one failed turn.
    let sizes: Vec<usize> = run.endpoint.requests().iter().map(|r| r.messages.len()).collect();
    let plans: Vec<usize> = run
        .endpoint
        .requests()
        .iter()
        .map(|r| r.messages.iter().filter(|m| m.text.starts_with("## Your plan")).count())
        .collect();

    // A failed turn leaves the history the size it found it.
    assert_eq!(
        sizes.iter().max(), sizes.iter().min(),
        "the history grew across {FAILURES} consecutive failures: {sizes:?}",
    );
    // And leaves at most one plan message.
    assert!(
        plans.iter().all(|count| *count <= 1),
        "a failed turn left its plan message behind: {plans:?}",
    );

    // The failures resolve to waits, the operator is told, and the next turn decides.
    assert!(run.said("the turn could not be completed"), "a failing endpoint has to be visible");
    let before = run.decisions().len();
    assert!(
        run.tick_until(PATIENCE, |run| run.decisions().len() > before),
        "the run never recovered once the endpoint did",
    );
}

/// A streak of undated refusals parks the run and stops the cartridge clock.
#[test]
fn a_streak_of_undated_hard_failures_parks_the_run_and_stops_the_cartridge_clock() {
    use crate::llm::worker::RefusalPark;
    use crate::published::RunStatus;

    let park = RefusalPark { after: 3, first: Duration::from_secs(2), max: Duration::from_secs(2) };
    let brain = FaultThen::new(
        Fault::Http { status: 402, message: "Prompt tokens limit exceeded: add more credits".into() },
        park.after as usize,
        Reply::Calls(vec![Call::wait(1)]),
    );
    let served = Arc::clone(&brain.served);
    let mut run = LlmRun::builder(FIXTURE).named("park-402").refusal_park(park).start(Box::new(brain));

    assert!(
        run.tick_until(PATIENCE, |run| matches!(run.published.run_status(), RunStatus::Throttled { .. })),
        "a streak of refusals did not park the run; status is {:?}",
        run.published.run_status(),
    );
    // Parked on the streak's last refusal, not before; the earlier ones are failed turns.
    assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), park.after as usize);
    let failed_turns = run.notices().iter().filter(|(_, m)| m.contains("the turn could not be completed")).count();
    assert_eq!(failed_turns, park.after as usize - 1, "{:?}", run.notices());
    assert!(run.said("the endpoint refused 3 requests in a row"), "a park has to say why: {:?}", run.notices());
    let stopped_at = run.playtime_seconds();
    // The failed turns before the park resolved to waits, which are decisions too.
    let decided = run.decisions().len();

    let parked_until = std::time::Instant::now() + Duration::from_millis(1500);
    while std::time::Instant::now() < parked_until {
        run.tick();
    }
    assert_eq!(run.playtime_seconds(), stopped_at, "the cartridge clock ran while the run was parked");

    assert!(
        run.tick_until(PATIENCE, |run| run.decisions().len() > decided),
        "the run never resumed after the park",
    );
    assert!(run.said("the pause is over"), "{:?}", run.notices());
    // The same question, because the world did not move.
    let requests = run.endpoint.requests();
    let (refused, answered) = (&requests[park.after as usize - 1], &requests[park.after as usize]);
    assert_eq!(refused.messages.len(), answered.messages.len(), "the parked turn was not re-asked as it stood");
    assert!(refused.messages.iter().zip(&answered.messages).all(|(a, b)| a.text == b.text));
}

/// A dated 429 parks the run, stops the cartridge clock, and asks again when the window reopens.
#[test]
fn a_dated_rate_limit_parks_the_run_and_stops_the_cartridge_clock() {
    use crate::published::RunStatus;

    let brain = FaultThen::new(
        Fault::RateLimited {
            retry_after: Some(Duration::from_secs(2)),
            message: "your daily quota is spent".into(),
        },
        1,
        Reply::Calls(vec![Call::wait(1)]),
    );
    let mut run = LlmRun::builder(FIXTURE).named("park-429").start(Box::new(brain));

    // Read the clock as the park is entered.
    assert!(
        run.tick_until(PATIENCE, |run| matches!(run.published.run_status(), RunStatus::Throttled { .. })),
        "a dated rate limit did not park the run; status is {:?}",
        run.published.run_status(),
    );
    assert!(run.said("the endpoint's quota is spent"), "a park has to say so: {:?}", run.notices());
    let stopped_at = run.playtime_seconds();

    // The driver honours `throttled_until` as `host.rs` does.
    let parked_until = std::time::Instant::now() + Duration::from_millis(1500);
    while std::time::Instant::now() < parked_until {
        run.tick();
    }
    assert_eq!(run.playtime_seconds(), stopped_at, "the cartridge clock ran while the run was parked");

    // It comes back on its own with the same question.
    assert!(
        run.tick_until(PATIENCE, |run| !run.decisions().is_empty()),
        "the run never resumed after the quota window reopened",
    );
    assert!(run.said("the quota window reopened"), "{:?}", run.notices());
    // The cartridge clock counts whole seconds, so moving again needs more than one.
    assert!(
        run.tick_until(PATIENCE, |run| run.playtime_seconds() > stopped_at + 1),
        "the game never restarted: still {stopped_at}s on the cartridge clock",
    );
}

/// An undated 429 is the ordinary transient one: back off in seconds, do not park.
#[test]
fn an_undated_rate_limit_is_backed_off_from_rather_than_parked() {
    let brain = FaultThen::new(
        Fault::RateLimited { retry_after: None, message: "slow down".into() },
        2,
        Reply::Calls(vec![Call::wait(1)]),
    );
    let served = Arc::clone(&brain.served);
    let mut run = LlmRun::builder(FIXTURE).named("backoff-429").start(Box::new(brain));

    assert!(
        run.tick_until(PATIENCE, |run| !run.decisions().is_empty()),
        "the run never got past two rate limits",
    );
    assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 2, "both limits were served");
    assert!(run.said("retrying in"), "a retry has to be visible: {:?}", run.notices());
    assert!(!run.said("the endpoint's quota is spent"), "an undated 429 parked the run: {:?}", run.notices());
    assert!(run.published.throttled_until().is_none(), "an undated 429 stopped the emulator");
}

/// A request the endpoint took and never answered ends the turn without a retry.
#[test]
fn a_timeout_ends_the_turn_without_retrying_it() {
    let brain = FaultThen::new(Fault::Timeout, 1, Reply::Calls(vec![Call::wait(1)]));
    let served = Arc::clone(&brain.served);
    let mut run = LlmRun::builder(FIXTURE)
        .named("timeout")
        .request_timeout(Duration::from_millis(400))
        .start(Box::new(brain));

    assert!(
        run.tick_until(PATIENCE, |run| !run.decisions().is_empty()),
        "the run never got past a timed-out request",
    );
    assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 1, "the timeout was served once");
    assert!(run.said("took the request"), "a timeout must not read as a broken connection: {:?}", run.notices());
    assert!(!run.said("retrying in"), "a timeout was retried: {:?}", run.notices());
    assert!(
        run.endpoint.requests().windows(2).all(|pair| pair[1].messages.len() >= pair[0].messages.len()),
        "the history went backwards, which means a message was dropped that had been answered",
    );
}

/// Arguments that are not JSON are refused, and the turn still ends in a decision.
#[test]
fn malformed_tool_arguments_do_not_stop_the_turn_ending() {
    let brain = FaultThen::new(Fault::MalformedToolArgs, 2, Reply::Calls(vec![Call::wait(1)]));
    let mut run = LlmRun::builder(FIXTURE).named("bad-args").start(Box::new(brain));
    assert!(
        run.tick_until(PATIENCE, |run| !run.decisions().is_empty()),
        "a turn whose arguments would not parse never ended: {:?}",
        run.notices(),
    );
}

/// A body that stops part-way through a `data:` frame fails the turn and the run carries on.
#[test]
fn a_truncated_stream_fails_the_turn_and_the_run_carries_on() {
    let brain = FaultThen::new(Fault::TruncatedStream, 1, Reply::Calls(vec![Call::wait(1)]));
    let mut run = LlmRun::builder(FIXTURE).named("truncated").start(Box::new(brain));
    assert!(
        run.tick_until(PATIENCE, |run| !run.decisions().is_empty()),
        "the run never got past a truncated stream: {:?}",
        run.notices(),
    );
    assert!(
        run.said("malformed") || run.said("the turn could not be completed"),
        "a truncated stream said nothing an operator could act on: {:?}",
        run.notices(),
    );
}

/// A completion with neither content nor a tool call is nudged once, then the rule is enforced.
#[test]
fn a_completion_with_nothing_in_it_is_nudged_then_forced() {
    let mut run = LlmRun::builder(FIXTURE)
        .named("empty-choice")
        .start(Box::new(Always(Reply::Fault(Fault::EmptyChoice))));
    assert!(
        run.tick_until(PATIENCE, |run| !run.decisions().is_empty()),
        "a model that says nothing at all hung the turn: {:?}",
        run.notices(),
    );
    assert!(
        run.decisions().iter().any(|decision| decision.starts_with("wait")),
        "the forced fallback is a wait: {:?}",
        run.decisions(),
    );
    // The nudge went out before the rule was enforced, quoting the contract.
    let nudged = run
        .endpoint
        .requests()
        .iter()
        .any(|request| request.situation().contains("no tool call, so nothing happened"));
    assert!(nudged, "the model was never told what it was doing wrong");
}

#[test]
fn a_compaction_with_no_turn_to_drop_says_so() {
    /// Plays normally until asked to summarise, then refuses, as an endpoint out of credit does.
    struct RefusesToSummarise;

    impl Brain for RefusesToSummarise {
        fn respond(&mut self, request: &TurnRequest) -> Reply {
            match request.is_summary() {
                true => Reply::Fault(Fault::Http {
                    status: 402,
                    message: "Prompt tokens limit exceeded".into(),
                }),
                false => plays_the_menu(request),
            }
        }
    }

    // Small enough that the prompt and two turns overrun it, so every compaction falls to the last
    // resort.
    let mut run = LlmRun::builder(FIXTURE)
        .named("dead-compaction")
        .context_limit(4_000)
        .compact_above(0.5)
        .start(Box::new(RefusesToSummarise));

    assert!(
        run.tick_until(PATIENCE, |run| !run.compactions().is_empty()),
        "no compaction ever fired: {:?}",
        run.notices(),
    );
    assert!(
        run.said("could not summarise the history"),
        "the summary was refused and nothing said so: {:?}",
        run.notices(),
    );

    // Every compaction reclaimed something or said it could not.
    for (before, after, summarised) in run.compactions() {
        if after < before || summarised {
            continue;
        }
        assert!(
            run.said("compaction reclaimed nothing") || run.said("dropped the"),
            "a compaction reported {before} → {after} and said nothing about it: {:?}",
            run.notices(),
        );
    }
}

// ── The bundled strategy, against real battles ──

/// What the other Pokémon did reaches the model.
#[test]
fn what_the_enemy_did_is_reported_rather_than_only_what_we_did() {
    use crate::pokemon::GameState;
    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::agent::AgentEvent;
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::policy::Policy;
    use crate::pokemon::world_graph::WorldGraph;
    use std::sync::Mutex;

    /// Always the first move, collecting every word the agent reports on the way.
    struct Probe { said: Arc<Mutex<Vec<String>>> }

    impl Policy for Probe {
        fn name(&self) -> &'static str { "scripted" }
        fn pick_overworld_action(&mut self, _: &GameState, _: &WorldGraph) -> Option<OverworldAction> { None }
        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            state.battle.as_ref().and_then(|b| b.player.moves[0])
                .map(|battle_move| BattleAction::Fight { slot: 0, battle_move })
        }
        fn on_event(&mut self, event: &AgentEvent) {
            if let AgentEvent::TextBox { message } = event {
                self.said.lock().expect("the log is never poisoned").push(message.clone());
            }
        }
    }

    let said = Arc::new(Mutex::new(Vec::new()));
    let mut fixture = TestFixture::with_policy(
        crate::pokemon::integration_tests::BATTLE_STATE,
        Duration::from_secs(120),
        Box::new(Probe { said: Arc::clone(&said) }),
    );

    let mut ticks = 0;
    while fixture.total_cycles < fixture.max_cycles {
        ticks += 1;
        fixture.step();
        if ticks > 50 && fixture.try_game_state().map_or(true, |s| s.battle.is_none()) {
            break;
        }
    }

    let said = said.lock().expect("the log is never poisoned");
    let all = said.join(" | ");
    assert!(
        said.iter().any(|line| line.to_uppercase().contains("ENEMY")),
        "nothing the enemy did was reported. What was: {all}",
    );
    assert!(said.iter().any(|line| line.contains("used")), "no move was named in the game's own words: {all}");

    // The move list must not bleed into it.
    for line in said.iter() {
        let listed = ["TACKLE", "TAIL WHIP", "BUBBLE", "WATER GUN"]
            .iter().filter(|name| line.contains(**name)).count();
        assert!(listed < 3, "the move list leaked into a message box: {line:?}");
    }
}
