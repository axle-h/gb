//! **C0** — the LLM end-to-end harness: everything between a stand-in model and the game, exactly as
//! it is deployed.
//!
//! `docs/coverage-plan.md` §2. Before this file, `llm.rs` built the whole stack for one test — a real
//! socket, a real `text/event-stream` body, [`OpenAiClient`] parsing it, the real worker, the real
//! [`LlmPolicy`], the real agent, the real emulator — and every later phase would have rebuilt it.
//! Now it is built once here and everything else is a client of it.
//!
//! Three pieces:
//!
//! - [`Brain`] — what decides. It is handed [`TurnRequest`], which is **strings and nothing else**.
//! - [`MockEndpoint`] — an OpenAI-compatible server on a loopback port that asks a `Brain` and
//!   answers in SSE, including the [`Fault`]s that have actually ended deployed runs.
//! - [`LlmRun`] — the assembled stack, with the seams the old test omitted: a run directory, the
//!   host's coarse tick, and a restart.
//!
//! ⚠️ **A `Brain` sees strings and nothing else, and that is enforced by the type rather than by a
//! comment.** It gets no `GameState`, no `&mut GameBoy`, no fixture handle. If a brain cannot find
//! what it needs in the rendered situation and the action menu, a real model cannot either — and
//! that is a finding about [`crate::llm::prompt`], not a test to work around. This is the single
//! most valuable property in the whole plan and it is free; anything that adds a field to
//! [`TurnRequest`] carrying live game state throws it away.
//!
//! ⚠️ **The endpoint fragments its tool-call arguments across several `data:` frames on purpose**,
//! and interleaves the fragments of parallel calls. That is the one thing about the wire format most
//! likely to be got wrong, and an endpoint that sent a whole call in one frame would never catch it.
//! Inherited from the mock this replaces; do not simplify it away.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;

use gb::cycles::MachineCycles;
use crate::llm::battle_script::BattleScript;
use crate::llm::client::{OpenAiClient, RetryPolicy};
use crate::llm::config::LlmConfig;
use crate::llm::history::History;
use crate::llm::todo::TodoList;
use crate::llm::worker;
use crate::pokemon::integration_tests::fixture::TestFixture;
use crate::pokemon::llm_policy::LlmPolicy;
use crate::published::{Published, UiEvent, UiEventBody};

// ── What a brain is allowed to see ───────────────────────────────────────────────────────────────

/// One message, as the endpoint received it.
///
/// The multi-part content form is flattened: `text` is every text part joined, and `images` is every
/// `image_url` part as `(data_url, detail)`. A brain that wants to prove a picture arrived counts
/// them; nothing else about them is a brain's business.
#[derive(Debug, Clone)]
pub struct SeenMessage {
    pub role: String,
    pub text: String,
    pub images: Vec<(String, String)>,
}

/// One tool, as the endpoint was offered it.
#[derive(Debug, Clone)]
pub struct SeenTool {
    pub name: String,
}

/// Everything a [`Brain`] is allowed to see: the request, as strings.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub messages: Vec<SeenMessage>,
    pub tools: Vec<SeenTool>,
    /// How many requests this endpoint has answered before this one, from zero.
    pub seen: usize,
}

impl TurnRequest {
    /// The newest `user` message — the situation this turn is being asked about.
    pub fn situation(&self) -> &str {
        self.messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map_or("", |message| message.text.as_str())
    }

    /// The ids the situation offered, in the order it offered them. Parsed out of the rendered menu
    /// exactly as a model would have to — there is no other way in, by construction.
    pub fn menu_ids(&self) -> Vec<String> {
        self.situation()
            .lines()
            .filter_map(|line| line.strip_prefix("- `"))
            .filter_map(|line| line.split_once('`'))
            .map(|(id, _)| id.to_string())
            .collect()
    }

    /// The menu as `(id, description)`, parsed out of the rendered situation exactly as a model
    /// would have to.
    ///
    /// ⚠️ **The description is the half that names a destination.** An id is
    /// `Route1:5,0:Connection` and says nothing about where the connection goes; the row says "go to
    /// ViridianCity". A brain that wants to travel has to read the prose, which is the property
    /// [`Brain`] exists to enforce: if it is not there, a real model cannot travel either.
    pub fn menu_rows(&self) -> Vec<(String, String)> {
        self.situation()
            .lines()
            .filter_map(|line| line.strip_prefix("- `"))
            .filter_map(|line| line.split_once('`'))
            .map(|(id, rest)| {
                (id.to_string(), rest.trim_start_matches([' ', '—']).trim().to_string())
            })
            .collect()
    }

