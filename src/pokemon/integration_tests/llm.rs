//! **W4** — the LLM path end to end, against a mock OpenAI server in this process.
//!
//! This is the test that stops the tool schema, the resolution logic and the agent's expectations
//! drifting apart, and it costs no API key and no network. Everything between the model and the game
//! is the real thing: a real socket, a real `text/event-stream` body, [`OpenAiClient`] parsing it,
//! the real worker, the real [`LlmPolicy`], the real agent, and the real emulator executing what
//! comes out. Only the model is a stand-in — a forty-line program that reads the menu it is sent and
//! picks the warp.
//!
//! ⚠️ **The mock fragments its tool-call arguments across several `data:` frames on purpose.** That is
//! the one thing about the wire format most likely to be got wrong (§7.1's ⚠️), and a mock that sends
//! a whole call in one frame would never catch it.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::post;

use crate::llm::client::OpenAiClient;
use crate::llm::map_image;
use crate::llm::screenshot::SCALE;
use crate::llm::config::LlmConfig;
use crate::llm::worker;
use crate::pokemon::integration_tests::fixture::TestFixture;
use crate::pokemon::llm_policy::LlmPolicy;
use crate::pokemon::map::Map;
use crate::web::published::Published;

/// Pallet Town, standing outside. Chosen for what it does **not** contain: no tall grass and no
/// scripted encounter, so the only thing that can move the player off this map is the decision the
/// model made.
///
/// ⚠️ `oaks-lab-just-got-squirtle.bin` was the obvious pick and is the wrong one — walking out of the
/// lab trips the rival battle, and a mock that has to *win a fight* to reach its assertion is testing
/// the RNG rather than the wire format.
const FIXTURE: &[u8] = include_bytes!("../data/pallet-town-state.bin");

#[derive(Clone, Default)]
struct Mock {
    turns: Arc<AtomicUsize>,
    /// What came back from `read_map`. `None` until the batch round trip has completed, which is the
    /// only way a `tool` message can appear in a request at all.
    map_result: Arc<std::sync::Mutex<Option<String>>>,
    /// **W5** — the `image_url` the `screenshot` answer put into the history, as the endpoint saw it.
    /// This is the only place the multi-part content form is exercised over a real socket.
    /// Every `(data_url, detail)` the endpoint was sent, in arrival order and deduplicated —
    /// a turn that reads the map and takes a screenshot sends two.
    picture: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    /// **W9** — the situation of the first watchdog turn, and the terminal tools it was sent with.
    /// `None` until one arrives, which for the test below is the whole question.
    stuck_turn: Arc<std::sync::Mutex<Option<(String, Vec<String>)>>>,
}

