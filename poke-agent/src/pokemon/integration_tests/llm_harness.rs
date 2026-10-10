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
use crate::llm::worker::{self, RefusalPark};
use crate::pokemon::integration_tests::fixture::TestFixture;
use crate::pokemon::llm_policy::LlmPolicy;
use crate::published::{Published, UiEvent, UiEventBody};
use crate::tour::endpoint::{error_body, sse_body};
pub use crate::tour::turn::{Brain, Call, Fault, Reply, SeenMessage, TurnRequest};

/// A brain that answers with the same thing every time.
pub struct Always(pub Reply);

impl Brain for Always {
    fn respond(&mut self, _request: &TurnRequest) -> Reply {
        self.0.clone()
    }
}

/// A brain that faults `count` times and then plays on with `then`.
pub struct FaultThen {
    pub fault: Fault,
    pub count: usize,
    pub then: Reply,
    /// How many faults have actually been served.
    pub served: Arc<AtomicUsize>,
}

impl FaultThen {
    pub fn new(fault: Fault, count: usize, then: Reply) -> Self {
        Self { fault, count, then, served: Arc::new(AtomicUsize::new(0)) }
    }
}

impl Brain for FaultThen {
    fn respond(&mut self, _request: &TurnRequest) -> Reply {
        // Counted per request, not per turn: `stream_with_retries` asks a retryable fault several
        // times in one turn.
        let served = self.served.load(Ordering::SeqCst);
        if served < self.count {
            self.served.store(served + 1, Ordering::SeqCst);
            return Reply::Fault(self.fault.clone());
        }
        self.then.clone()
    }
}

// ── The endpoint ──

#[derive(Clone)]
struct Endpoint {
    brain: Arc<Mutex<Box<dyn Brain>>>,
    seen: Arc<AtomicUsize>,
    /// Every request the endpoint was sent, so a test can assert on what the model was shown.
    log: Arc<Mutex<Vec<TurnRequest>>>,
    /// How long a `Timeout` fault holds the body open for.
    timeout_hold: Duration,
}

/// How many requests [`MockEndpoint::requests`] keeps.
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

async fn completions(State(endpoint): State<Endpoint>, body: String) -> Response {
    let wire: serde_json::Value = serde_json::from_str(&body).expect("the client sends JSON");
    let request = TurnRequest::from_wire(&wire, endpoint.seen.fetch_add(1, Ordering::SeqCst));
    {
        let mut log = endpoint.log.lock().expect("not poisoned");
        if log.len() == KEPT_REQUESTS {
            log.remove(0);
        }
        log.push(request.clone());
    }

    let reply = endpoint.brain.lock().expect("not poisoned").respond(&request);
    if let Some(body) = sse_body(&reply) {
        return ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response();
    }
    let Reply::Fault(fault) = reply else { unreachable!("every other reply is a stream") };
    match fault {
        Fault::Http { status, message } => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            error_body(&message),
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
            (StatusCode::TOO_MANY_REQUESTS, headers, error_body(&message)).into_response()
        }
        // A 200 with the body half-written: `timeout_recv_body` is a separate deadline from
        // `timeout_recv_response`.
        _ => {
            let timeout_hold = endpoint.timeout_hold;
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
    }
}

/// How the harness paces the emulator by default.
pub const HOST_TICK: Duration = Duration::from_millis(20);

/// No backoff.
pub const NO_BACKOFF: RetryPolicy =
    RetryPolicy { attempts: 3, base: Duration::ZERO, max: Duration::ZERO };

/// The deployed refusal streak, parked just long enough to tick through.
pub const SHORT_REFUSAL_PARK: RefusalPark =
    RefusalPark { after: 3, first: Duration::from_millis(100), max: Duration::from_millis(200) };

