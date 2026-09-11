//! The turn loop, on a plain blocking `std::thread`.
//! ```text
//! recv TurnRequest (blocking)
//!   ├─ append a user message: the situation, the menu, the events since the last turn
//!   ├─ loop up to GB_MAX_TOOL_STEPS:
//!   │     ├─ stream a completion  →  UiEventBody::AssistantDelta…       [cancel point]
//!   │     ├─ no tool calls?  →  nudge once, then force `wait`
//!   │     ├─ non-terminal calls → send ToolBatch, block on recv         [cancel point]
//!   │     │     ├─ Answered   → append tool result messages, continue
//!   │     │     └─ Cancelled  → drop the last assistant message, abandon the turn
//!   │     └─ terminal tool call  →  break
//!   ├─ budget exhausted without a terminal call → force `wait`
//!   ├─ send TurnOutcome
//!   └─ over GB_COMPACT_ABOVE of the context? → compact
//! ```

use std::path::PathBuf;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::llm::accounting::Accounting;
use crate::llm::battle_script::{self, BattleScript};
use crate::llm::client::{ChatEndpoint, RetryPolicy, stream_with_retries};
use crate::llm::compaction;
use crate::llm::config::LlmConfig;
use crate::llm::history::{CompactionNote, History};
use crate::llm::incident;
use crate::llm::todo::TodoList;
use crate::llm::prompt;
use crate::llm::map_image;
use crate::llm::screenshot;
use crate::llm::protocol::{self, ChatRequest, Completion, Fragment, ImageDetail, Message, StreamOptions, ToolCall, Usage};
use crate::pokemon::tile_map::MetaTileMap;
use crate::run::CurrentRun;
use crate::llm::tools::{self, CallKind, DecisionKind, Terminal};
use crate::llm::LlmError;
use crate::published::{Published, RunStatus, TodoView, UiEventBody, now_ms};

/// One question, from the policy to the worker.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    /// The generation this turn belongs to. It is stale the moment [`TurnHandles::generation`]
    /// moves past it.
    pub id: u64,
    pub kind: DecisionKind,
    /// The rendered user message — see [`prompt::situation`].
    pub situation: String,
    /// A one-line description for the UI, so a viewer sees what is being decided without the
    /// thousand tokens that were sent to decide it.
    pub headline: String,
    /// The ids this turn's situation offered, in the order it offered them.
    pub menu: Vec<String>,
}

/// The answer. Always a [`Terminal`]: a turn that could not produce one is turned into a `wait`
/// *here*, with a `UiEvent` marking it, so a model that cannot hold the contract shows up as a
/// visible rate rather than a mysteriously idle game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub id: u64,
    pub kind: DecisionKind,
    pub decision: Terminal,
}

/// Read tool calls from one assistant message, to be answered at one poll.
#[derive(Debug, Clone)]
pub struct ToolBatch {
    pub turn: u64,
    pub calls: Vec<ToolCall>,
}

/// One read tool's answer, as the emulator thread hands it back.
#[derive(Debug, Clone)]
pub struct ToolAnswer {
    pub json: String,
    pub map: Option<Box<MetaTileMap>>,
    /// Whether the map is unlit — a `GameState` fact the `MetaTileMap` does not carry.
    pub is_dark: bool,
}

impl ToolAnswer {
    pub fn text(json: impl Into<String>) -> Self {
        Self { json: json.into(), map: None, is_dark: false }
    }
}

#[derive(Debug, Clone)]
pub enum ToolBatchResult {
    /// One entry per call, in the order they were sent.
    Answered(Vec<ToolAnswer>),
    /// The decision kind changed before the batch could be serviced.
    Cancelled,
}

/// The policy's end of every channel.
pub struct TurnHandles {
    pub turns: Sender<TurnRequest>,
    pub outcomes: Receiver<TurnOutcome>,
    pub tool_calls: Receiver<ToolBatch>,
    pub tool_results: Sender<ToolBatchResult>,
    /// Bumped by the policy when the decision kind changes; read by the worker at its two cancel
    /// points. The policy owns the writes, which is why there is no lock.
    pub generation: Arc<AtomicU64>,
    /// `POST /api/new-run` and `POST /api/clear` — a pending "start again" notice. See [`Reset`].
    pub reset: Resets,
    /// The armed battle script, which the policy runs on its own thread rather than asking for.
    /// See [`crate::llm::battle_script::Live`].
    pub live_script: Arc<battle_script::Live>,
}

/// The conversation has to start again, and there are exactly two reasons it ever does.
#[derive(Debug, Clone)]
pub struct Reset {
    /// The run directory the conversation now belongs to: the new one after a restart, and the
    /// same one after a clear. `None` keeps everything in memory, as the tests do.
    pub run_dir: Option<PathBuf>,
    pub kind: ResetKind,
}

/// Why `Worker::apply_reset` is happening, which decides what survives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetKind {
    NewGame,
    /// `POST /api/clear`: the same game, played by a model that no longer remembers any of it.
    Cleared,
}

/// The cell a [`Reset`] waits in. Written by the policy, taken by the worker.
pub type Resets = Arc<Mutex<Option<Reset>>>;

impl TurnHandles {
    /// Abandon whatever is in flight and claim the next turn id.
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
}

/// After a failure that is not the model's fault — the endpoint is down, the key is wrong — the
/// turn resolves to a wait of this many agent ticks (two seconds of game time) rather than one.
const FAILURE_WAIT_TICKS: u16 = 100;

/// The longest [`Worker::park_until`] will stop the run for, however far off the endpoint says
/// its quota reopens.
const MAX_PARK: Duration = Duration::from_secs(25 * 60 * 60);

/// How finely the park is chopped.
const PARK_SLICE: Duration = Duration::from_millis(200);