/// The stand-in model.
///
/// Two rules, both of which need the request to have been *correct* for the test to pass: on its
/// first overworld turn it asks for `read_map` **and** a `screenshot` in one message — which only
/// come back if the batch round trip and the worker's own encoding both work — and after that it
/// picks the warp out of the menu it was sent, which only exists if the situation carried one.
async fn completions(State(mock): State<Mock>, body: String) -> impl IntoResponse {
    let request: serde_json::Value = serde_json::from_str(&body).expect("the client sends JSON");
    let tools: Vec<&str> = request["tools"]
        .as_array()
        .expect("a request always carries tools")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default())
        .collect();
    let last_user = request["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default()
        .to_string();

    if let Some(result) = request["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|message| message["role"] == "tool")
        .and_then(|message| message["content"].as_str())
    {
        *mock.map_result.lock().expect("not poisoned") = Some(result.to_string());
    }

    // The picture arrives as a *user* message in the multi-part form, never on the tool result —
    // see `Message::user_with_image`. Finding it here is finding it exactly where an endpoint would.
    // ⚠️ **Every** image part, not the first. A turn that reads the map *and* takes a screenshot
    // sends two, and which one arrives first is the order the model called the tools in.
    for part in request["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .filter(|part| part["type"] == "image_url")
    {
        let url = part["image_url"]["url"].as_str().expect("a data URL").to_string();
        let detail = part["image_url"]["detail"].as_str().unwrap_or_default().to_string();
        let mut seen = mock.picture.lock().expect("not poisoned");
        if !seen.iter().any(|(existing, _)| *existing == url) {
            seen.push((url, detail));
        }
    }

    let turn = mock.turns.fetch_add(1, Ordering::SeqCst);
    let ids = menu_ids(&last_user);
    // **W9.** A stuck turn is recognisable from the `tools` array alone, which is the point of §7.5
    // scoping it: it is the only kind that can press buttons and has no menu tool to choose from.
    let is_stuck = tools.contains(&"press_buttons")
        && !tools.contains(&"choose_action")
        && !tools.contains(&"choose_battle_action");
    let calls = if is_stuck {
        let mut seen = mock.stuck_turn.lock().expect("not poisoned");
        if seen.is_none() {
            let terminals = tools.iter().filter(|name| **name != "screenshot")
                .filter(|name| name.starts_with("press") || **name == "wait")
                .map(|name| name.to_string())
                .collect();
            *seen = Some((last_user.clone(), terminals));
        }
        // `why` is required of the model and read back out of the record it lands in — see
        // `the_watchdog_asks_the_model_for_a_nudge_and_delivers_it`.
        vec![("press_buttons", serde_json::json!({ "buttons": ["a"], "why": "the agent is wedged" }))]
    } else if tools.contains(&"choose_battle_action") {
        // Nothing should start a battle here, but a wild encounter is never impossible and a mock
        // that only knows how to run would hang the test in a trainer fight.
        let id = ids.iter().find(|id| id.starts_with("fight:")).cloned().unwrap_or_else(|| "run".into());
        vec![("choose_battle_action", serde_json::json!({ "id": id }))]
    } else if turn == 0 {
        // Both in one assistant message: the read goes to the emulator thread and the screenshot is
        // answered by the worker, so this is the one request that exercises both paths at once.
        vec![("read_map", serde_json::json!({})), ("screenshot", serde_json::json!({}))]
    } else if let Some(id) = ids.iter().find(|id| id.ends_with(":Warp") || id.ends_with(":Connection")) {
        vec![("choose_action", serde_json::json!({ "id": id }))]
    } else {
        vec![("wait", serde_json::json!({ "ticks": 1 }))]
    };

    // ⚠️ **Every terminal call needs a `summary` and `tools::classify` refuses one without it**, so
    // it is added here rather than repeated in each arm above. A real model fills it in: across the
    // deployed run's 2427 decisions the only ones without were the synthesised fallback waits, which
    // never go through `classify`. A mock that omitted it would have every turn rejected and spend
    // its whole tool budget finding that out.
    let calls: Vec<(&str, String)> = calls
        .into_iter()
        .map(|(name, mut arguments)| {
            if let Some(object) = arguments.as_object_mut() {
                object.entry("summary").or_insert_with(|| serde_json::json!("what the mock is doing"));
            }
            (name, serde_json::to_string(&arguments).expect("valid JSON"))
        })
        .collect();
    ([(header::CONTENT_TYPE, "text/event-stream")], sse(&calls))
}

/// The ids the turn request offered, in order. Parsed out of the rendered menu exactly as a model
/// would have to.
fn menu_ids(situation: &str) -> Vec<String> {
    situation
        .lines()
        .filter_map(|line| line.strip_prefix("- `"))
        .filter_map(|line| line.split_once('`'))
        .map(|(id, _)| id.to_string())
        .collect()
}

/// One completion, as an OpenAI-compatible stream — with the arguments deliberately chopped into
/// three-character fragments, and every call's fragments interleaved with every other's, which is
/// what a parallel tool call actually looks like on the wire.
fn sse(calls: &[(&str, String)]) -> String {
    let mut out = String::new();
    let frame = |value: serde_json::Value| format!("data: {value}\n\n");

    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": { "role": "assistant", "content": "Let me look at where I am." } }]
    })));
    for (index, (name, _)) in calls.iter().enumerate() {
        out.push_str(&frame(serde_json::json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": index, "id": format!("call_mock_{index}"), "type": "function",
                "function": { "name": name, "arguments": "" },
            }] } }]
        })));
    }
    let fragments: Vec<Vec<&str>> = calls.iter().map(|(_, arguments)| chunks(arguments, 3)).collect();
    for step in 0..fragments.iter().map(Vec::len).max().unwrap_or(0) {
        for (index, call) in fragments.iter().enumerate() {
            let Some(fragment) = call.get(step) else { continue };
            out.push_str(&frame(serde_json::json!({
                "choices": [{ "delta": { "tool_calls": [{
                    "index": index, "function": { "arguments": fragment },
                }] } }]
            })));
        }
    }
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
    })));
    out.push_str(&frame(serde_json::json!({
        "choices": [], "usage": { "prompt_tokens": 1200, "completion_tokens": 40, "total_tokens": 1240 }
    })));
    out.push_str("data: [DONE]\n\n");
    out
}