/// The assembled stack: mock endpoint, worker, run directory, policy, agent, emulator.
pub struct LlmRun {
    pub endpoint: MockEndpoint,
    pub published: Arc<Published>,
    /// Where `history.json`, `conversation.jsonl`, `todo.json` and `battle-script.json` live, as
    /// deployed.
    pub run_dir: std::path::PathBuf,
    /// `None` only between a restart's teardown and its bring-up.
    fixture: Option<TestFixture>,
    run: Option<Arc<crate::run::CurrentRun>>,
    root: std::path::PathBuf,
    /// Kept so the directory outlives the run.
    _scratch: crate::run::Scratch,
    config: LlmConfig,
    retry: RetryPolicy,
    refusal_park: RefusalPark,
    worker: Option<std::thread::JoinHandle<()>>,
    events: Mutex<tokio::sync::broadcast::Receiver<UiEvent>>,
    seen: Mutex<Vec<UiEvent>>,
    fixture_state: &'static [u8],
    want_coverage: bool,
    /// The coverage log, held across a restart.
    coverage_log: Option<crate::pokemon::integration_tests::coverage::CoverageLog>,
    max_game_time: Duration,
    stuck_timeout: Option<Duration>,
    /// How many processes this run has had. `1` until the first [`Self::restart`].
    pub processes: u64,
    pub cheats: Option<crate::tour::cheats::Cheats>,
    options: crate::pokemon::options::GameOptions,
    recording: Option<Arc<Mutex<crate::lockstep::action_for_action::Log>>>,
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
    refusal_park: RefusalPark,
    name: &'static str,
    coverage: bool,
    options: crate::pokemon::options::GameOptions,
    recording: Option<Arc<Mutex<crate::lockstep::action_for_action::Log>>>,
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
            refusal_park: SHORT_REFUSAL_PARK,
            name: "llm-run",
            coverage: false,
            options: crate::pokemon::options::HEADLESS_OPTIONS,
            recording: None,
        }
    }

    /// Write down, into `log`, where the cartridge stood before every overworld action and every
    /// answer after it, for `lockstep::action_for_action` to replay.
    pub fn recording(mut self, log: Arc<Mutex<crate::lockstep::action_for_action::Log>>) -> Self {
        self.recording = Some(log);
        self
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

    pub fn refusal_park(mut self, park: RefusalPark) -> Self {
        self.refusal_park = park;
        self
    }

    /// Play on `options` rather than [`crate::pokemon::options::HEADLESS_OPTIONS`].
    pub fn options(mut self, options: crate::pokemon::options::GameOptions) -> Self {
        self.options = options;
        self
    }

    /// Names the scratch directory, so a failing test says which run's files to look at.
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    #[cfg(feature = "slow-tests")]
    pub fn with_coverage(mut self) -> Self {
        self.coverage = true;
        self
    }

    fn endpoint(&self, brain: Box<dyn Brain>) -> MockEndpoint {
        // A `Timeout` fault outlasts the client's patience and nothing more, to keep wall clock
        // down.
        MockEndpoint::start_with_timeout_hold(brain, self.request_timeout * 3 + Duration::from_millis(200))
    }

    fn config(&self, endpoint: &MockEndpoint) -> LlmConfig {
        self.config_for(&endpoint.base_url())
    }

    fn config_for(&self, base_url: &str) -> LlmConfig {
        LlmConfig {
            base_url: base_url.to_string(),
            api_key: "mock".to_string(),
            model: "mock".to_string(),
            context_limit: self.context_limit,
            compact_above: self.compact_above,
            temperature: None,
            max_tool_steps: self.max_tool_steps,
            request_timeout: self.request_timeout,
            max_tokens: Some(crate::llm::config::DEFAULT_MAX_TOKENS),
            reasoning_effort: None,
            reasoning_budget: None,
            stuck_timeout: self.stuck_timeout,
        }
    }

    pub fn start(self, brain: Box<dyn Brain>) -> LlmRun {
        let endpoint = self.endpoint(brain);
        let config = self.config(&endpoint);

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
            refusal_park: self.refusal_park,
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
            options: self.options,
            recording: self.recording,
        };
        run.bring_up(None);
        run
    }
}

impl LlmRunBuilder {
    /// The same stack over the recreation: `game` played by `LlmPolicy` through the native agent,
    /// from a fresh run directory. The fixture and the options are the emulator's and go unused.
    pub fn start_native(self, game: pokered::Game, brain: Box<dyn Brain>) -> NativeLlmRun {
        let endpoint = self.endpoint(brain);
        let config = self.config(&endpoint);
        let client = Box::new(OpenAiClient::new(&config));
        self.start_native_on(game, config, client, Some(endpoint))
    }

