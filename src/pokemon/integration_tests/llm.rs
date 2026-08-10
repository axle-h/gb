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
    picture: Arc<std::sync::Mutex<Option<String>>>,
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
    if let Some(url) = request["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .find(|part| part["type"] == "image_url")
        .and_then(|part| part["image_url"]["url"].as_str())
    {
        *mock.picture.lock().expect("not poisoned") = Some(url.to_string());
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
        vec![("press_buttons", serde_json::json!({ "buttons": ["a"] }))]
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

    let calls: Vec<(&str, String)> = calls
        .into_iter()
        .map(|(name, arguments)| (name, serde_json::to_string(&arguments).expect("valid JSON")))
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
        temperature: 1.0,
        max_tool_steps: 6,
        stuck_timeout: None,
    };
    let published = Published::new();
    let (worker, handles) =
        worker::channels(
            Box::new(OpenAiClient::new(&config)),
            config,
            Arc::clone(&published),
            crate::llm::notes::Notes::open(None),
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
    assert!(read.contains("\"grid\"") && read.contains("\"legend\""), "read_map lost its shape: {read:.200}");

    // **W5** — and the screenshot in the same assistant message came back too, encoded by the worker
    // and carried to the endpoint in the multi-part content form. This is the only test in which
    // that form goes through the real client, and a PNG that decodes is a PNG the endpoint would
    // have accepted.
    let picture = mock.picture.lock().expect("not poisoned").clone();
    let picture = picture.expect("`screenshot` never reached the endpoint as an image part");
    let payload = picture
        .strip_prefix("data:image/png;base64,")
        .unwrap_or_else(|| panic!("not a PNG data URL: {picture:.60}"));
    let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
        .expect("the worker encoded this");
    use image::GenericImageView;
    let decoded = image::load_from_memory(&png).expect("a PNG the worker produced");
    assert_eq!(
        decoded.dimensions(),
        ((crate::ppu::LCD_WIDTH * SCALE) as u32, (crate::ppu::LCD_HEIGHT * SCALE) as u32),
        "the picture is the Game Boy screen, upscaled",
    );
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
        temperature: 1.0,
        max_tool_steps: 6,
        stuck_timeout: Some(Duration::from_secs(1)),
    };
    let published = Published::new();
    let (worker, handles) = worker::channels(
        Box::new(OpenAiClient::new(&config)),
        config,
        Arc::clone(&published),
        crate::llm::notes::Notes::open(None),
    );
    let _worker = worker.spawn().expect("the worker thread starts");

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
}