    /// The map the situation says the player is on. `None` on a turn that carries no location line,
    /// which is every kind but the overworld.
    pub fn location(&self) -> Option<String> {
        self.situation()
            .lines()
            .find_map(|line| line.strip_prefix("Location: "))
            .and_then(|line| line.split(" at (").next())
            .map(str::to_string)
    }

    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.iter().map(|tool| tool.name.as_str()).collect()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool.name == name)
    }

    /// A battle turn is the one that can commit a battle action.
    pub fn is_battle(&self) -> bool {
        self.has_tool("choose_battle_action")
    }

    /// **W9's turn.** Recognisable from the `tools` array alone, which is the point of scoping it: it
    /// is the only kind that can press buttons and has no menu tool to choose from.
    pub fn is_stuck(&self) -> bool {
        self.has_tool("press_buttons")
            && !self.has_tool("choose_action")
            && !self.has_tool("choose_battle_action")
    }

    /// **Compaction's own request.** No tools at all, and the last user message is the instruction —
    /// see [`crate::llm::compaction::summary_request`]. A brain that answers this with a tool call
    /// hangs the compaction, so every brain here has to recognise it.
    pub fn is_summary(&self) -> bool {
        self.tools.is_empty()
            && self.situation().starts_with(
                crate::llm::compaction::SUMMARY_INSTRUCTION.split('\n').next().unwrap_or_default(),
            )
    }

    /// Every `(data_url, detail)` the endpoint has been sent in this request, in order.
    pub fn images(&self) -> Vec<(String, String)> {
        self.messages.iter().flat_map(|message| message.images.iter().cloned()).collect()
    }

}

// ── What a brain answers with ────────────────────────────────────────────────────────────────────

/// One tool call, before it is fragmented onto the wire.
#[derive(Debug, Clone)]
pub struct Call {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl Call {
    pub fn new(name: &str, arguments: serde_json::Value) -> Self {
        Self { name: name.to_string(), arguments }
    }

    /// The terminal `wait`, which every brain needs as its "I have nothing" answer.
    pub fn wait(ticks: u64) -> Self {
        Self::new("wait", serde_json::json!({ "ticks": ticks }))
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    /// Tool calls, fragmented across `data:` frames by the endpoint.
    Calls(Vec<Call>),
    /// Prose and no tool call — the nudge-then-force path in [`worker::Worker::decide`], and what a
    /// compaction summary is answered with.
    Content(String),
    Fault(Fault),
}

impl Reply {
    pub fn call(name: &str, arguments: serde_json::Value) -> Self {
        Self::Calls(vec![Call::new(name, arguments)])
    }
}

/// The failures that have actually ended deployed runs. Every one of these is a way the endpoint can
/// answer, not a way the harness can be broken — see `llm.rs`'s fault tests for what each must do.
#[derive(Debug, Clone)]
pub enum Fault {
    /// A non-2xx with a body. ⛔ `402` is the undated hard failure of `docs/coverage-plan.md` §2.2.1.
    Http { status: u16, message: String },
    /// A 429. `retry_after` becomes a `Retry-After` header, which is what turns an ordinary
    /// backoff into a park — see [`crate::llm::LlmError::RateLimited`].
    RateLimited { retry_after: Option<Duration>, message: String },
    /// A 200 whose body starts and then stops arriving, for longer than `GB_REQUEST_TIMEOUT_SECS`.
    Timeout,
    /// Valid SSE, arguments that are not JSON.
    MalformedToolArgs,
    /// The body ends part-way through a `data:` line.
    TruncatedStream,
    /// A completion with neither content nor a tool call.
    EmptyChoice,
}

/// What decides. Handed [`TurnRequest`] and nothing else.
pub trait Brain: Send {
    fn respond(&mut self, request: &TurnRequest) -> Reply;
}

impl<F: FnMut(&TurnRequest) -> Reply + Send> Brain for F {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        self(request)
    }
}

/// A brain that answers with the same thing every time. The fault tests are all one of these.
pub struct Always(pub Reply);

impl Brain for Always {
    fn respond(&mut self, _request: &TurnRequest) -> Reply {
        self.0.clone()
    }
}

/// A brain that faults `count` times and then plays on with `then`.
///
/// The shape every fault test wants: N consecutive failures, and then a turn that has to still work.
pub struct FaultThen {
    pub fault: Fault,
    pub count: usize,
    pub then: Reply,
    /// How many faults have actually been served, so a test can say "and it really did fail N times".
    pub served: Arc<AtomicUsize>,
}

impl FaultThen {
    pub fn new(fault: Fault, count: usize, then: Reply) -> Self {
        Self { fault, count, then, served: Arc::new(AtomicUsize::new(0)) }
    }
}

impl Brain for FaultThen {
    fn respond(&mut self, _request: &TurnRequest) -> Reply {
        // ⚠️ Counted per *request*, not per turn: a retryable fault is asked several times by
        // `stream_with_retries` for one turn, and a test that counted turns would be measuring the
        // retry policy by accident.
        let served = self.served.load(Ordering::SeqCst);
        if served < self.count {
            self.served.store(served + 1, Ordering::SeqCst);
            return Reply::Fault(self.fault.clone());
        }
        self.then.clone()
    }
}

// ── The endpoint ─────────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Endpoint {
    brain: Arc<Mutex<Box<dyn Brain>>>,
    seen: Arc<AtomicUsize>,
    /// Every request the endpoint was sent, kept so a test can assert on what the model was shown
    /// without the brain having to hoard it. Bounded: only the last [`KEPT_REQUESTS`].
    log: Arc<Mutex<Vec<TurnRequest>>>,
    /// How long a `Timeout` fault holds the body open for. Longer than the config's request timeout,
    /// and no longer, so a fault test costs that and not a wall-clock second more.
    timeout_hold: Duration,
}

