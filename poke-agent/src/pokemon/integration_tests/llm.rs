//! **W4** — the LLM path end to end, against a mock OpenAI server in this process; and **C0**, the
//! seven faults that have actually ended deployed runs.
//!
//! Everything between the model and the game is the real thing: a real socket, a real
//! `text/event-stream` body, [`OpenAiClient`](crate::llm::client::OpenAiClient) parsing it, the real
//! worker, the real [`LlmPolicy`](crate::pokemon::llm_policy::LlmPolicy), the real agent, the real
//! emulator, and — since C0 — a real run directory too. Only the model is a stand-in.
//!
//! The assembly lives in [`llm_harness`], not here: this file is a *client* of it, and so is
//! everything `docs/coverage-plan.md` builds after it. What is left here is the tests.
//!
//! ⚠️ **A [`Brain`] is handed strings and nothing else**, and every test below has to find what it
//! needs in the rendered situation. See the harness's module note for why that property is worth
//! more than any test in this file.

use std::sync::Arc;
use std::time::Duration;

use crate::llm::map_image;
use crate::llm::screenshot::SCALE;
use crate::pokemon::integration_tests::llm_harness::{
    Always, Brain, Call, Fault, FaultThen, LlmRun, Reply, TurnRequest,
};
use crate::pokemon::map::Map;
use crate::pokemon::integration_tests::TestFixture;

/// Pallet Town, standing outside. Chosen for what it does **not** contain: no tall grass and no
/// scripted encounter, so the only thing that can move the player off this map is the decision the
/// model made.
///
/// ⚠️ `oaks-lab-just-got-squirtle.bin` was the obvious pick and is the wrong one — walking out of the
/// lab trips the rival battle, and a mock that has to *win a fight* to reach its assertion is testing
/// the RNG rather than the wire format.
const FIXTURE: &[u8] = include_bytes!("../data/pallet-town-state.bin");

/// How long a default-tier test will wait on the wall clock for something to happen. Generous: the
/// worker is a real thread talking over a real socket, and a machine under load is not a failure.
const PATIENCE: Duration = Duration::from_secs(30);