    #[cfg(feature = "slow-tests")]
    /// [`Self::start_native`] with `brain` answering in this process, behind a
    /// [`BrainEndpoint`](crate::tour::endpoint::BrainEndpoint) rather than a server.
    pub fn start_native_in_process(self, game: pokered::Game, brain: Box<dyn Brain>) -> NativeLlmRun {
        let config = self.config_for("");
        let client = Box::new(crate::tour::endpoint::BrainEndpoint::new(brain));
        self.start_native_on(game, config, client, None)
    }

    fn start_native_on(self, game: pokered::Game, config: LlmConfig, client: Box<dyn crate::llm::client::ChatEndpoint>,
                       endpoint: Option<MockEndpoint>) -> NativeLlmRun {
        let scratch = crate::run::Scratch::new(self.name);
        let (run, _origin, _saved) = crate::run::RunDir::open(&scratch.0, true, "mock", &|bytes| !bytes.is_empty())
            .expect("a run directory");
        let run_dir = run.path().to_path_buf();
        let dir = Some(run_dir.as_path());
        let published = Published::new();
        let (worker, handles) = worker::channels(
            client,
            config.clone(),
            Arc::clone(&published),
            TodoList::open(dir),
            BattleScript::open(dir),
            History::open(dir),
        );
        let current = Arc::new(crate::run::CurrentRun::new(scratch.0.clone(), "mock".into(), run));
        let worker = worker
            .with_retry(self.retry)
            .with_refusal_park(self.refusal_park)
            .with_run(current)
            .spawn()
            .expect("the worker thread starts");
        let policy = Box::new(LlmPolicy::new(handles, self.stuck_timeout));
        let agent = crate::pokemon::native_agent::NativeAgent::new(game, policy).expect("a native agent");
        NativeLlmRun {
            _endpoint: endpoint,
            published,
            agent: Some(agent),
            _scratch: scratch,
            worker: Some(worker),
            max_frames: self.max_game_time.as_secs() * 60,
            slots: pokered::save_slots::MemoryStore::default(),
        }
    }
}

/// [`LlmRun`] over the recreation: the same endpoint, worker and policy, a [`NativeAgent`] where
/// the emulator was, and a frame where the host ticked 20 ms.
///
/// [`NativeAgent`]: crate::pokemon::native_agent::NativeAgent
pub struct NativeLlmRun {
    /// Held so the server outlives the run; `None` when the brain answers in process.
    _endpoint: Option<MockEndpoint>,
    published: Arc<Published>,
    /// `None` only once dropped, so the policy's channel closes before the worker is joined.
    agent: Option<crate::pokemon::native_agent::NativeAgent>,
    _scratch: crate::run::Scratch,
    worker: Option<std::thread::JoinHandle<()>>,
    /// The game time the run may play, in frames.
    max_frames: u64,
    /// The host's save slots, which only the Hall of Fame writes.
    slots: pokered::save_slots::MemoryStore,
}

impl NativeLlmRun {
    pub fn agent(&mut self) -> &mut crate::pokemon::native_agent::NativeAgent {
        self.agent.as_mut().expect("a live agent")
    }

    /// One frame, honouring the park as `host.rs` does. An error is the agent finding no answer.
    pub fn tick(&mut self) -> Result<(), String> {
        if self.published.throttled_until().is_some_and(|until| crate::published::now_ms() < until) {
            std::thread::sleep(Duration::from_millis(1));
            return Ok(());
        }
        let max_frames = self.max_frames;
        let Self { agent, slots, .. } = self;
        let agent = agent.as_mut().expect("a live agent");
        if agent.game().frames() >= max_frames {
            return Err(format!("out of game time after {} frames", agent.game().frames()));
        }
        // The ceremony, the credits and the title screen they end on are the game playing to
        // itself: the agent stops at the Hall of Fame and a deployed run ends there, so the buttons
        // that see a run on into the postgame are the harness's own, as the emulated run presses them.
        if crate::tour::native_ceremony(agent, slots) {
            return Ok(());
        }
        agent.tick()
    }

    /// Tick until `done`, or until the wall clock runs out. `false` means it never happened.
    pub fn tick_until(&mut self, within: Duration, mut done: impl FnMut(&mut Self) -> bool) -> Result<bool, String> {
        let deadline = std::time::Instant::now() + within;
        while std::time::Instant::now() < deadline {
            if done(self) {
                return Ok(true);
            }
            self.tick()?;
        }
        Ok(done(self))
    }
}