/// How many requests [`MockEndpoint::requests`] keeps. A god run makes thousands and each carries a
/// whole history; the tests that read this only ever want the recent ones.
const KEPT_REQUESTS: usize = 64;

/// The mock, and the handle a test keeps.
pub struct MockEndpoint {
    base_url: String,
    inner: Endpoint,
}

impl MockEndpoint {

    pub fn start_with_timeout_hold(brain: Box<dyn Brain>, timeout_hold: Duration) -> Self {
        let inner = Endpoint {
            brain: Arc::new(Mutex::new(brain)),
            seen: Arc::new(AtomicUsize::new(0)),
            log: Arc::new(Mutex::new(Vec::new())),
            timeout_hold,
        };
        let serving = inner.clone();
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
                        .with_state(serving);
                    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                        .await
                        .expect("an arbitrary loopback port");
                    ready.send(listener.local_addr().expect("bound")).expect("the test is waiting");
                    axum::serve(listener, app).await.expect("the mock serves");
                });
            })
            .expect("the mock server thread starts");

        let address =
            address.recv_timeout(Duration::from_secs(10)).expect("the mock server bound a port");
        Self { base_url: format!("http://{address}/v1"), inner }
    }

    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    /// How many requests the endpoint has answered.
    pub fn requests_served(&self) -> usize {
        self.inner.seen.load(Ordering::SeqCst)
    }

    /// The last [`KEPT_REQUESTS`] requests, oldest first.
    pub fn requests(&self) -> Vec<TurnRequest> {
        self.inner.log.lock().expect("not poisoned").clone()
    }

}