/// The stand-in model.
///
/// Two rules, both of which need the request to have been *correct* for a test to pass: on its first
/// turn it asks for `read_map` **and** a `screenshot` in one message — which only come back if the
/// batch round trip and the worker's own encoding both work — and after that it picks the warp out
/// of the menu it was sent, which only exists if the situation carried one.
fn plays_the_menu(request: &TurnRequest) -> Reply {
    // Compaction asks the same endpoint, with no tools and an instruction. A tool call here would
    // hang the compaction rather than fail it.
    if request.is_summary() {
        return Reply::Content("I am in Pallet Town and I am trying to leave it.".to_string());
    }
    if request.is_stuck() {
        // `why` is required of the model and read back out of the record it lands in — see
        // `the_watchdog_asks_the_model_for_a_nudge_and_delivers_it`.
        return Reply::call(
            "press_buttons",
            serde_json::json!({ "buttons": ["a"], "why": "the agent is wedged" }),
        );
    }
    let ids = request.menu_ids();
    if request.is_battle() {
        // Nothing should start a battle here, but a wild encounter is never impossible and a brain
        // that only knew how to run would hang the test in a trainer fight.
        let id = ids.iter().find(|id| id.starts_with("fight:")).cloned().unwrap_or_else(|| "run".into());
        return Reply::call("choose_battle_action", serde_json::json!({ "id": id }));
    }
    if request.seen == 0 {
        // Both in one assistant message: the read goes to the emulator thread and the screenshot is
        // answered by the worker, so this is the one request that exercises both paths at once.
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

/// **W4's acceptance, without an API key.** The model asks a read tool, is answered from the live
/// game, picks the warp out of the menu it was given, and the agent walks the player through it.
#[test]
fn the_llm_plays_from_a_fixture() {
    let mut run = LlmRun::builder(FIXTURE).named("llm-plays").start(Box::new(Menu));
    let left = run.tick_until(PATIENCE, |run| run.map() != Map::PalletTown);
    assert!(left, "the player never left Pallet Town — still at {}", run.fixture().game_state().map.player_position);

    // …and it got there having actually used a tool. Without this the test would still pass if the
    // batch round trip silently answered nothing, because the second turn does not need the answer.
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
    // ⚠️ The grid and its legend were *replaced* by the picture, not supplemented — a model given
    // both would be reading the same map twice, in two coordinate systems, for twice the tokens.
    assert!(!read.contains("\"grid\"") && !read.contains("\"legend\""),
            "read_map is still shipping the ASCII grid: {read:.200}");

    // **W5** — and both pictures from that assistant message came back too, encoded by the worker
    // and carried to the endpoint in the multi-part content form. This is the only test in which
    // that form goes through the real client, so it is the only place a PNG the endpoint would have
    // accepted is actually proved to be one.
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

    // The map of Pallet Town, at one pixel per game pixel plus the coordinate ruler. ⚠️ `high` —
    // the flat `low` price is a lie for a picture this size, and one 512x512 tile would squash the
    // whole town into mush.
    let map = (
        (map_image::RULER_LEFT + 10 * 2 * map_image::CELL_PX) as u32,
        (map_image::RULER_TOP + (9 * 2 + 2) * map_image::CELL_PX) as u32,
    );
    assert!(decoded.iter().any(|(size, detail)| *size == map && detail == "high"),
            "no `detail: high` map at {map:?} among {decoded:?}");
}

/// **The run directory is written as it is in deployment, and a restart resumes on it.**
///
/// ⚠️ **The one seam the pre-C0 test omitted entirely.** `history.json`, `conversation.jsonl`,
/// `todo.json` and `battle-script.json` are all written by the real code paths here, and the restart
/// below is the only place `GB_RESTORE_HISTORY`, the re-minted system prompt and
/// [`prompt::RESUMED_NOTE`](crate::llm::prompt::RESUMED_NOTE) are ever exercised end to end.
#[test]
fn a_restart_resumes_the_conversation_as_well_as_the_game() {
    let mut run = LlmRun::builder(FIXTURE).named("llm-restart").start(Box::new(Menu));
    assert!(run.tick_until_requests(3, PATIENCE), "the run never got going");

    let before = run.saved_history();
    let stored = before["messages"].as_array().expect("history.json holds messages").len();
    assert!(stored > 0, "a turn completed and nothing was written down: {before}");
    // ⚠️ The system prompt is never stored — it is re-minted from the build that is running — so a
    // deployment that edits it gets the edit rather than a copy pinned to the last process.
    assert!(
        !before["messages"].as_array().unwrap().iter().any(|m| m["role"] == "system"),
        "message 0 must not be stored: {before}",
    );
    // ⚠️ Only the two the conversation owns. `todo.json` and `battle-script.json` are written when
    // the model first touches them, and this brain touches neither — asserting on them here would be
    // asserting that the *defaults* get files, which is not something anything relies on.
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
    // The conversation came back: the first request of the second process carries the messages the
    // first one wrote, plus the note that says the game may be a little behind them.
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

/// **W9's acceptance (§14): fires on a deliberately jammed agent.**
///
/// The whole chain, and every link of it is the real thing except the model: the agent notices it
/// has asked nothing for the timeout, raises a `Stuck` turn, the worker sends it over a socket with
/// only `press_buttons` and `wait` to end it, the endpoint answers with a press, and the press
/// arrives back through `take_manual_input` and is delivered to the joypad.
///
/// ⚠️ **The timeout is one second here, and that is what makes the "jam" happen at all.** Nothing in
/// this fixture is genuinely wedged — an ordinary walk is a multi-second stretch in which the agent
/// asks nothing, which is exactly what the watchdog measures. At the shipped default of 300 emulated
/// seconds it would never fire (`mechanics::ordinary_play_stays_far_inside_the_stuck_timeout`
/// measures the real headroom); what is under test here is the mechanism, not the threshold.
#[test]
fn the_watchdog_asks_the_model_for_a_nudge_and_delivers_it() {
    use crate::pokemon::agent::AgentEvent;
    use std::sync::Mutex;

    /// The stuck turn as the endpoint saw it: its situation and the terminal tools it was offered.
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

    // Scoped as §7.5 requires: the escape hatch and doing nothing, and nothing else. A menu tool
    // here would let a turn end in a decision the wedged agent cannot carry out.
    assert_eq!(terminals, vec!["press_buttons".to_string(), "wait".to_string()]);

    // And the turn says what is wrong in terms the model can act on — the agent's own state, and
    // that this is the agent's fault rather than a puzzle in the game.
    assert!(situation.contains("## Decision: the game is stuck"), "{situation:.300}");
    assert!(situation.contains("bug in the agent"), "{situation:.600}");

    assert!(reported, "a firing has to be reported — §14: every one of them is a bug report");
    assert!(delivered, "the model's press never reached the joypad");

    // And the press left a record: the reason the model gave, the screen at the time, and the
    // conversation that led to it. ⚠️ This is the only end-to-end proof of the wiring — the unit
    // tests in `llm::incident` never go through the worker, and `with_run` is one line to forget.
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

// ── C0 §2.2: the seven faults ────────────────────────────────────────────────────────────────────
//
// Not one of these had a test before, and every one of them is a way a deployed run has actually
// ended. What each asserts is *what the run does next*, because that is the only thing an operator
// ever sees.

/// ⛔ **The 402 death loop** — `docs/coverage-plan.md` §2.2.1, and the four criteria are its own.
///
/// Observed live on 2026-09-05: OpenRouter credit ran out, which presents as an **undated 402**
/// rather than as the dated 429 the park is built for. It is therefore an ordinary turn failure and
/// is retried at once, for ever — and the history *ratchets*, because a failed turn used to leave
/// behind both the situation it was rejected on and the plan message
/// [`sync_plan`](crate::llm::worker::Worker) had just appended in front of it. The run reached turn
/// 16 555 and a 403 300-token history against a 100 000-token limit, of which 1373 messages were
/// copies of the plan.
///
/// (a) the messages a failed turn appended are rolled back;
/// (b) a failed turn does not count towards `turns_since_plan`;
/// (d) **the history does not grow across N consecutive failures** — the one assertion that would
///     have caught it.
///
/// (c) is [`a_compaction_with_no_turn_to_drop_says_so`], which needs a full history rather than a
/// failing one.
#[test]
fn an_undated_hard_failure_does_not_ratchet_the_history() {
    /// More than [`PLAN_REFRESH_TURNS`](crate::llm::worker::PLAN_REFRESH_TURNS), twice over, so a
    /// periodic plan refresh falls due inside the failing stretch. That is what the deployed run's
    /// 1373 copies were made of, and a run of five failures would not reach it.
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

    // A 402 is not retryable, so one request is one turn: what follows is a picture of the
    // conversation across `FAILURES` consecutive failed turns.
    let sizes: Vec<usize> = run.endpoint.requests().iter().map(|r| r.messages.len()).collect();
    let plans: Vec<usize> = run
        .endpoint
        .requests()
        .iter()
        .map(|r| r.messages.iter().filter(|m| m.text.starts_with("## Your plan")).count())
        .collect();

    // (d) — one line, and it is the whole defect.
    assert_eq!(
        sizes.iter().max(), sizes.iter().min(),
        "the history grew across {FAILURES} consecutive failures: {sizes:?}",
    );
    // (a) and (b), said in the terms the deployed file was wrong in.
    assert!(
        plans.iter().all(|count| *count <= 1),
        "a failed turn left its plan message behind: {plans:?}",
    );

    // And the run is still playing rather than wedged: the failures resolve to a wait, the operator
    // is told, and the turn after the last one decides something.
    assert!(run.said("the turn could not be completed"), "a failing endpoint has to be visible");
    let before = run.decisions().len();
    assert!(
        run.tick_until(PATIENCE, |run| run.decisions().len() > before),
        "the run never recovered once the endpoint did",
    );
}

/// **A dated 429 parks the run**: the emulator stops, the cartridge's own clock stops with it, and
/// the same question is put again when the window reopens.
///
/// ⚠️ **The cartridge clock is the assertion that matters**, because it is what the leaderboard ranks
/// on. A park that stopped the requests and let the game run would hand a run a free hour.
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

    // Wait for the park to be entered, and read the clock at that moment.
    assert!(
        run.tick_until(PATIENCE, |run| matches!(run.published.run_status(), RunStatus::Throttled { .. })),
        "a dated rate limit did not park the run; status is {:?}",
        run.published.run_status(),
    );
    assert!(run.said("the endpoint's quota is spent"), "a park has to say so: {:?}", run.notices());
    let stopped_at = run.playtime_seconds();

    // Through the park: the driver honours `throttled_until` exactly as `host.rs` does.
    let parked_until = std::time::Instant::now() + Duration::from_millis(1500);
    while std::time::Instant::now() < parked_until {
        run.tick();
    }
    assert_eq!(run.playtime_seconds(), stopped_at, "the cartridge clock ran while the run was parked");

    // And it comes back on its own, with the same question.
    assert!(
        run.tick_until(PATIENCE, |run| !run.decisions().is_empty()),
        "the run never resumed after the quota window reopened",
    );
    assert!(run.said("the quota window reopened"), "{:?}", run.notices());
    // ⚠️ The cartridge clock counts whole seconds, so "it is moving again" needs more than one of
    // them — a resumed run that had ticked twice would read as still parked.
    assert!(
        run.tick_until(PATIENCE, |run| run.playtime_seconds() > stopped_at + 1),
        "the game never restarted: still {stopped_at}s on the cartridge clock",
    );
}

/// **An undated 429 is the ordinary transient one**: back off in seconds, do not park.
///
/// ⚠️ **The distinction is the whole of [`LlmError::RateLimited`](crate::llm::LlmError)** — a limit
/// with no stated reset is far more often a per-minute one than a daily cap, and parking a run for
/// twenty-five hours on one would be far worse than the four wasted requests.
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
    // ⚠️ The park is what must *not* have happened. Its notice and its status are both distinctive.
    assert!(!run.said("the endpoint's quota is spent"), "an undated 429 parked the run: {:?}", run.notices());
    assert!(run.published.throttled_until().is_none(), "an undated 429 stopped the emulator");
}

/// **A request the endpoint took and never answered ends the turn without a retry.**
///
/// ⚠️ **Not retried, deliberately** — see [`LlmError::Timeout`](crate::llm::LlmError). A connection
/// that never opened consumed no work at the far end; a request that was *accepted* is being worked
/// on, and on an endpoint that serves one at a time a retry queues behind the very request it is
/// replacing.
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
    // And the question it was asked on is not left in the conversation twice.
    assert!(
        run.endpoint.requests().windows(2).all(|pair| pair[1].messages.len() >= pair[0].messages.len()),
        "the history went backwards, which means a message was dropped that had been answered",
    );
}

/// **Arguments that are not JSON are refused, and the turn still ends in a decision.**
///
/// The model is told what was wrong and gets its remaining tool steps; the loop's own fallback ends
/// the turn if it cannot use them. What must never happen is a turn that simply does not finish.
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

/// **A body that stops part-way through a `data:` frame.** The stream is unparseable, the turn
/// fails, and the run carries on.
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

/// **A completion with neither content nor a tool call is nudged once, then the rule is enforced.**
///
/// §7.5's fallback. A model that replies twice with nothing gets a forced `wait` rather than a turn
/// that hangs — the run has to keep playing, and a stall here is invisible from outside.
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
    // The nudge went out before the rule was enforced, and it quoted the contract.
    let nudged = run
        .endpoint
        .requests()
        .iter()
        .any(|request| request.situation().contains("no tool call, so nothing happened"));
    assert!(nudged, "the model was never told what it was doing wrong");
}