impl Drop for NativeLlmRun {
    fn drop(&mut self) {
        self.agent = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl LlmRun {
    pub fn builder(fixture: &'static [u8]) -> LlmRunBuilder {
        LlmRunBuilder::new(fixture)
    }

    /// Open or resume the run directory, build the worker on it, and hang a fresh fixture off the
    /// policy.
    fn bring_up(&mut self, carry: Option<MachineCycles>) {
        // `new_run` is false after the first start, and the resume needs the `state.gbst` that
        // [`Self::restart`] checkpoints.
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
                .with_refusal_park(self.refusal_park)
                .with_run(current)
                .spawn()
                .expect("the worker thread starts"),
        );

        let mut policy: Box<dyn crate::pokemon::policy::Policy> = Box::new(LlmPolicy::new(handles, self.stuck_timeout));
        if let Some(log) = &self.recording {
            policy = Box::new(crate::lockstep::action_for_action::Recording::new(policy, Arc::clone(log)));
        }
        // Resumed from `state.gbst`, not from the emulator just held.
        let state = saved.unwrap_or_else(|| self.fixture_state.to_vec());
        let mut fixture = TestFixture::with_policy(&state, self.max_game_time, policy).with_options(self.options);
        if self.want_coverage {
            fixture = fixture.with_coverage();
        }
        fixture.total_cycles = carry.unwrap_or(MachineCycles::ZERO);
        // The coverage log belongs to the run, not the process.
        if let Some(carried) = self.coverage_log.take() {
            fixture.coverage = Some(carried);
        }
        self.fixture = Some(fixture);
        self.processes += 1;
    }

    /// Checkpoint, drop the worker and the fixture, and rebuild both from the run directory.
    pub fn restart(&mut self) {
        self.checkpoint();
        self.restart_from_last_checkpoint();
    }

    /// Restart on whatever [`Self::checkpoint`] last wrote, not where the game has since got to.
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

    /// Drop the policy, closing the turn channel, and wait for the worker thread to notice.
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

    /// One iteration of the host loop, honouring the park as `host.rs` does.
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
        // Between ticks, and only ever here.
        if self.cheats.is_some() {
            let state = match self.fixture().try_game_state() {
                Ok(state) => state,
                // Mid-transition or mid-load: nothing to hold the game to yet.
                Err(_) => return,
            };
            let mut cheats = self.cheats.take().expect("checked above");
            cheats.apply(&mut self.fixture().api(), &state);
            self.cheats = Some(cheats);
        }
        if let Some(log) = self.recording.clone() {
            log.lock().expect("not poisoned").after_tick(&self.fixture().gb);
        }
    }

    #[cfg(feature = "slow-tests")]
    /// Turn on the cheat sidecar.
    pub fn with_cheats(&mut self, cheats: crate::tour::cheats::Cheats) {
        self.cheats = Some(cheats);
    }

    #[cfg(feature = "slow-tests")]
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

    #[cfg(feature = "slow-tests")]
    /// Each completed turn's worker time in ms, `TurnStarted` to `Decision`, off the events'
    /// stamps.
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

    /// `history.json` on disk: the file a restart resumes on.
    pub fn saved_history(&self) -> serde_json::Value {
        let path = self.run_dir.join(crate::run::files::HISTORY);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("no history at {}: {e}", path.display()));
        serde_json::from_str(&text).expect("history.json is JSON")
    }

    /// The cartridge's own clock: what the leaderboard ranks on, and what a park has to stop.
    pub fn playtime_seconds(&mut self) -> u32 {
        let api = self.fixture().api();
        crate::pokemon::observe::playtime_seconds(&api)
    }

    pub fn map(&mut self) -> crate::pokemon::map::Map {
        self.fixture().game_state().map.map
    }

    #[cfg(feature = "slow-tests")]
    /// The map, or `None` mid-warp, mid-transition, or while a starter is written into an empty
    /// party.
    pub fn map_if_readable(&mut self) -> Option<crate::pokemon::map::Map> {
        self.fixture().try_game_state().ok().map(|state| state.map.map)
    }
}

impl Drop for LlmRun {
    /// Close the turn channel and let the worker finish, so no backoff outlives the test on a port
    /// the next one may get.
    fn drop(&mut self) {
        self.tear_down();
    }
}