/// Flatten one wire message into what a brain is allowed to see.
fn seen_message(message: &serde_json::Value) -> SeenMessage {
    let role = message["role"].as_str().unwrap_or_default().to_string();
    let (mut text, mut images) = (String::new(), Vec::new());
    match &message["content"] {
        serde_json::Value::String(whole) => text.push_str(whole),
        serde_json::Value::Array(parts) => {
            for part in parts {
                match part["type"].as_str() {
                    Some("image_url") => images.push((
                        part["image_url"]["url"].as_str().unwrap_or_default().to_string(),
                        part["image_url"]["detail"].as_str().unwrap_or_default().to_string(),
                    )),
                    _ => {
                        if let Some(fragment) = part["text"].as_str() {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(fragment);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    SeenMessage { role, text, images }
}

async fn completions(State(endpoint): State<Endpoint>, body: String) -> Response {
    let wire: serde_json::Value = serde_json::from_str(&body).expect("the client sends JSON");
    let messages: Vec<SeenMessage> =
        wire["messages"].as_array().expect("messages").iter().map(seen_message).collect();
    let tools: Vec<SeenTool> = wire["tools"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|tool| SeenTool {
            name: tool["function"]["name"].as_str().unwrap_or_default().to_string(),
        })
        .collect();
    let seen = endpoint.seen.fetch_add(1, Ordering::SeqCst);
    let request = TurnRequest { messages, tools, seen };
    {
        let mut log = endpoint.log.lock().expect("not poisoned");
        if log.len() == KEPT_REQUESTS {
            log.remove(0);
        }
        log.push(request.clone());
    }

    let reply = endpoint.brain.lock().expect("not poisoned").respond(&request);
    match reply {
        Reply::Calls(calls) => sse_calls(&calls).into_response(),
        Reply::Content(text) => sse_content(&text).into_response(),
        Reply::Fault(fault) => serve_fault(fault, endpoint.timeout_hold).await,
    }
}

async fn serve_fault(fault: Fault, timeout_hold: Duration) -> Response {
    match fault {
        Fault::Http { status, message } => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            serde_json::json!({ "error": { "message": message } }).to_string(),
        )
            .into_response(),
        Fault::RateLimited { retry_after, message } => {
            let mut headers = HeaderMap::new();
            if let Some(after) = retry_after {
                headers.insert(
                    "retry-after",
                    after.as_secs().to_string().parse().expect("a number is a header value"),
                );
            }
            (
                StatusCode::TOO_MANY_REQUESTS,
                headers,
                serde_json::json!({ "error": { "message": message } }).to_string(),
            )
                .into_response()
        }
        // ⚠️ A 200 with the body half-written, not a slow *response*: `timeout_recv_response` and
        // `timeout_recv_body` are separate deadlines and the deployed failure is the second one. The
        // stream is opened, a frame is sent, and then nothing.
        Fault::Timeout => {
            // One chunk, then a silence longer than the client will wait. The sender is held by a
            // task that sleeps and then drops it, so the body is open and idle for `timeout_hold`.
            let (sender, receiver) =
                tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(1);
            tokio::spawn(async move {
                let _ = sender
                    .send(Ok(axum::body::Bytes::from_static(
                        b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"...\"}}]}\n\n",
                    )))
                    .await;
                tokio::time::sleep(timeout_hold).await;
            });
            (
                [(header::CONTENT_TYPE, "text/event-stream")],
                axum::body::Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receiver)),
            )
                .into_response()
        }
        Fault::MalformedToolArgs => {
            ([(header::CONTENT_TYPE, "text/event-stream")], sse_raw_call("choose_action", "{not json"))
                .into_response()
        }
        // Ends part-way through a `data:` line, which is what `read_stream` sees when the socket
        // closes mid-frame: the partial line is the last thing `lines()` yields.
        Fault::TruncatedStream => (
            [(header::CONTENT_TYPE, "text/event-stream")],
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"hel".to_string(),
        )
            .into_response(),
        Fault::EmptyChoice => (
            [(header::CONTENT_TYPE, "text/event-stream")],
            format!(
                "{}{}",
                frame(serde_json::json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] })),
                "data: [DONE]\n\n",
            ),
        )
            .into_response(),
    }
}

fn frame(value: serde_json::Value) -> String {
    format!("data: {value}\n\n")
}

/// One completion carrying prose and no tool call.
fn sse_content(text: &str) -> impl IntoResponse {
    let mut out = String::new();
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": { "role": "assistant", "content": text } }]
    })));
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": {}, "finish_reason": "stop" }]
    })));
    out.push_str(&usage_frame());
    out.push_str("data: [DONE]\n\n");
    ([(header::CONTENT_TYPE, "text/event-stream")], out)
}

/// One call whose arguments are sent verbatim, so a fault can send something that is not JSON.
fn sse_raw_call(name: &str, arguments: &str) -> String {
    let mut out = String::new();
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": { "tool_calls": [{
            "index": 0, "id": "call_mock_0", "type": "function",
            "function": { "name": name, "arguments": arguments },
        }] } }]
    })));
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
    })));
    out.push_str(&usage_frame());
    out.push_str("data: [DONE]\n\n");
    out
}

fn usage_frame() -> String {
    frame(serde_json::json!({
        "choices": [], "usage": { "prompt_tokens": 1200, "completion_tokens": 40, "total_tokens": 1240 }
    }))
}

/// One completion, as an OpenAI-compatible stream — with the arguments deliberately chopped into
/// three-character fragments, and every call's fragments interleaved with every other's, which is
/// what a parallel tool call actually looks like on the wire.
///
/// ⚠️ **Every terminal call needs a `summary` and `tools::classify` refuses one without it**, so it
/// is added here rather than in every brain. A real model fills it in: across the deployed run's
/// 2427 decisions the only ones without were the synthesised fallback waits, which never go through
/// `classify`. A brain that omitted it would have every turn rejected and spend its whole tool
/// budget finding that out.
fn sse_calls(calls: &[Call]) -> impl IntoResponse {
    let rendered: Vec<(String, String)> = calls
        .iter()
        .map(|call| {
            let mut arguments = call.arguments.clone();
            if let Some(object) = arguments.as_object_mut() {
                object
                    .entry("summary")
                    .or_insert_with(|| serde_json::json!("what the brain is doing"));
            }
            (call.name.clone(), serde_json::to_string(&arguments).expect("valid JSON"))
        })
        .collect();

    let mut out = String::new();
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": { "role": "assistant", "content": "Let me look at where I am." } }]
    })));
    for (index, (name, _)) in rendered.iter().enumerate() {
        out.push_str(&frame(serde_json::json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": index, "id": format!("call_mock_{index}"), "type": "function",
                "function": { "name": name, "arguments": "" },
            }] } }]
        })));
    }
    let fragments: Vec<Vec<&str>> =
        rendered.iter().map(|(_, arguments)| chunks(arguments, 3)).collect();
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
    out.push_str(&usage_frame());
    out.push_str("data: [DONE]\n\n");
    ([(header::CONTENT_TYPE, "text/event-stream")], out)
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