/// When a run that the endpoint keeps refusing outright is parked, and for how long. A refusal is
/// an [`LlmError::Http`] that [`LlmError::is_retryable`] rejects: a spent credit, a revoked key,
/// none of which carries a time to come back at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalPark {
    /// Refusals in a row that are only failed turns. Each of those re-sends a different
    /// conversation, and a request the conversation made unacceptable can be cured between them.
    pub after: u32,
    /// The first park, doubled for every refusal after it up to `max`.
    pub first: Duration,
    pub max: Duration,
}

impl Default for RefusalPark {
    fn default() -> Self {
        Self { after: 3, first: Duration::from_secs(60), max: Duration::from_secs(30 * 60) }
    }
}

impl RefusalPark {
    /// How long to park after the `in_a_row`th consecutive refusal, if at all.
    fn window(&self, in_a_row: u32) -> Option<Duration> {
        let doublings = in_a_row.checked_sub(self.after)?;
        Some(self.first.saturating_mul(1u32 << doublings.min(16)).min(self.max))
    }
}

/// Why [`Worker::park_until`] is stopping the run, which is what the page is told.
enum Park {
    /// A dated 429.
    Quota,
    Refused { in_a_row: u32 },
}

impl Park {
    fn why(&self) -> String {
        match self {
            Park::Quota => "the endpoint's quota is spent".to_string(),
            Park::Refused { in_a_row } => format!("the endpoint refused {in_a_row} requests in a row"),
        }
    }

    fn resumed(&self) -> &'static str {
        match self {
            Park::Quota => "the quota window reopened; the run is resuming where it stopped",
            Park::Refused { .. } => "the pause is over; the run is asking the endpoint again",
        }
    }
}