/// ⛔ **§2.2.1 (c) — a compaction that can drop nothing must say so rather than report success.**
///
/// The deployed run's compaction fell all the way through to `trim_history`, which cuts only at turn
/// boundaries — and its history held no completed turn, only unanswered questions. It dropped
/// nothing and published `{"before":403300,"after":403300,"summarised":false}`, which reads exactly
/// like a compaction that worked.
///
/// Reproduced by making the window small enough that an ordinary conversation overruns it and the
/// summary itself fails, which is what a spent quota does to it.
#[test]
fn a_compaction_with_no_turn_to_drop_says_so() {
    /// A brain that plays normally until the history is being summarised, and then refuses — the
    /// shape of an endpoint whose credit has run out mid-run.
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

    // Small enough that the system prompt and two turns overrun it, so a compaction is due almost
    // immediately and every one of them has to fall through to the last resort.
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

    // The point: every compaction either reclaimed something or said out loud that it could not.
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

// ── The bundled strategy, against real battles ───────────────────────────────────────────────────

/// **What the other Pokémon did reaches the model.**
///
/// ⚠️ **It did not, for the whole life of the battle layer, and nothing noticed.** Across eleven
/// battle turns not one `AgentEvent::TextBox` was emitted while a battle was live, though the box
/// that *opens* a battle was captured and overworld boxes were captured normally. `TextBox` is the
/// only channel the enemy's turn has: `BattleActionStarted` is the **player's** intent and the enemy
/// never gets one, and `### On screen` is a rolling fragment read at the decision point, by which
/// time the battle menu is back. So the model could see the move it chose and the HP that resulted,
/// and never "ENEMY ODDISH used ABSORB!", "It's super effective!", "fainted" or "gained 198 EXP".
///
/// The cause is in `agent::reading_dialogue`'s ⚠️: `wTopMenuItemX/Y` linger, so for the whole of a
/// turn's resolution the agent believed a move list was open and the arm that handles one
/// deliberately did not read. Asserted on the game's own words rather than on an event count,
/// because the bug produced a healthy stream of *empty* boxes and `PokemonAgent::event` drops those.
///
/// ⚠️ **Here rather than in `mechanics.rs`** only because that file was being edited by someone else
/// at the time; it belongs beside the other battle-timing tests whenever it is safe to move it.
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

    // ⚠️ **And the move list must not bleed into it.** `wTextBoxID` flips to `MessageBox` before
    // `AutoBgMapTransfer` has cleared the list the player just chose from, so a read taken too early
    // prefixes every quoted line with the whole moveset. `reading_dialogue` waits out `confirming`
    // for exactly this; without it these lines open "TACKLE TAIL WHIP BUBBLE WATER GUN Celina …".
    for line in said.iter() {
        let listed = ["TACKLE", "TAIL WHIP", "BUBBLE", "WATER GUN"]
            .iter().filter(|name| line.contains(**name)).count();
        assert!(listed < 3, "the move list leaked into a message box: {line:?}");
    }
}