// ── The assembled stack ──────────────────────────────────────────────────────────────────────────

/// How the harness paces the emulator by default.
///
/// ⚠️ **[`TestFixture::step_coarse`], not `step`, and that is the whole reason this constant exists.**
/// Every other test in the suite hands the agent exactly one tick's worth of time while both real
/// drivers (`host.rs`, `sdl/render.rs`) hand it however long their last loop iteration took — and the
/// resolution defect of 2026-09-03 lived entirely in that gap. An e2e harness that claims to
/// reproduce deployment has to use the driver's cadence. See `docs/test-suite.md`.
pub const HOST_TICK: Duration = Duration::from_millis(20);

/// No backoff.
///
/// ⚠️ **The shipped policy sleeps 1, 2, 4, 8 seconds between attempts**, and four of those in a
/// default-tier fault test is half a minute of nothing. What is worth asserting is what the loop
/// *does*, and the sleep is not part of it — the same argument `client.rs` makes for letting its own
/// tests set `base` to zero. A test that cares about the backoff sets its own.
pub const NO_BACKOFF: RetryPolicy =
    RetryPolicy { attempts: 3, base: Duration::ZERO, max: Duration::ZERO };

/// The assembled stack: mock endpoint, worker, run directory, policy, agent, emulator.
///
/// Everything a deployed process has except the HTTP server and the encoders, which is deliberate:
/// what is under test is the turn loop and the agent, and neither has ever been the thing that
/// broke.
pub struct LlmRun {
    pub endpoint: MockEndpoint,
    pub published: Arc<Published>,
    /// Where `history.json`, `conversation.jsonl`, `todo.json` and `battle-script.json` live —
    /// exactly as they do in deployment.
    pub run_dir: std::path::PathBuf,
    /// `None` only between a restart's teardown and its bring-up. [`Self::fixture`] unwraps it.
    fixture: Option<TestFixture>,
    run: Option<Arc<crate::run::CurrentRun>>,
    root: std::path::PathBuf,
    /// Kept so the directory outlives the run and is removed when it does not.
    _scratch: crate::run::Scratch,
    config: LlmConfig,
    retry: RetryPolicy,
    worker: Option<std::thread::JoinHandle<()>>,
    events: Mutex<tokio::sync::broadcast::Receiver<UiEvent>>,
    seen: Mutex<Vec<UiEvent>>,
    fixture_state: &'static [u8],
    want_coverage: bool,
    /// The coverage log, held across a restart. See `bring_up`.
    coverage_log: Option<crate::pokemon::integration_tests::coverage::CoverageLog>,
    max_game_time: Duration,
    stuck_timeout: Option<Duration>,
    /// How many processes this run has had. `1` until the first [`Self::restart`].
    pub processes: u64,
    /// **C1** — applied between ticks, never inside a policy. `None` is an honest run.
    pub cheats: Option<crate::pokemon::integration_tests::cheats::Cheats>,
}

/// How to build one. Everything has a default that suits the default tier.
pub struct LlmRunBuilder {
    fixture: &'static [u8],
    max_game_time: Duration,
    stuck_timeout: Option<Duration>,
    context_limit: u64,
    compact_above: f64,
    max_tool_steps: usize,
    request_timeout: Duration,
    retry: RetryPolicy,
    name: &'static str,
    coverage: bool,
}

impl LlmRunBuilder {
    pub fn new(fixture: &'static [u8]) -> Self {
        Self {
            fixture,
            max_game_time: Duration::from_secs(120),
            stuck_timeout: None,
            context_limit: 128_000,
            compact_above: crate::llm::config::DEFAULT_COMPACT_ABOVE,
            max_tool_steps: 6,
            request_timeout: Duration::from_secs(crate::llm::config::DEFAULT_REQUEST_TIMEOUT_SECS),
            retry: NO_BACKOFF,
            name: "llm-run",
            coverage: false,
        }
    }

    pub fn game_time(mut self, budget: Duration) -> Self {
        self.max_game_time = budget;
        self
    }

    pub fn stuck_timeout(mut self, timeout: Duration) -> Self {
        self.stuck_timeout = Some(timeout);
        self
    }

    pub fn context_limit(mut self, tokens: u64) -> Self {
        self.context_limit = tokens;
        self
    }