/// A duration as a viewer would say it.
fn describe_wait(ms: u64) -> String {
    let seconds = ms / 1000;
    match (seconds / 3600, (seconds % 3600) / 60, seconds % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// How much of a tool's answer the page and the transcript keep.
const MAX_TOOL_RESULT: usize = 2_000;

/// Where [`Worker::trim_history`] drops back to.
const TRIM_TO: f64 = 0.50;

/// How many overworld turns an unchanged plan may sit at before a fresh copy is appended.
pub const PLAN_REFRESH_TURNS: u32 = 10;

/// What a turn found the conversation looking like when it started, so a turn that fails outright
/// can put it back exactly.
#[derive(Debug, Clone, Copy, Default)]
struct TurnOpen {
    /// `history.len()` before the plan or the situation was appended.
    at: usize,
    /// [`Worker::turns_since_plan`] as it stood then.
    turns_since_plan: u32,
}

pub struct Worker {
    endpoint: Box<dyn ChatEndpoint>,
    config: LlmConfig,
    published: Arc<Published>,
    retry: RetryPolicy,
    refusal_park: RefusalPark,
    /// Consecutive refusals, reset by any other answer — see [`RefusalPark`].
    refused_in_a_row: u32,

    generation: Arc<AtomicU64>,
    turns: Receiver<TurnRequest>,
    outcomes: Sender<TurnOutcome>,
    tool_calls: Sender<ToolBatch>,
    tool_results: Receiver<ToolBatchResult>,

    /// The conversation, and the two files it is kept in — see [`crate::llm::history`].
    history: History,
    /// The model's plan.
    todo: TodoList,
    /// The model's battle script, and the cell the policy reads it through.
    battle_script: BattleScript,
    live_script: Arc<battle_script::Live>,
    /// What the page was last told the plan is, so [`Self::publish_todo`] can be called from
    /// every moment it might have changed without publishing the same list twice.
    published_plan: Option<Vec<TodoView>>,
    /// Plan calls this turn that the list refused, so an identical one is answered rather than
    /// run again.
    refused_todo: Vec<crate::llm::todo::TodoCall>,
    /// The same, for the battle script: `(source, armed, last_failure)` as the page last saw it.
    /// `is_default` is not in here because it is a pure function of the source.
    published_script: Option<(Option<String>, bool, Option<String>)>,
    /// Turns since the plan message was last (re)placed at the tail of the history — see
    /// [`Self::sync_plan`] and [`PLAN_REFRESH_TURNS`].
    turns_since_plan: u32,
    /// Where the turn in flight started, so a turn that fails outright can be rolled back whole.
    turn_open: TurnOpen,
    /// Tokens reported, tokens spent, and how far our own estimate is from the endpoint's.
    accounting: Accounting,
    /// `POST /api/new-run` and `POST /api/clear` — taken at the top of every turn.
    reset: Resets,
    /// Where the `press_buttons` records go — see [`crate::llm::incident`].
    run: Option<Arc<CurrentRun>>,
}

/// Build the worker and its counterpart handles. The thread is started by [`Worker::spawn`]; this
/// is separate so a test can drive [`Worker::run_one`] on its own thread and control the timing.
pub fn channels(
    endpoint: Box<dyn ChatEndpoint>,
    config: LlmConfig,
    published: Arc<Published>,
    todo: TodoList,
    battle_script: BattleScript,
    history: History,
) -> (Worker, TurnHandles) {
    let (turn_tx, turn_rx) = mpsc::channel();
    let (outcome_tx, outcome_rx) = mpsc::channel();
    let (call_tx, call_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let generation = Arc::new(AtomicU64::new(0));
    let reset: Resets = Arc::new(Mutex::new(None));
    // Armed from the file at construction, not on the first `set_battle_script`.
    let live_script = Arc::new(battle_script::Live::default());
    live_script.arm(battle_script.live_source(), battle_script.state(), battle_script.standing());

    // The calibration comes back with the conversation, and nothing else does.
    let (accounting, turns_since_plan) = match history.restored() {
        Some(restored) => (
            Accounting::resumed(config.context_limit, restored.calibration),
            restored.turns_since_plan,
        ),
        None => (Accounting::new(config.context_limit), 0),
    };
    let worker = Worker {
        endpoint,
        config,
        published,
        retry: RetryPolicy::default(),
        refusal_park: RefusalPark::default(),
        refused_in_a_row: 0,
        generation: Arc::clone(&generation),
        turns: turn_rx,
        outcomes: outcome_tx,
        tool_calls: call_tx,
        tool_results: result_rx,
        history,
        todo,
        battle_script,
        live_script: Arc::clone(&live_script),
        published_plan: None,
        refused_todo: Vec::new(),
        published_script: None,
        turns_since_plan,
        turn_open: TurnOpen::default(),
        accounting,
        reset: Arc::clone(&reset),
        run: None,
    };
    let handles = TurnHandles {
        turns: turn_tx,
        outcomes: outcome_rx,
        tool_calls: call_rx,
        tool_results: result_tx,
        generation,
        reset,
        live_script,
    };
    (worker, handles)
}

impl Worker {
    /// Point the `press_buttons` records at a run directory. Without it nothing is recorded,
    /// which is what every test wants and what `gb serve` never does.
    pub fn with_run(mut self, run: Arc<CurrentRun>) -> Self {
        self.run = Some(run);
        self
    }

    /// Replace the retry policy.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_refusal_park(mut self, park: RefusalPark) -> Self {
        self.refusal_park = park;
        self
    }

    /// File a report: the screen, the run's state, a save state and the last few turns of
    /// conversation.
    fn record_report(
        &self,
        turn: u64,
        kind: DecisionKind,
        report: incident::Report<'_>,
        summary: Option<&str>,
    ) {
        let Some(run) = &self.run else { return };
        let what = match report {
            incident::Report::Issue { .. } => "report_issue",
            incident::Report::Press { .. } => "press_buttons",
        };
        match incident::record(run, &self.published, turn, kind, report, summary, &self.history) {
            Ok(path) => println!("{what} on turn {turn} ({}): recorded in {path:?}", kind.label()),
            Err(why) => eprintln!("could not record the {what} on turn {turn}: {why}"),
        }
    }

    /// Write a `report_issue` call to disk and answer it.
    fn file_issue(
        &self,
        turn: u64,
        kind: DecisionKind,
        message: &str,
        summary: Option<&str>,
    ) -> String {
        self.record_report(turn, kind, incident::Report::Issue { message }, summary);
        "Filed, with the screen and a save state. A developer will read it; nothing changes now, \
         and this did not end your turn. Carry on and try a different way of getting what you \
         wanted."
            .to_string()
    }

    /// Run the loop on a new thread. It ends when the policy is dropped, which closes the
    /// channel.
    pub fn spawn(self) -> Result<std::thread::JoinHandle<()>, String> {
        std::thread::Builder::new()
            .name("llm-worker".to_string())
            .spawn(move || self.run())
            .map_err(|e| format!("could not start the LLM worker thread: {e}"))
    }

    pub fn run(mut self) {
        while let Ok(request) = self.turns.recv() {
            self.run_one(request);
        }
    }

    /// Start the conversation again — about the game that is playing now, or about the same game
    /// this model can no longer remember.
    fn apply_reset(&mut self, reset: Reset) {
        let dir = reset.run_dir.as_deref();
        self.todo = match reset.kind {
            ResetKind::NewGame => TodoList::open(dir),
            ResetKind::Cleared => TodoList::cleared(dir),
        };
        self.publish_todo();
        if reset.kind == ResetKind::NewGame {
            // Reopened against the new directory, and the cell re-armed from what it finds.
            self.battle_script = BattleScript::open(dir);
            self.live_script.arm(self.battle_script.live_source(), self.battle_script.state(), self.battle_script.standing());
            self.publish_battle_script();
        }
        // `fresh`/`cleared`, never `open`.
        self.history = match reset.kind {
            ResetKind::NewGame => History::fresh(dir),
            ResetKind::Cleared => History::cleared(dir),
        };
        // The history the counter was measured against is gone, and `sync_plan` finds no plan in
        // the fresh one, so it appends immediately — leaving a stale count would make the *next*
        // refresh fall due at the wrong time.
        self.turns_since_plan = 0;
        self.accounting = Accounting::new(self.config.context_limit);
        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: match reset.kind {
                ResetKind::NewGame => "the game restarted; the conversation starts again from the system prompt".to_string(),
                ResetKind::Cleared => "the conversation and the plan were cleared; this turn starts again from the system prompt".to_string(),
            },
        });
    }

    /// One turn, start to finish. Public so a test can step the worker without a thread.
    pub fn run_one(&mut self, request: TurnRequest) {
        // Before the situation is appended, not after.
        if let Some(reset) = self.reset.lock().ok().and_then(|mut cell| cell.take()) {
            self.apply_reset(reset);
        }
        // The policy disarms in memory and this is what makes it durable.
        if let Some(why) = self.live_script.take_failure() {
            self.battle_script.disarm(&why);
            self.published.publish_event(UiEventBody::Notice {
                level: "warn",
                message: format!("the battle script was disarmed: {why}"),
            });
        }
        // Written back for the same reason the failure above is: the counter lives on the
        // emulator thread and only this one may touch the file.
        self.battle_script.record_decided(self.live_script.standing().decided);

        let TurnRequest { id, kind, situation, headline, menu } = request;
        self.published.publish_event(UiEventBody::TurnStarted { turn: id, kind: kind.label(), headline });

        // Before `sync_plan`, not after.
        self.turn_open = TurnOpen { at: self.history.len(), turns_since_plan: self.turns_since_plan };
        let carried = self.sync_plan(kind);
        self.publish_todo();
        self.publish_battle_script();
        // The turn that does not carry the plan has to say so.
        self.history.push(Message::user(match carried {
            true => situation,
            false => format!("{situation}\n{}\n", prompt::PLAN_UNCHANGED),
        }));
        let outcome = self.decide(id, kind, &menu);
        // Before the outcome is sent, not at the end of the turn.
        self.history.checkpoint(id, self.accounting.calibration(), self.turns_since_plan);
        match outcome {
            Some((decision, narration)) => {
                self.published.publish_event(UiEventBody::Decision {
                    turn: id,
                    summary: describe(&decision),
                    narration,
                    usage: self.accounting.has_figures().then(|| self.accounting.view()),
                });
                let _ = self.outcomes.send(TurnOutcome { id, kind, decision });
            }
            // Abandoned.
            None => {
                self.published.publish_event(UiEventBody::TurnCancelled {
                    turn: id,
                    reason: "the game moved on to a different decision".to_string(),
                });
            }
        }
        // An error stays on the board until the next turn starts.
        if !matches!(self.published.run_status(), RunStatus::Error { .. }) {
            self.published.set_status(RunStatus::Playing);
        }
        // The log is flushed above, before this runs, and that ordering is what makes the
        // watermark sound: everything a compaction is about to destroy has already been written
        // down.
        if let Some(note) = self.compact_if_needed() {
            self.history.note_compaction(id, &note);
            self.history.checkpoint(id, self.accounting.calibration(), self.turns_since_plan);
        }
    }

    /// Put the model's plan in front of it, in exactly one place, at the cheapest moment.
    fn sync_plan(&mut self, kind: DecisionKind) -> bool {
        let plan = prompt::plan_message(&self.todo);
        // The periodic refresh is an overworld thing; the edit-driven one is not.
        let due = self.turns_since_plan >= PLAN_REFRESH_TURNS && kind == DecisionKind::Overworld;
        let newest = self.history.iter().rposition(|message| prompt::is_plan(message));
        if newest.is_some_and(|at| self.history[at] == plan) && !due {
            self.turns_since_plan += 1;
            return false;
        }
        self.turns_since_plan = 0;
        self.history.push(plan);
        true
    }

    /// Put the conversation back exactly as this turn found it, and un-count the turn.
    fn roll_back_failed_turn(&mut self) {
        self.history.rollback_to(self.turn_open.at);
        self.turns_since_plan = self.turn_open.turns_since_plan;
    }

    /// Service one battle-script call, returning the sentence the model is shown.
    fn apply_battle_script(&mut self, call: tools::BattleScriptCall) -> String {
        match call {
            tools::BattleScriptCall::Docs => battle_script::DOCS.to_string(),
            tools::BattleScriptCall::Read => self.battle_script.read(),
            // Refused here rather than armed without one.
            tools::BattleScriptCall::Set { script: Some(script), purpose: None } => {
                let _ = script;
                "Not armed, and nothing was changed: `set_battle_script` needs a `purpose` as well \
                 as a `script`. One line on what you are writing it for — it stays armed and decides \
                 every battle until you replace it, and you are shown that line back on every \
                 overworld turn, so it is what will tell you later whether it still fits."
                    .to_string()
            }
            tools::BattleScriptCall::Set { script: source, purpose } => {
                let answer = self.battle_script.set(source.as_deref(), purpose.as_deref());
                self.live_script.arm(self.battle_script.live_source(), self.battle_script.state(), self.battle_script.standing());
                self.publish_battle_script();
                answer
            }
        }
    }

    /// One TODO call, applied and published.
    fn apply_todo(&mut self, call: crate::llm::todo::TodoCall) -> String {
        if self.refused_todo.contains(&call) {
            return "You already made that exact call this turn and it was refused; nothing has \
                    changed since. Your plan is the `## Your plan` message nearest the end of this \
                    conversation — use a number it actually lists, or leave the plan alone and \
                    finish the turn."
                .to_string();
        }
        let answer = self.todo.apply_reporting(call.clone());
        if answer.refused {
            self.refused_todo.push(call);
        }
        self.publish_todo();
        answer.text
    }

    /// Tell the page, if there is anything new to tell it.
    fn publish_todo(&mut self) {
        let items: Vec<TodoView> = self.todo.items().iter().map(TodoView::from).collect();
        if self.published_plan.as_ref() == Some(&items) {
            return;
        }
        self.published_plan = Some(items.clone());
        self.published.publish_event(UiEventBody::Plan { items });
    }

    /// Tell the page what is deciding the battles, if that has changed.
    fn publish_battle_script(&mut self) {
        let current = (
            self.battle_script.source().map(str::to_string),
            self.battle_script.armed(),
            self.battle_script.last_failure().map(str::to_string),
        );
        if self.published_script.as_ref() == Some(&current) {
            return;
        }
        self.published_script = Some(current.clone());
        let (source, armed, last_failure) = current;
        // Derived from the source rather than carried in the dedupe tuple above, which it would
        // only duplicate: it is a pure function of what is installed.
        let is_default = self.battle_script.is_default();
        self.published.publish_event(UiEventBody::BattleScript { source, armed, is_default, last_failure });
    }

    /// Stop the run until `until_ms`, and stop the emulator with it.
    fn park_until(&mut self, id: u64, until_ms: u64, park: Park, message: &str) -> bool {
        let now = now_ms();
        // Clamped, because the wait is driven by a number the *endpoint* chose.
        let until_ms = until_ms.min(now.saturating_add(MAX_PARK.as_millis() as u64));
        if until_ms <= now {
            return true;
        }

        self.published.publish_event(UiEventBody::Notice {
            level: "warn",
            message: format!(
                "{}, so the run is paused for {}; the game is stopped and nothing is lost. {message}",
                park.why(),
                describe_wait(until_ms - now),
            ),
        });
        self.published.set_status(RunStatus::Throttled { until_ms, message: message.to_string() });
        // Last, after the status: the emulator thread reads this one every tick, and a page that
        // sees the screen stop before it is told why has nothing to draw its overlay from.
        self.published.set_throttled_until(until_ms);

        let mut cancelled = false;
        while now_ms() < until_ms {
            if self.is_stale(id) {
                cancelled = true;
                break;
            }
            std::thread::sleep(PARK_SLICE);
        }

        self.published.set_throttled_until(0);
        if !cancelled {
            self.published.publish_event(UiEventBody::Notice {
                level: "info",
                message: park.resumed().to_string(),
            });
        }
        !cancelled
    }

    /// `None` means the turn was cancelled and abandoned.
    fn decide(&mut self, id: u64, kind: DecisionKind, menu: &[String]) -> Option<(Terminal, Option<String>)> {
        let specs = tools::for_kind(kind);
        let mut nudged = false;
        // Per turn: see `apply_todo`.
        self.refused_todo.clear();

        for step in 0..self.config.max_tool_steps {
            if self.is_stale(id) {
                return None;
            }

            self.published.set_status(RunStatus::AwaitingLlm { kind: kind.label() });
            let completion = {
                let request = ChatRequest {
                    model: self.config.model.clone(),
                    messages: self.history.to_vec(),
                    tools: specs.clone(),
                    parallel_tool_calls: Some(true),
                    max_tokens: self.config.max_tokens,
                    reasoning_effort: self.config.reasoning_effort.clone(),
                    temperature: self.config.temperature,
                    stream: true,
                    stream_options: StreamOptions { include_usage: true },
                };
                // A loop, so a parked turn asks the *same* question when the quota reopens.
                let result = loop {
                    let published = Arc::clone(&self.published);
                    let generation = Arc::clone(&self.generation);
                    let result = stream_with_retries(
                        self.retry,
                        self.endpoint.as_ref(),
                        &request,
                        &mut |delta| {
                            published.set_status(RunStatus::Streaming);
                            published.publish_event(match delta {
                                Fragment::Content(text) => {
                                    UiEventBody::AssistantDelta { turn: id, text: text.to_string() }
                                }
                                Fragment::Reasoning(text) => {
                                    UiEventBody::AssistantReasoning { turn: id, text: text.to_string() }
                                }
                            });
                        },
                        &|| generation.load(Ordering::SeqCst) != id,
                        &mut |retry| {
                            published.set_status(RunStatus::RateLimited {
                                retry_in_ms: retry.waiting.as_millis() as u64,
                            });
                            published.publish_event(UiEventBody::Notice {
                                level: "warn",
                                message: format!(
                                    "attempt {}/{} failed ({}); retrying in {:?}{}",
                                    retry.attempt,
                                    retry.of,
                                    retry.failure,
                                    retry.waiting,
                                    if retry.already_spoke { " (the reply will start again)" } else { "" },
                                ),
                            });
                        },
                    );
                    match result {
                        // The quota is spent and the endpoint dated its reopening: stop asking,
                        // stop the game with it, and put the same question again when it opens.
                        Err(LlmError::RateLimited { resets_at_ms: Some(until_ms), message }) => {
                            if !self.park_until(id, until_ms, Park::Quota, &message) {
                                break Err(LlmError::Cancelled);
                            }
                            // Back to `AwaitingLlm` *before* asking again.
                            self.published.set_status(RunStatus::AwaitingLlm { kind: kind.label() });
                        }
                        // Refused outright, and not for the first time: stop the game rather
                        // than ask again every two seconds of it, for longer each time.
                        Err(refusal @ LlmError::Http { .. }) if !refusal.is_retryable() => {
                            self.refused_in_a_row += 1;
                            let Some(window) = self.refusal_park.window(self.refused_in_a_row) else {
                                break Err(refusal);
                            };
                            let until_ms = now_ms().saturating_add(window.as_millis() as u64);
                            let park = Park::Refused { in_a_row: self.refused_in_a_row };
                            if !self.park_until(id, until_ms, park, &refusal.to_string()) {
                                break Err(LlmError::Cancelled);
                            }
                            self.published.set_status(RunStatus::AwaitingLlm { kind: kind.label() });
                        }
                        settled => {
                            if !matches!(settled, Err(LlmError::Cancelled)) {
                                self.refused_in_a_row = 0;
                            }
                            break settled;
                        }
                    }
                };
                match result {
                    Ok(completion) => completion,
                    Err(LlmError::Cancelled) => return None,
                    Err(failure) => {
                        self.published.set_status(RunStatus::Error { message: failure.to_string() });
                        self.published.publish_event(UiEventBody::Notice {
                            level: "error",
                            message: format!("the turn could not be completed: {failure}"),
                        });
                        // Nothing this turn appended was ever answered, so none of it belongs in
                        // the conversation.
                        self.roll_back_failed_turn();
                        return Some((Terminal::Wait { ticks: FAILURE_WAIT_TICKS }, None));
                    }
                }
            };

            self.account_for(&completion);
            self.history.push(Message::assistant(completion.content.clone(), completion.tool_calls.clone()));

            if completion.tool_calls.is_empty() {
                let truncated = completion.finish_reason.as_deref() == Some("length");
                if nudged {
                    return Some((self.give_up(id, match truncated {
                        true => "the model twice ran past the length limit without deciding",
                        false => "the model replied twice with no tool call",
                    }), None));
                }
                nudged = true;
                self.history.push(Message::user(match truncated {
                    true => prompt::truncated_nudge(kind),
                    false => prompt::nudge(kind),
                }));
                continue;
            }

            let classified: Vec<CallKind> =
                completion.tool_calls.iter().map(|call| tools::classify(kind, call, menu)).collect();

            // Published *after* classification, not before, so each call arrives at the page
            // already labelled — a rejected call reads as rejected rather than as one that never
            // answered.
            for (call, classification) in completion.tool_calls.iter().zip(&classified) {
                self.published.publish_event(UiEventBody::ToolCall {
                    turn: id,
                    id: call.id.clone(),
                    kind: classification.label(),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
            }

            // A message that mixes reads with a terminal call ends the turn: the model has
            // already committed, so running the reads would be answering a question it stopped
            // asking.
            if let Some(position) = classified.iter().position(|c| matches!(c, CallKind::Terminal(_))) {
                let CallKind::Terminal(decision) = &classified[position] else { unreachable!() };
                let decision = decision.clone();
                // Read off the call rather than carried through `CallKind`: it is prose for the
                // page and for the model's own memory, and nothing between here and the emulator
                // has any use for it.
                let summary = tools::call_summary(&completion.tool_calls[position]);
                let ended_with = completion.tool_calls[position].function.name.clone();
                for (index, call) in completion.tool_calls.iter().enumerate() {
                    let content = match &classified[index] {
                        _ if index == position => {
                            "Accepted. The agent is carrying it out now; the next turn will tell you \
                             what happened."
                                .to_string()
                        }
                        CallKind::Todo(call) => self.apply_todo(call.clone()),
                        // Filed, not dropped — the same exception TODO calls get, for the same
                        // reason.
                        CallKind::Issue(message) => {
                            self.file_issue(id, kind, message, summary.as_deref())
                        }
                        // The same exception a third time.
                        CallKind::BattleScript(call) => self.apply_battle_script(call.clone()),
                        CallKind::Rejected(complaint) => complaint.clone(),
                        _ => format!("Not run — the turn ended with `{ended_with}` in the same message."),
                    };
                    self.publish_tool_result(id, call, &classified[index], &content, None);
                    self.history.push(Message::tool_result(&call.id, content));
                }
                // After the tool results are appended, not before.
                if let Terminal::PressButtons { buttons } = &decision {
                    // `why` is enforced by `tools::classify` for the one kind that offers the
                    // tool, so the `unwrap_or_default` is unreachable rather than a tolerated
                    // absence.
                    let why = tools::call_reason(&completion.tool_calls[position]).unwrap_or_default();
                    let report = incident::Report::Press { buttons, why: &why };
                    self.record_report(id, kind, report, summary.as_deref());
                }
                return Some((decision, summary));
            }

            // No terminal call, so this is a read step.
            let out_of_reads = step + 2 >= self.config.max_tool_steps;
            let reads: Vec<ToolCall> = completion
                .tool_calls
                .iter()
                .zip(&classified)
                .filter(|(_, kind)| matches!(kind, CallKind::Read))
                .map(|(call, _)| call.clone())
                .collect();

            let answers = match reads.is_empty() {
                true => Vec::new(),
                false => match self.run_batch(id, reads.clone()) {
                    Some(answers) => answers,
                    None => {
                        self.history.pop();
                        return None;
                    }
                },
            };
            let mut answers = answers.into_iter();

            // Pictures cannot ride on a `tool` message (see `Message::user_with_image`), so they
            // are collected and appended *after* every tool result.
            let mut pictures: Vec<Message> = Vec::new();
            for (call, classification) in completion.tool_calls.iter().zip(&classified) {
                // The encoded picture, when this call answered with one, so the page can be
                // offered the same image the model was — see `publish_tool_result`.
                let mut png: Option<Vec<u8>> = None;
                let content = match classification {
                    CallKind::Read => {
                        let answer = answers.next().unwrap_or_else(|| ToolAnswer::text(
                            "{\"error\": \"the agent returned no result for this call\"}"));
                        match answer.map {
                            None => answer.json,
                            // Same shape as `screenshot` below, and for the same reason: the tool
                            // result is text saying a picture follows, and the picture is a
                            // `user` message appended after every result.
                            Some(map) => match map_image::render(&map) {
                                // The ASCII grid is the safety net, not dead code.
                                None => format!(
                                    "{}\n\nThis map could not be drawn, so here it is as characters \
                                     instead — one per square, {} wide, and you are the `P`:\n{map}",
                                    answer.json, map.width),
                                Some(mut canvas) => {
                                    if answer.is_dark {
                                        map_image::darken(&mut canvas);
                                    }
                                    let (width, height) = canvas.dimensions();
                                    let caption = map_image::caption(&map, answer.is_dark);
                                    // Encoded once and used twice: the model's message and the
                                    // page's ring.
                                    let encoded = map_image::encode(&canvas);
                                    let url = screenshot::png_data_url(&encoded);
                                    png = Some(encoded);
                                    pictures.push(Message::user_with_image_detail(
                                        caption.clone(),
                                        url,
                                        // `high`: a map is up to 1600 px on a side, and one
                                        // 512x512 tile would turn forty squares of terrain to
                                        // mush.
                                        ImageDetail::High,
                                        protocol::image_tokens(ImageDetail::High, width, height),
                                    ));
                                    format!("{}\n\n{caption} It is attached to the message after \
                                             this one.", answer.json)
                                }
                            },
                        }
                    }
                    CallKind::Screenshot => {
                        self.published.set_status(RunStatus::RunningTool { name: "screenshot".into() });
                        let frame = self.published.latest_frame();
                        let caption = screenshot::caption(frame.seq);
                        let encoded = screenshot::encode(&frame.pixels);
                        let url = screenshot::png_data_url(&encoded);
                        png = Some(encoded);
                        pictures.push(Message::user_with_image(caption.clone(), url));
                        format!("{caption} It is attached to the message after this one.")
                    }
                    CallKind::Todo(call) => self.apply_todo(call.clone()),
                    CallKind::Issue(message) => self.file_issue(id, kind, message, None),
                    CallKind::BattleScript(call) => self.apply_battle_script(call.clone()),
                    CallKind::Rejected(complaint) => complaint.clone(),
                    CallKind::Terminal(_) => unreachable!("handled above"),
                };
                self.publish_tool_result(id, call, classification, &content, png);
                self.history.push(Message::tool_result(&call.id, content));
            }
            self.history.extend(pictures);
            if out_of_reads {
                self.history.push(Message::user(prompt::OUT_OF_STEPS));
            }
        }

        Some((self.give_up(id, "the model used its whole tool budget without deciding"), None))
    }

    /// Say on the page what one tool call answered, and park its picture where the page can fetch
    /// it.
    fn publish_tool_result(
        &self,
        turn: u64,
        call: &ToolCall,
        classification: &CallKind,
        content: &str,
        png: Option<Vec<u8>>,
    ) {
        let (content, truncated) = match content.char_indices().nth(MAX_TOOL_RESULT) {
            None => (content.to_string(), false),
            Some((cut, _)) => (content[..cut].to_string(), true),
        };
        let content = match truncated {
            false => content,
            true => format!("{content}\n\n… (truncated for the log; the model was sent all of it)"),
        };
        let seq = self.published.publish_event(UiEventBody::ToolResult {
            turn,
            id: call.id.clone(),
            name: call.function.name.clone(),
            ok: !matches!(classification, CallKind::Rejected(_)),
            content,
            image: png.is_some(),
        });
        if let Some(png) = png {
            self.published.put_tool_image(seq, png);
        }
    }

    /// Hand a batch to the policy and block until it comes back.
    fn run_batch(&mut self, id: u64, calls: Vec<ToolCall>) -> Option<Vec<ToolAnswer>> {
        // Nothing may stop the emulator between here and the answer.
        self.published.set_status(RunStatus::RunningTool { name: names(&calls) });
        let answers = self.tool_calls.send(ToolBatch { turn: id, calls }).ok().and_then(|()| {
            // Blocking, and that is the point: this thread is *supposed* to wait.
            match self.tool_results.recv() {
                Ok(ToolBatchResult::Answered(answers)) => Some(answers),
                Ok(ToolBatchResult::Cancelled) | Err(_) => None,
            }
        });
        answers
    }

    /// The forced answer, and the event that makes it visible.
    fn give_up(&mut self, id: u64, why: &str) -> Terminal {
        self.published.publish_event(UiEventBody::Notice {
            level: "warn",
            message: format!("forcing a 1-tick wait: {why}"),
        });
        self.published.publish_event(UiEventBody::TurnCancelled { turn: id, reason: why.to_string() });
        Terminal::Wait { ticks: 1 }
    }

    fn is_stale(&self, id: u64) -> bool {
        self.generation.load(Ordering::SeqCst) != id
    }

    /// Fold one response into [`Accounting`].
    fn account_for(&mut self, completion: &Completion) {
        let usage = completion.usage.unwrap_or_else(|| Usage::estimate(&self.history, completion));
        self.accounting.record(usage, &self.history);
    }

    /// The two-stage compaction, run after every turn.
    fn compact_if_needed(&mut self) -> Option<CompactionNote> {
        if self.accounting.occupancy(&self.history) < self.config.compact_above {
            return None;
        }
        let resume = self.published.run_status();
        self.published.set_status(RunStatus::Compacting);
        let before = self.accounting.tokens_in(&self.history);
        let was = self.history.len();

        let images_evicted = compaction::evict_images(&mut self.history, compaction::KEEP_IMAGES);
        let mut summary = None;
        let still_over = self.accounting.occupancy(&self.history) >= self.config.compact_above;
        if still_over && compaction::worth_summarising(&self.history, compaction::KEEP_MESSAGES) {
            if let Some(prose) = self.summarise() {
                compaction::apply_summary(&mut self.history, &prose, compaction::KEEP_MESSAGES);
                summary = Some(prose);
            }
            // A summary could not be had — the endpoint is down, or the model returned nothing.
        }
        // Still over: the summary was refused, or what it kept is itself too big.
        if self.accounting.occupancy(&self.history) >= self.config.compact_above {
            self.trim_history();
        }
        // And the last resort has a last resort, because a history can hold no turns to drop.
        if self.accounting.occupancy(&self.history) >= self.config.compact_above {
            self.drop_unanswered();
        }

        let after = self.accounting.tokens_in(&self.history);
        let summarised = summary.is_some();
        self.published.publish_event(UiEventBody::Compacted { before, after, images_evicted, summarised });
        // A compaction that reclaimed nothing has to say so.
        if after >= before && self.accounting.occupancy(&self.history) >= self.config.compact_above {
            self.published.publish_event(UiEventBody::Notice {
                level: "error",
                message: format!(
                    "compaction reclaimed nothing: the history is still {after} tokens against a                      limit of {}, and there is nothing left in it that can be dropped safely. Every                      request from here will be over the window.",
                    self.accounting.limit(),
                ),
            });
        }
        self.published.set_status(resume);
        Some(CompactionNote {
            before,
            after,
            images_evicted,
            // What the history *lost*, which is not `was - len()`: `apply_summary` adds the
            // summary back, so the two added messages would understate the drop by exactly that
            // much.
            dropped: was.saturating_sub(self.history.len()),
            summary,
        })
    }

    /// One extra completion, asking the model to write the story so far.
    fn summarise(&mut self) -> Option<String> {
        let request = compaction::summary_request(&self.config, &self.history);
        let published = Arc::clone(&self.published);
        let result = stream_with_retries(
            self.retry,
            self.endpoint.as_ref(),
            &request,
            // Not published as an `AssistantDelta`: the summary is bookkeeping, and a thousand
            // words of it in the conversation pane would read as the model talking to itself.
            &mut |_| {},
            &|| false,
            &mut |retry| {
                published.publish_event(UiEventBody::Notice {
                    level: "warn",
                    message: format!("compaction attempt {}/{} failed ({})", retry.attempt, retry.of, retry.failure),
                });
            },
        );

        match result {
            Ok(completion) if !completion.content.trim().is_empty() => {
                let usage = completion
                    .usage
                    .unwrap_or_else(|| Usage::estimate(&request.messages, &completion));
                self.accounting.record(usage, &request.messages);
                Some(completion.content)
            }
            Ok(_) => {
                self.published.publish_event(UiEventBody::Notice {
                    level: "warn",
                    message: "the model returned an empty summary; dropping the oldest turns instead".to_string(),
                });
                None
            }
            Err(failure) => {
                self.published.publish_event(UiEventBody::Notice {
                    level: "error",
                    message: format!("could not summarise the history ({failure}); dropping the oldest turns instead"),
                });
                None
            }
        }
    }

    /// The last resort described at [`TRIM_TO`]: drop whole turns from the front until the
    /// history is back under half the window.
    fn trim_history(&mut self) {
        let target = (self.accounting.limit() as f64 * TRIM_TO) as u64;
        // Index 0 is the system prompt; index 1 is the summary, if a stage 2 has ever run.
        let first = 1 + usize::from(self.history.get(1).is_some_and(compaction::is_summary));
        let mut dropped = 0;
        while self.accounting.tokens_in(&self.history) > target {
            let Some(boundary) =
                self.history.iter().skip(first).position(compaction::is_turn_start).map(|i| i + first)
            else {
                break;
            };
            let Some(next) = self
                .history
                .iter()
                .skip(boundary + 1)
                .position(compaction::is_turn_start)
                .map(|i| i + boundary + 1)
            else {
                break; // only one turn left; dropping it would leave nothing to answer
            };
            self.history.drain(boundary..next);
            dropped += 1;
        }
        if dropped > 0 {
            self.published.publish_event(UiEventBody::Notice {
                level: "info",
                message: format!("context is full; dropped the {dropped} oldest turns"),
            });
        }
    }

    /// The last resort's last resort: drop `user` messages that were never answered.
    fn drop_unanswered(&mut self) -> usize {
        // Index 0 is the system prompt; index 1 is the summary, if a stage 2 has ever run.
        let first = 1 + usize::from(self.history.get(1).is_some_and(compaction::is_summary));
        let protected = self.history.len().saturating_sub(compaction::KEEP_MESSAGES);
        if protected <= first {
            return 0;
        }
        let mut keep = Vec::with_capacity(self.history.len());
        let mut dropped = 0;
        for index in 0..self.history.len() {
            let unanswered = index >= first
                && index < protected
                && self.history[index].role == crate::llm::protocol::Role::User
                && self.history[index + 1].role == crate::llm::protocol::Role::User;
            match unanswered {
                true => dropped += 1,
                false => keep.push(self.history[index].clone()),
            }
        }
        if dropped > 0 {
            *self.history = keep;
            self.published.publish_event(UiEventBody::Notice {
                level: "warn",
                message: format!(
                    "context is full and holds no completed turn to drop; removed {dropped} \
                     questions the endpoint never answered",
                ),
            });
        }
        dropped
    }
}

/// What the status shows while a batch is out.
fn names(calls: &[ToolCall]) -> String {
    match calls {
        [] => "nothing".to_string(),
        [one] => one.function.name.clone(),
        [first, rest @ ..] => format!("{} +{}", first.function.name, rest.len()),
    }
}

fn describe(decision: &Terminal) -> String {
    match decision {
        // The chain is on the line the page shows, because a decision that carries three actions
        // and reads as one is a decision nobody watching can account for afterwards.
        Terminal::ChooseAction { id, then, resume_after_battle } => {
            let mut line = format!("choose_action {id}");
            if !then.is_empty() {
                line.push_str(&format!(", then {}", then.join(", ")));
            }
            if *resume_after_battle {
                line.push_str(" (resuming after a battle)");
            }
            line
        }
        // On the line for the same reason the chain is: a turn that also stops the run's battle
        // script deciding the rest of this fight is not the same decision as one that does not,
        // and a scripted battle is otherwise invisible from outside.
        Terminal::ChooseBattleAction { id, take_over } => match take_over {
            true => format!("choose_battle_action {id} (taking over this battle)"),
            false => format!("choose_battle_action {id}"),
        },
        Terminal::UseFieldMove(request) => format!("use_field_move {request:?}"),
        Terminal::PressButtons { buttons } => format!(
            "press_buttons {}",
            buttons.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(" "),
        ),
        Terminal::SetNickname { name } => match name {
            Some(name) => format!("set_nickname {name}"),
            None => "set_nickname (keep the default)".to_string(),
        },
        Terminal::BuyItem { item, then } => match item {
            Some(item) => match then.is_empty() {
                true => format!("buy_item {item}"),
                false => format!(
                    "buy_item {item}, then {}",
                    then.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", "),
                ),
            },
            None => "buy_item (nothing)".to_string(),
        },
        Terminal::ForgetMove { slot } => match slot {
            Some(slot) => format!("forget_move slot {slot}"),
            None => "forget_move (decline)".to_string(),
        },
        Terminal::Wait { ticks } => format!("wait {ticks} ticks"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_parks_only_after_the_streak_and_then_for_longer_each_time() {
        let park = RefusalPark::default();
        let windows: Vec<Option<u64>> = (1..=10).map(|n| park.window(n).map(|w| w.as_secs())).collect();
        assert_eq!(
            windows,
            [None, None, Some(60), Some(120), Some(240), Some(480), Some(960), Some(1800), Some(1800), Some(1800)],
        );
        assert_eq!(park.window(u32::MAX), Some(park.max), "a long enough streak must not overflow");
    }
}