fn chunks(text: &str, size: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let split = rest.char_indices().nth(size).map_or(rest.len(), |(i, _)| i);
        let (head, tail) = rest.split_at(split);
        out.push(head);
        rest = tail;
    }
    out
}

/// Start the mock on an arbitrary port and return the base URL. The runtime lives on its own thread
/// and is never joined — the test process ends and takes it with it.
fn serve_mock(mock: Mock) -> String {
    let (ready, address) = std::sync::mpsc::channel::<SocketAddr>();
    std::thread::Builder::new()
        .name("mock-openai".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime");
            runtime.block_on(async move {
                let app = Router::new()
                    .route("/v1/chat/completions", post(completions))
                    .with_state(mock);
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                    .await
                    .expect("an arbitrary loopback port");
                ready.send(listener.local_addr().expect("bound")).expect("the test is waiting");
                axum::serve(listener, app).await.expect("the mock serves");
            });
        })
        .expect("the mock server thread starts");

    let address = address.recv_timeout(Duration::from_secs(10)).expect("the mock server bound a port");
    format!("http://{address}/v1")
}

/// **W4's acceptance, without an API key.** The model asks a read tool, is answered from the live
/// game, picks the warp out of the menu it was given, and the agent walks the player through it.
#[test]
fn the_llm_plays_from_a_fixture() {
    let mock = Mock::default();
    let config = LlmConfig {
        base_url: serve_mock(mock.clone()),
        api_key: "mock".to_string(),
        model: "mock".to_string(),
        context_limit: 128_000,
        compact_above: crate::llm::config::DEFAULT_COMPACT_ABOVE,
        temperature: 1.0,
        max_tool_steps: 6,
        request_timeout: std::time::Duration::from_secs(crate::llm::config::DEFAULT_REQUEST_TIMEOUT_SECS),
        max_tokens: Some(crate::llm::config::DEFAULT_MAX_TOKENS),
        reasoning_effort: None,
        stuck_timeout: None,
    };
    let published = Published::new();
    let (worker, handles) =
        worker::channels(
            Box::new(OpenAiClient::new(&config)),
            config,
            Arc::clone(&published),
            crate::llm::todo::TodoList::open(None),
            crate::llm::battle_script::BattleScript::open(None),
            crate::llm::history::History::open(None),
        );
    let _worker = worker.spawn().expect("the worker thread starts");

    let mut fixture = TestFixture::with_policy(
        FIXTURE,
        Duration::from_secs(120),
        Box::new(LlmPolicy::new(handles, None)),
    );
    let arrived = fixture.try_run_until(|state| state.map.map != Map::PalletTown);

    let state = arrived.unwrap_or_else(|| {
        let state = fixture.game_state();
        panic!("the player never left Pallet Town — still at {}", state.map.player_position);
    });
    assert_ne!(state.map.map, Map::PalletTown, "the decision the model made is what moved the player");

    // …and it got there having actually used a tool. Without this the test would still pass if the
    // batch round trip silently answered nothing, because the second turn does not need the answer.
    let read = mock.map_result.lock().expect("not poisoned").clone();
    let read = read.expect("`read_map` was never answered — the tool round trip did not complete");
    assert!(read.contains("\"PalletTown\""), "read_map answered from the wrong state: {read:.200}");
    assert!(read.contains("\"warps\"") && read.contains("\"is_dark\""), "read_map lost its shape: {read:.200}");
    // ⚠️ The grid and its legend were *replaced* by the picture, not supplemented — a model given
    // both would be reading the same map twice, in two coordinate systems, for twice the tokens.
    assert!(!read.contains("\"grid\"") && !read.contains("\"legend\""),
            "read_map is still shipping the ASCII grid: {read:.200}");

    // **W5** — and both pictures from that assistant message came back too, encoded by the worker
    // and carried to the endpoint in the multi-part content form. This is the only test in which
    // that form goes through the real client, so it is the only place a PNG the endpoint would have
    // accepted is actually proved to be one.
    use image::GenericImageView;
    let pictures = mock.picture.lock().expect("not poisoned").clone();
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

    let screen = ((crate::ppu::LCD_WIDTH * SCALE) as u32, (crate::ppu::LCD_HEIGHT * SCALE) as u32);
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
    let mock = Mock::default();
    let config = LlmConfig {
        base_url: serve_mock(mock.clone()),
        api_key: "mock".to_string(),
        model: "mock".to_string(),
        context_limit: 128_000,
        compact_above: crate::llm::config::DEFAULT_COMPACT_ABOVE,
        temperature: 1.0,
        max_tool_steps: 6,
        request_timeout: std::time::Duration::from_secs(crate::llm::config::DEFAULT_REQUEST_TIMEOUT_SECS),
        max_tokens: Some(crate::llm::config::DEFAULT_MAX_TOKENS),
        reasoning_effort: None,
        stuck_timeout: Some(Duration::from_secs(1)),
    };
    let published = Published::new();
    let (worker, handles) = worker::channels(
        Box::new(OpenAiClient::new(&config)),
        config,
        Arc::clone(&published),
        crate::llm::todo::TodoList::open(None),
        crate::llm::battle_script::BattleScript::open(None),
        crate::llm::history::History::open(None),
    );
    // A real run directory, so the press is recorded the way a deployed one would be.
    let scratch = crate::run::tests::Scratch::new("watchdog");
    let (run, _, _) = crate::run::RunDir::open(&scratch.0, true, "mock", &|_| false)
        .expect("a fresh run directory");
    let run_path = run.path().to_path_buf();
    let current = Arc::new(crate::run::CurrentRun::new(scratch.0.clone(), "mock".into(), run));
    let _worker = worker.with_run(current).spawn().expect("the worker thread starts");

    let mut fixture = TestFixture::with_policy(
        FIXTURE,
        Duration::from_secs(120),
        Box::new(LlmPolicy::new(handles, Some(Duration::from_secs(1)))),
    );

    let mut reported = false;
    let mut delivered = false;
    for _ in 0..4_000 {
        fixture.step();
        reported |= fixture.agent.drain_events().iter().any(|event| {
            matches!(event, crate::pokemon::agent::AgentEvent::WatchdogFired { .. })
        });
        delivered |= fixture.agent.manual_input_pending() > 0;
        if reported && delivered && mock.stuck_turn.lock().expect("not poisoned").is_some() {
            break;
        }
    }

    let stuck = mock.stuck_turn.lock().expect("not poisoned").clone();
    let (situation, terminals) = stuck.expect("no stuck turn ever reached the endpoint");

    // Scoped as §7.5 requires: the escape hatch and doing nothing, and nothing else. A menu tool
    // here would let a turn end in a decision the wedged agent cannot carry out.
    let mut terminals = terminals;
    terminals.sort();
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
    let records = run_path.join(crate::run::files::PRESS_BUTTONS);
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

// ── The bundled strategy, against real battles ───────────────────────────────────────────────────

/// Drive the real agent through Mt Moon with [`battle_script::DETERMINISTIC`] deciding every battle
/// turn, and print the reports the model would have been sent.
///
/// ⚠️ **The only place the sandbox meets the actual game.** Everything else about battle scripts is
/// checked against hand-built `GameState`s or a static fixture: this is a real emulator, real wild
/// encounters and real trainers, with the script answering each turn on the emulator thread exactly
/// as a deployed run would. It prints rather than asserts — the thing worth checking is whether the
/// reports read like an account of a battle — so it is `#[ignore]`d on top of its feature gate, as
/// `CLAUDE.md` requires of every `probe_`.
///
/// ⚠️ **The overworld is still the deterministic policy's**, so the run walks Mt Moon the way the
/// leg test does and the battles are the ones it would really have had. Only `pick_battle_action` is
/// replaced. And the party is deliberately **not** `pimp_pokemon`'d: a maxed team one-shots
/// everything and the reports would all be one line.
#[cfg(feature = "diagnostics")]
#[test]
#[ignore]
fn probe_scripted_battles() {
    use crate::llm::battle_report::BattleReport;
    use crate::llm::battle_script::{self, Outcome};
    use crate::pokemon::GameState;
    use crate::pokemon::agent::AgentEvent;
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::policy::{DeterministicPolicy, Policy, PolicyStep};
    use crate::pokemon::world_graph::WorldGraph;
    use std::sync::Mutex;

    /// The deterministic policy with its battle turns handed to the sandbox.
    struct Scripted {
        inner: DeterministicPolicy,
        report: Option<BattleReport>,
        finishing: Option<BattleReport>,
        last_battle: Option<GameState>,
        turns: u32,
        done: Arc<Mutex<Vec<String>>>,
        /// How many `TextBox` events arrived while a battle was being written up, so the probe can
        /// say whether the cartridge's own sentences reached the report or merely could have.
        said: Arc<AtomicUsize>,
    }

    impl Policy for Scripted {
        fn name(&self) -> &'static str { "scripted-battles" }

        fn pick_overworld_action(&mut self, state: &GameState, graph: &WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction>
        {
            self.inner.pick_overworld_action(state, graph)
        }

        /// ⚠️ **This is what makes the probe faithful, and its absence was the first thing wrong
        /// with it.** `LlmPolicy` keeps the last state that still had a battle in it from *every*
        /// poll (see `LlmPolicy::last_battle_state`); a probe that only snapshotted at decision time
        /// closed each turn against the state the turn *opened* with, so a one-shot KO reported the
        /// foe at full HP.
        fn service_tools(&mut self, state: &GameState, _api: &mut crate::pokemon::PokemonApi<'_>,
                         _graph: &WorldGraph) {
            if state.battle.is_some() && self.report.is_some() {
                self.last_battle = Some(state.clone());
            }
            if self.finishing.is_some() && state.battle.is_none() {
                let closing = self.last_battle.take();
                if let Some(report) = self.finishing.take() {
                    self.done.lock().unwrap().push(report.finish(Some(state).or(closing.as_ref())));
                }
            }
        }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            if state.battle.as_ref()?.battle_type == crate::pokemon::battle::BattleType::Safari {
                return self.inner.pick_battle_action(state);
            }
            let report = match self.report.as_mut() {
                Some(report) => report,
                None => self.report.insert(BattleReport::open(state, 0)?),
            };
            self.turns += 1;
            let evaluation = battle_script::run(battle_script::DETERMINISTIC, state, self.turns);
            match evaluation.outcome {
                Outcome::Action(action) => {
                    report.decided(state, &action, evaluation.prints);
                    Some(action)
                }
                // Neither should ever happen here, and both are worth seeing loudly if they do:
                // this is the strategy the game is finished with.
                Outcome::Ask => {
                    println!("  [script] asked for help on turn {}", self.turns);
                    report.handed_back(state);
                    self.inner.pick_battle_action(state)
                }
                Outcome::Failed(why) => panic!("the bundled strategy failed mid-battle: {why}"),
            }
        }

        fn pick_nickname(&mut self, species: crate::pokemon::species::PokemonSpecies)
            -> Option<Option<String>> { self.inner.pick_nickname(species) }
        fn pick_mart_purchase(&mut self, state: &GameState)
            -> Option<Option<crate::pokemon::bag::BagItem>> { self.inner.pick_mart_purchase(state) }
        fn pick_move_to_forget(&mut self, slot: usize, current: &[crate::pokemon::move_name::PokemonMove],
                               new: crate::pokemon::move_name::PokemonMoveName)
            -> Option<Option<usize>> { self.inner.pick_move_to_forget(slot, current, new) }
        fn pick_field_move(&mut self, state: &GameState)
            -> Option<crate::pokemon::policy::FieldMove> { self.inner.pick_field_move(state) }
        fn is_exhausted(&self) -> bool { self.inner.is_exhausted() }
        fn steps_remaining(&self) -> Option<usize> { self.inner.steps_remaining() }

        fn on_event(&mut self, event: &AgentEvent) {
            match event {
                AgentEvent::TextBox { message } => {
                    if let Some(report) = self.report.as_mut() {
                        report.said(message);
                        self.said.fetch_add(1, Ordering::Relaxed);
                    }
                }
                // ⚠️ **Handed over rather than finished, exactly as `LlmPolicy` does it.** There is
                // nothing worth closing against yet: the last state this policy holds is the one the
                // final turn *opened* with. It is closed at the next observation with no battle in
                // it, where the party carries the real HP.
                AgentEvent::BattleEnded => {
                    self.turns = 0;
                    self.finishing = self.report.take();
                }
                _ => {}
            }
            self.inner.on_event(event);
        }
    }

    /// How many battles to print before stopping. Enough to show a wild encounter, a trainer and
    /// whatever the script does when something goes wrong; not so many that the output is a wall.
    const WANTED: usize = 6;

    let done = Arc::new(Mutex::new(Vec::new()));
    let said = Arc::new(AtomicUsize::new(0));
    let policy = Scripted {
        inner: DeterministicPolicy::new(42, PolicyStep::mt_moon_traversal()),
        report: None,
        finishing: None,
        last_battle: None,
        turns: 0,
        done: Arc::clone(&done),
        said: Arc::clone(&said),
    };
    // ⚠️ **No cartridge text reaches these reports, and it is not the script's doing.** Across 11
    // battle turns not one `AgentEvent::TextBox` fires while `AgentState::Battle` is live, though
    // the box that *opens* the battle is captured and overworld boxes are captured normally. It is
    // not the harness's fast options either: `with_original_battle_timing` changes nothing. The
    // reader is only fed from one arm of `BattleState::WaitingForMenu`, and `update_with` presses a
    // button as it reads, so feeding it from the others is a battle-timing change rather than an
    // observation. Left alone deliberately — see the ⚠️ on `with_original_battle_timing`.
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/mt-moon.bin"),
        Duration::from_mins(40),
        Box::new(policy),
    );

    while done.lock().unwrap().len() < WANTED && !fixture.agent.policy_exhausted() {
        fixture.step();
    }

    let said = said.load(Ordering::Relaxed);
    let reports = done.lock().unwrap();
    println!("\n════════ {} battles, {said} lines of cartridge text ════════\n", reports.len());
    for report in reports.iter() {
        println!("{report}");
    }
}

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