    pub fn compact_above(mut self, fraction: f64) -> Self {
        self.compact_above = fraction;
        self
    }

    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Names the scratch directory, so a failing test says which run's files to look at.
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// **C3** — keep a verdict on every action the run takes. See
    /// [`TestFixture::with_coverage`](crate::pokemon::integration_tests::TestFixture::with_coverage);
    /// the fixture then owns the event stream, so nothing else may drain it.
    pub fn with_coverage(mut self) -> Self {
        self.coverage = true;
        self
    }

    pub fn start(self, brain: Box<dyn Brain>) -> LlmRun {
        // A `Timeout` fault has to outlast the client's patience and nothing more, or every test
        // that provokes one pays the difference in wall clock.
        let endpoint = MockEndpoint::start_with_timeout_hold(
            brain,
            self.request_timeout * 3 + Duration::from_millis(200),
        );
        let config = LlmConfig {
            base_url: endpoint.base_url(),
            api_key: "mock".to_string(),
            model: "mock".to_string(),
            context_limit: self.context_limit,
            compact_above: self.compact_above,
            temperature: 1.0,
            max_tool_steps: self.max_tool_steps,
            request_timeout: self.request_timeout,
            max_tokens: Some(crate::llm::config::DEFAULT_MAX_TOKENS),
            reasoning_effort: None,
            stuck_timeout: self.stuck_timeout,
        };

        let scratch = crate::run::Scratch::new(self.name);
        let root = scratch.0.clone();
        let published = Published::new();
        let events = Mutex::new(published.subscribe_events());

        let mut run = LlmRun {
            endpoint,
            published,
            run_dir: root.clone(),
            fixture: None,
            run: None,
            root,
            _scratch: scratch,
            config,
            retry: self.retry,
            worker: None,
            events,
            seen: Mutex::new(Vec::new()),
            want_coverage: self.coverage,
            coverage_log: None,
            fixture_state: self.fixture,
            max_game_time: self.max_game_time,
            stuck_timeout: self.stuck_timeout,
            processes: 0,
            cheats: None,
        };
        run.bring_up(None);
        run
    }
}

impl LlmRun {
    pub fn builder(fixture: &'static [u8]) -> LlmRunBuilder {
        LlmRunBuilder::new(fixture)
    }

    /// Open (or resume) the run directory, build the worker on it, and hang a fresh fixture off the
    /// policy. `carry` is the emulated time already spent, so a restart's budget is the run's rather
    /// than the process's.
    ///
    /// This is the whole of a process start, which is what makes [`Self::restart`] short.
    fn bring_up(&mut self, carry: Option<MachineCycles>) {
        // ⚠️ `new_run` is false on every start after the first, and the resume then depends on there
        // being a `state.gbst` to find — which is why [`Self::restart`] checkpoints before it tears
        // down. That is the deployed order too.
        let fresh = self.processes == 0;
        let (run, _origin, saved) =
            crate::run::RunDir::open(&self.root, fresh, "mock", &|bytes| !bytes.is_empty())
                .expect("a run directory");
        self.run_dir = run.path().to_path_buf();
        let dir = Some(self.run_dir.as_path());

        let (worker, handles) = worker::channels(
            Box::new(OpenAiClient::new(&self.config)),
            self.config.clone(),
            Arc::clone(&self.published),
            TodoList::open(dir),
            BattleScript::open(dir),
            History::open(dir),
        );
        let current = Arc::new(crate::run::CurrentRun::new(self.root.clone(), "mock".into(), run));
        self.run = Some(Arc::clone(&current));
        self.worker = Some(
            worker
                .with_retry(self.retry)
                .with_run(current)
                .spawn()
                .expect("the worker thread starts"),
        );

        let policy = Box::new(LlmPolicy::new(handles, self.stuck_timeout));
        // ⚠️ **Resumed from `state.gbst`, not from the emulator we were just holding.** That gap is
        // the "conversation is a little ahead of the game" line in `prompt::RESUMED_NOTE`, and a
        // harness that handed the new process the live emulator would never reproduce it.
        let state = saved.unwrap_or_else(|| self.fixture_state.to_vec());
        let mut fixture = TestFixture::with_policy(&state, self.max_game_time, policy);
        if self.want_coverage {
            fixture = fixture.with_coverage();
        }
        fixture.total_cycles = carry.unwrap_or(MachineCycles::ZERO);
        // ⚠️ **The coverage log belongs to the *run*, not to the process.** A restart hangs a fresh
        // fixture off a fresh policy, and `with_coverage` above gives that fixture an empty log — so
        // without this every verdict taken before a restart is thrown away. Nothing needed it until
        // the coverage walk started restarting itself past the Hall of Fame
        // (`docs/coverage-plan.md` step 1.3), which is a restart in the middle of the very run whose
        // log is the deliverable.
        if let Some(carried) = self.coverage_log.take() {
            fixture.coverage = Some(carried);
        }
        self.fixture = Some(fixture);
        self.processes += 1;
    }

    /// Checkpoint, then drop the worker and the fixture, then rebuild both from the run directory.
    ///
    /// ⚠️ **This is the only way `GB_RESTORE_HISTORY`, the re-minted system prompt and the
    /// "conversation is a little ahead of the save" line are ever exercised.**
    pub fn restart(&mut self) {
        self.checkpoint();
        self.restart_from_last_checkpoint();
    }

    /// Restart **without** checkpointing first, so the process comes back up on whatever
    /// [`Self::checkpoint`] last wrote rather than on where the game has since got to.
    ///
    /// ⭐ **This is a rewind, and the coverage walk is what it is for.** Three of the ten walks win
    /// the game at 42-60% of their budget and then have no world left to walk
    /// (`docs/coverage-plan.md` step 1.3); the answer is to checkpoint at the Indigo Plateau lobby,
    /// report the Hall of Fame when it happens, and then come back up before the gauntlet and spend
    /// the rest of the budget exploring. [`Self::restart`] cannot do it — it checkpoints on the way
    /// out, which is the deployed order and would save the title screen.
    ///
    /// The emulated time already spent carries across, so a rewind buys **budget** for nothing: the
    /// walk's bound is `TestFixture::total_cycles`, and the cartridge clock going backwards under it
    /// is exactly what makes this a rewind rather than a second life.
    pub fn restart_from_last_checkpoint(&mut self) {
        let carry = self.fixture().total_cycles;
        self.coverage_log = self.fixture().coverage.take();
        self.tear_down();
        self.bring_up(Some(carry));
    }

    /// Write `state.gbst` and `sram.bin`, exactly as `EmulatorHost::checkpoint` does.
    pub fn checkpoint(&mut self) {
        let Some(run) = self.run.clone() else { return };
        let fixture = self.fixture.as_mut().expect("a live fixture");
        let state = fixture.gb.save_state().expect("a save state");
        let sram = fixture.gb.dump_sram();
        run.get()
            .checkpoint(&state, &sram, crate::run::RunProgress::default())
            .expect("the run directory is writable");
    }

    /// Drop the policy — which closes the turn channel — and wait for the worker thread to notice.
    fn tear_down(&mut self) {
        self.fixture = None;
        self.run = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub fn fixture(&mut self) -> &mut TestFixture {
        self.fixture.as_mut().expect("the run is between processes")
    }

    /// One iteration of the **host** loop, and the park honoured exactly as `host.rs` honours it.
    ///
    /// ⚠️ **The park is here rather than in the fixture** because it is the driver's job in
    /// deployment too: `EmulatorHost::paused_for` reads the same cell and skips the emulation block.
    /// Without it the fault test for a dated 429 would watch the cartridge clock run through the
    /// park it is asserting stops it.
    pub fn tick(&mut self) {
        self.drain_events();
        if self
            .published
            .throttled_until()
            .is_some_and(|until| crate::published::now_ms() < until)
        {
            std::thread::sleep(Duration::from_millis(1));
            return;
        }
        self.fixture().step_coarse(MachineCycles::from_duration(HOST_TICK));
        // ⚠️ **Between ticks, and only ever here.** `Cheats::apply` writes RAM; the whole reason a
        // finding from a cheated run is trustworthy is that the policy never sees the write except
        // through an ordinary `GameState`. See `integration_tests::cheats`.
        if self.cheats.is_some() {
            let state = match self.fixture().try_game_state() {
                Ok(state) => state,
                // Mid-transition, mid-load: there is nothing to hold the game to yet.
                Err(_) => return,
            };
            let mut cheats = self.cheats.take().expect("checked above");
            cheats.apply(&mut self.fixture().api(), &state);
            self.cheats = Some(cheats);
        }
    }

    /// Turn on the cheat sidecar. See `docs/coverage-plan.md` §3.2.
    pub fn with_cheats(&mut self, cheats: crate::pokemon::integration_tests::cheats::Cheats) {
        self.cheats = Some(cheats);
    }

    /// The coverage log, if the fixture was asked for one.
    pub fn coverage(&mut self) -> Option<&crate::pokemon::integration_tests::coverage::CoverageLog> {
        self.fixture().coverage.as_ref()
    }

    /// Tick until `done`, or until the wall clock runs out. `false` means it never happened.
    pub fn tick_until(&mut self, within: Duration, mut done: impl FnMut(&mut Self) -> bool) -> bool {
        let deadline = std::time::Instant::now() + within;
        while std::time::Instant::now() < deadline {
            if done(self) {
                return true;
            }
            self.tick();
        }
        self.drain_events();
        done(self)
    }

    /// Tick until the endpoint has answered `n` requests, or give up. `false` means it never did.
    pub fn tick_until_requests(&mut self, n: usize, within: Duration) -> bool {
        self.tick_until(within, |run| run.endpoint.requests_served() >= n)
    }

    /// Take everything the publisher has said since the last drain.
    ///
    /// ⚠️ **Drained on every tick rather than read once at the end.** The channel is a bounded
    /// broadcast: a long run overruns it and the receiver starts reporting `Lagged`, at which point
    /// the events a test is asserting on are simply gone.
    pub fn drain_events(&self) {
        let mut receiver = self.events.lock().expect("not poisoned");
        let mut seen = self.seen.lock().expect("not poisoned");
        loop {
            match receiver.try_recv() {
                Ok(event) => seen.push(event),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    }

    pub fn events(&self) -> Vec<UiEvent> {
        self.drain_events();
        self.seen.lock().expect("not poisoned").clone()
    }

    /// Every notice published so far, as `(level, message)`.
    pub fn notices(&self) -> Vec<(&'static str, String)> {
        self.events()
            .into_iter()
            .filter_map(|event| match event.body {
                UiEventBody::Notice { level, message } => Some((level, message)),
                _ => None,
            })
            .collect()
    }

    /// Whether any notice so far contains `needle`.
    pub fn said(&self, needle: &str) -> bool {
        self.notices().iter().any(|(_, message)| message.contains(needle))
    }

    /// Everything the run has actually decided, in [`worker::describe`]'s words.
    pub fn decisions(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|event| match event.body {
                UiEventBody::Decision { summary, .. } => Some(summary),
                _ => None,
            })
            .collect()
    }

    /// How long each completed turn took the worker, in milliseconds: `TurnStarted` to `Decision`,
    /// off the events' own wall-clock stamps.
    ///
    /// ⚠️ **This, not wall clock over requests, is "milliseconds per turn".** The emulator runs while
    /// the model thinks, so a run's wall clock is roughly the *larger* of its emulation and its
    /// thinking, not the sum — dividing total wall clock by requests measures the emulator and calls
    /// it the endpoint. `docs/coverage-plan.md` §4.2 wants the figure that would bound a god run,
    /// which is this one.
    pub fn turn_latencies_ms(&self) -> Vec<u64> {
        let mut opened: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
        let mut out = Vec::new();
        for event in self.events() {
            match event.body {
                UiEventBody::TurnStarted { turn, .. } => {
                    opened.insert(turn, event.at);
                }
                UiEventBody::Decision { turn, .. } => {
                    if let Some(started) = opened.remove(&turn) {
                        out.push(event.at.saturating_sub(started));
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub fn compactions(&self) -> Vec<(u64, u64, bool)> {
        self.events()
            .into_iter()
            .filter_map(|event| match event.body {
                UiEventBody::Compacted { before, after, summarised, .. } => {
                    Some((before, after, summarised))
                }
                _ => None,
            })
            .collect()
    }

    /// `history.json` as it stands on disk — the file a restart resumes on.
    pub fn saved_history(&self) -> serde_json::Value {
        let path = self.run_dir.join(crate::run::files::HISTORY);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("no history at {}: {e}", path.display()));
        serde_json::from_str(&text).expect("history.json is JSON")
    }

    /// The cartridge's own clock, which is the figure the leaderboard ranks on and therefore the one
    /// a park has to stop.
    pub fn playtime_seconds(&mut self) -> u32 {
        let api = self.fixture().api();
        crate::pokemon::observe::playtime_seconds(&api)
    }

    pub fn map(&mut self) -> crate::pokemon::map::Map {
        self.fixture().game_state().map.map
    }

    /// The map, or `None` where the game has no readable state — mid-warp, mid-transition, or on the
    /// tick a starter is being written into an empty party.
    ///
    /// ⚠️ **A long-running predicate has to use this rather than [`Self::map`].** `game_state()`
    /// unwraps, and over a whole playthrough it *will* be called on a tick that has no map: the god
    /// run panicked with "Invalid Pokemon species" the instant it picked Squirtle out of Oak's ball.
    pub fn map_if_readable(&mut self) -> Option<crate::pokemon::map::Map> {
        self.fixture().try_game_state().ok().map(|state| state.map.map)
    }
}

impl Drop for LlmRun {
    /// Close the turn channel and let the worker thread finish, so a test that ends while a backoff
    /// is in flight does not leave one running against a port the next test may be handed.
    fn drop(&mut self) {
        self.tear_down();
    }
}
