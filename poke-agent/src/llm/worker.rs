//! The turn loop, on a plain blocking thread. A `TurnRequest` appends the situation and streams
//! up to `GB_MAX_TOOL_STEPS` completions: reads go to the policy as a `ToolBatch`, a reply with no
//! call is nudged once, and a terminal call ends the turn, while running out forces a `wait`. The
//! generation is checked while streaming and while a batch is out, and a stale one abandons the
//! turn. Compaction runs after the outcome is sent, or at once when the endpoint rejects a turn's
//! request.

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
use crate::llm::files::{self, FileCall, FileRead, Node};
use crate::llm::plan::Plan;
use crate::llm::prompt;
use crate::llm::map_image;
use crate::llm::screenshot;
use crate::llm::protocol::{self, ChatRequest, Completion, Fragment, ImageDetail, Message, StreamOptions, ToolCall, Usage};
use crate::pokemon::tile_map::MetaTileMap;
use crate::run::CurrentRun;
use crate::llm::tools::{self, CallKind, DecisionKind, Terminal};
use crate::llm::LlmError;
use crate::published::{Published, RunStatus, UiEventBody, now_ms};

/// One question, from the policy to the worker.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    /// Stale the moment [`TurnHandles::generation`] moves past it.
    pub id: u64,
    pub kind: DecisionKind,
    /// The rendered user message — see [`prompt::situation`].
    pub situation: String,
    /// One line for the UI, naming what is being decided.
    pub headline: String,
    /// The ids this turn's situation offered, in the order it offered them.
    pub menu: Vec<String>,
}

/// Always a [`Terminal`]: a turn that could not produce one becomes a `wait` here, with a
/// `UiEvent` marking it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub id: u64,
    pub kind: DecisionKind,
    pub decision: Terminal,
}

/// Read tool calls from one assistant message, to be answered at one poll.
#[derive(Debug, Clone)]
pub struct ToolBatch {
    /// `None` for the summary request, which no turn owns: answered from whatever state is current.
    pub turn: Option<u64>,
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
    /// Bumped only by the policy, when the decision kind changes; read at the cancel points.
    pub generation: Arc<AtomicU64>,
    /// `POST /api/new-run` and `POST /api/clear` — a pending "start again" notice. See [`Reset`].
    pub reset: Resets,
    /// The armed battle script, run on the policy's thread: [`crate::llm::battle_script::Live`].
    pub live_script: Arc<battle_script::Live>,
}

/// The conversation has to start again.
#[derive(Debug, Clone)]
pub struct Reset {
    /// The new run directory after a restart, the same one after a clear; `None` stays in memory.
    pub run_dir: Option<PathBuf>,
    pub kind: ResetKind,
}

/// Why `Worker::apply_reset` is happening, which decides what survives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetKind {
    NewGame,
    /// `POST /api/clear`: the same game, played by a model that remembers none of it.
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

/// The wait, in agent ticks, that a turn the endpoint failed resolves to.
const FAILURE_WAIT_TICKS: u16 = 100;

/// The longest [`Worker::park_until`] stops the run for, whatever the endpoint says.
const MAX_PARK: Duration = Duration::from_secs(25 * 60 * 60);

/// How finely the park is chopped.
const PARK_SLICE: Duration = Duration::from_millis(200);

/// Parking a run the endpoint keeps refusing. Only [`LlmError::is_account_refusal`] is a refusal; a
/// timeout, spent 5xx retries or a request rejected as malformed is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalPark {
    /// Refusals in a row that only fail the turn, since a changed conversation may cure one.
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

/// What ends a [`Worker::park_until`] early.
#[derive(Debug, Clone, Copy)]
enum Watch {
    /// The turn went stale.
    Turn(u64),
    /// A reset is waiting, which makes the summary moot.
    Compaction,
}

/// What [`Worker::summarise`] came back with.
enum Summarised {
    Prose(String),
    /// Two genuine failures: the history is compacted without a summary.
    Failed,
    /// A reset is waiting.
    Abandoned,
}

/// Why a summary request stopped without a summary.
enum Stop {
    /// Asking the same again will not help, so it is not asked again.
    Genuine(String),
    Abandoned,
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

/// Where a turn started, so a failed turn is rolled back to a length rather than by one `pop`: it
/// can fail on its second tool step, with an assistant message and results already appended.
#[derive(Debug, Clone, Copy, Default)]
struct TurnOpen {
    /// `history.len()` before the situation was appended.
    at: usize,
    /// Restored too, or the one turn told everything else was lost is the turn that failed.
    reminder: prompt::PlanReminder,
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
    plan: Plan,
    /// What the turn about to be asked says about `plan.md`.
    reminder: prompt::PlanReminder,
    /// The issues filed by a worker with no run directory to file them in.
    issues: Vec<incident::Filed>,
    /// The model's battle script, and the cell the policy reads it through.
    battle_script: BattleScript,
    live_script: Arc<battle_script::Live>,
    /// What the page was last told the plan is, so [`Self::publish_plan`] never repeats itself.
    published_plan: Option<String>,
    /// The same, for the battle script: `(source, armed, last_failure)` as the page last saw it.
    published_script: Option<(Option<String>, bool, Option<String>)>,
    turn_open: TurnOpen,
    /// Tokens reported, tokens spent, and how far our own estimate is from the endpoint's.
    accounting: Accounting,
    /// `POST /api/new-run` and `POST /api/clear` — taken at the top of every turn.
    reset: Resets,
    /// Where the raw-press and issue records go — see [`crate::llm::incident`].
    run: Option<Arc<CurrentRun>>,
}

/// The worker and its handles, apart from [`Worker::spawn`] so a test can drive `run_one`.
pub fn channels(
    endpoint: Box<dyn ChatEndpoint>,
    config: LlmConfig,
    published: Arc<Published>,
    plan: Plan,
    battle_script: BattleScript,
    history: History,
) -> (Worker, TurnHandles) {
    let (turn_tx, turn_rx) = mpsc::channel();
    let (outcome_tx, outcome_rx) = mpsc::channel();
    let (call_tx, call_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let generation = Arc::new(AtomicU64::new(0));
    let reset: Resets = Arc::new(Mutex::new(None));
    // Armed from the file at construction, not on the first write.
    let live_script = Arc::new(battle_script::Live::default());
    live_script.arm(battle_script.live_source(), battle_script.state(), battle_script.standing());

    // The calibration comes back with the conversation, and nothing else does.
    let (accounting, reminder) = match history.restored() {
        Some(restored) => (
            Accounting::resumed(config.context_limit, restored.calibration),
            prompt::PlanReminder::Resumed,
        ),
        None => (Accounting::new(config.context_limit), prompt::PlanReminder::Ordinary),
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
        plan,
        reminder,
        issues: Vec::new(),
        battle_script,
        live_script: Arc::clone(&live_script),
        published_plan: None,
        published_script: None,
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
    /// Point the incident records at a run directory; without it nothing is recorded.
    pub fn with_run(mut self, run: Arc<CurrentRun>) -> Self {
        self.run = Some(run);
        self
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_refusal_park(mut self, park: RefusalPark) -> Self {
        self.refusal_park = park;
        self
    }

    /// File a report: the screen, the status, a save state and the last few turns.
    fn record_report(
        &self,
        turn: u64,
        kind: DecisionKind,
        report: incident::Report<'_>,
        summary: Option<&str>,
    ) {
        let Some(run) = &self.run else { return };
        let what = match report {
            incident::Report::Issue { .. } => "issue",
            incident::Report::Press { .. } => "press",
        };
        match incident::record(run, &self.published, turn, kind, report, summary, &self.history) {
            Ok(path) => println!("{what} on turn {turn} ({}): recorded in {path:?}", kind.label()),
            Err(why) => eprintln!("could not record the {what} on turn {turn}: {why}"),
        }
    }

    /// Write an issue to disk and answer it.
    fn file_issue(
        &mut self,
        turn: u64,
        kind: DecisionKind,
        file: &str,
        message: &str,
        summary: Option<&str>,
    ) -> String {
        self.record_report(turn, kind, incident::Report::Issue { file, message }, summary);
        if self.run.is_none() {
            self.issues.push(incident::Filed { file: file.to_string(), message: message.to_string(), turn });
        }
        format!(
            "Filed as issues/{file}, with the screen and a save state. A developer will read it; \
             nothing changes now, and this did not end your turn. Carry on and try a different way \
             of getting what you wanted."
        )
    }

    /// Every issue this run has filed, oldest first.
    fn filed_issues(&self) -> Vec<incident::Filed> {
        match &self.run {
            Some(run) => incident::filed(run),
            None => self.issues.clone(),
        }
    }

    /// Run the loop on a new thread, until the policy drops its end of the channel.
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

    /// Start the conversation again, for a new game or for a model made to forget this one.
    fn apply_reset(&mut self, reset: Reset) {
        let dir = reset.run_dir.as_deref();
        self.plan = match reset.kind {
            ResetKind::NewGame => Plan::open(dir),
            ResetKind::Cleared => Plan::cleared(dir),
        };
        self.publish_plan();
        self.reminder = match reset.kind {
            ResetKind::NewGame => prompt::PlanReminder::Ordinary,
            ResetKind::Cleared => prompt::PlanReminder::Cleared,
        };
        if reset.kind == ResetKind::NewGame {
            self.issues.clear();
        }
        if reset.kind == ResetKind::NewGame {
            // Reopened against the new directory, and the cell re-armed from what it finds.
            self.battle_script = BattleScript::open(dir);
            self.live_script.arm(self.battle_script.live_source(), self.battle_script.state(), self.battle_script.standing());
            self.publish_battle_script();
        }
        // `fresh`/`cleared`, never `open`, or the old conversation comes back.
        self.history = match reset.kind {
            ResetKind::NewGame => History::fresh(dir),
            ResetKind::Cleared => History::cleared(dir),
        };
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
        // The counter lives on the emulator thread, and only this one may write the file.
        self.battle_script.record_decided(self.live_script.standing().decided);

        let TurnRequest { id, kind, situation, headline, menu } = request;
        self.published.publish_event(UiEventBody::TurnStarted { turn: id, kind: kind.label(), headline });

        self.publish_plan();
        self.publish_battle_script();
        self.open_turn(&situation);
        let outcome = self.decide(id, kind, &situation, &menu);
        // Before the outcome is sent: a winning action has the archiver copy the run next tick.
        self.history.checkpoint(id, self.accounting.calibration());
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
        // The log was flushed above, so everything a compaction destroys is already written down.
        self.compact(id, false);
    }

    /// Append the situation, closed by what it says about `plan.md`.
    fn open_turn(&mut self, situation: &str) {
        self.turn_open = TurnOpen { at: self.history.len(), reminder: self.reminder };
        self.history.push(Message::user(format!("{situation}\n{}\n", self.reminder.line())));
        self.reminder = prompt::PlanReminder::Ordinary;
    }

    /// Put the conversation back exactly as this turn found it.
    fn roll_back_failed_turn(&mut self) {
        self.history.rollback_to(self.turn_open.at);
        self.reminder = self.turn_open.reminder;
    }

    /// Answer one file call the worker owns: a read of a file it keeps, a write or an edit. The
    /// flag is whether it did what was asked.
    fn apply_file(&mut self, call: &FileCall) -> (String, bool) {
        match call {
            FileCall::Read(read) => (self.read_here(read), true),
            FileCall::Write { node: Node::Plan, content } => self.write_plan(content),
            FileCall::Write { node: Node::BattleScript, content } => self.write_script(content),
            FileCall::Edit { node: Node::Plan, old, new } => {
                match files::apply_edit(files::PLAN, self.plan.text(), old, new) {
                    Ok(text) => self.write_plan(&text),
                    Err(why) => (why, false),
                }
            }
            FileCall::Edit { node: Node::BattleScript, old, new } => {
                let source = self.battle_script.source().unwrap_or_default().to_string();
                match files::apply_edit(files::BATTLE_SCRIPT, &source, old, new) {
                    Ok(text) => self.write_script(&text),
                    Err(why) => (why, false),
                }
            }
            FileCall::Write { node, .. } | FileCall::Edit { node, .. } => (files::not_writable(node), false),
        }
    }

    fn write_plan(&mut self, text: &str) -> (String, bool) {
        match self.plan.write(text) {
            Ok(()) => {
                self.publish_plan();
                (format!("plan.md written: {} lines, {} bytes.", text.lines().count(), text.len()), true)
            }
            Err(why) => (why, false),
        }
    }

    /// A write or an edit of the script: validated, and armed from the next battle decision.
    fn write_script(&mut self, source: &str) -> (String, bool) {
        let written = self.battle_script.write(source);
        if written.is_ok() {
            self.live_script.arm(self.battle_script.live_source(), self.battle_script.state(), self.battle_script.standing());
        }
        self.publish_battle_script();
        match written {
            Ok(answer) => (answer, true),
            Err(why) => (why, false),
        }
    }

    /// A read of a file the worker keeps rather than the game.
    fn read_here(&self, read: &FileRead) -> String {
        let FileRead { node, offset, limit } = read;
        let path = node.path();
        let text = match node {
            Node::Root => files::TREE.to_string(),
            Node::GuideDir => files::guide_listing(),
            Node::Plan => self.plan.text().to_string(),
            Node::BattleScript => {
                let source = self.battle_script.source().unwrap_or_default();
                let answer = files::window(&path, source, *offset, *limit);
                // The state rides on the header line, so the lines below it are the file.
                return match answer.split_once('\n') {
                    Some((header, rest)) => format!("{header}; {}\n{rest}", self.battle_script.status()),
                    None => format!("{answer}; {}", self.battle_script.status()),
                };
            }
            Node::BattleScriptDocs => battle_script::DOCS.to_string(),
            Node::IssuesDir => {
                let filed = self.filed_issues();
                if filed.is_empty() {
                    return "issues/, empty: no issue has been filed in this run. Writing a new file \
                            under it files one."
                        .to_string();
                }
                filed.iter().map(|issue| format!("issues/{}, filed on turn {}\n", issue.file, issue.turn)).collect()
            }
            Node::Issue(name) => {
                let messages: Vec<String> = self.filed_issues().into_iter()
                    .filter(|issue| issue.file.eq_ignore_ascii_case(name))
                    .map(|issue| issue.message)
                    .collect();
                if messages.is_empty() {
                    return format!("There is no issue `{path}` yet. Writing it files one.");
                }
                messages.join("\n\n")
            }
            Node::NoSuchMap(name) => return files::no_such_map(name),
            other => return files::no_such_file(&other.path()),
        };
        files::window(&path, &text, *offset, *limit)
    }

    /// Tell the page the plan, if it has changed: every version goes into the transcript.
    fn publish_plan(&mut self) {
        if self.published_plan.as_deref() == Some(self.plan.text()) {
            return;
        }
        let text = self.plan.text().to_string();
        self.published_plan = Some(text.clone());
        self.published.publish_event(UiEventBody::Plan { text });
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
        // A pure function of the source, so not in the dedupe tuple.
        let is_default = self.battle_script.is_default();
        self.published.publish_event(UiEventBody::BattleScript { source, armed, is_default, last_failure });
    }

    fn abandoned(&self, watch: Watch) -> bool {
        match watch {
            Watch::Turn(id) => self.is_stale(id),
            Watch::Compaction => reset_waiting(&self.reset),
        }
    }

    /// Stop the run until `until_ms`, and stop the emulator with it.
    fn park_until(&mut self, watch: Watch, until_ms: u64, park: Park, message: &str) -> bool {
        let now = now_ms();
        // Clamped, because the wait is driven by a number the endpoint chose.
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
        // After the status, so the page knows why before the screen stops.
        self.published.set_throttled_until(until_ms);

        let mut cancelled = false;
        while now_ms() < until_ms {
            if self.abandoned(watch) {
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

    /// `None` means the turn was cancelled and abandoned. A request the endpoint rejects as wrong, a
    /// prompt too long among them, forces a compaction and is asked once more.
    fn decide(
        &mut self,
        id: u64,
        kind: DecisionKind,
        situation: &str,
        menu: &[String],
    ) -> Option<(Terminal, Option<String>)> {
        let rejected = match self.ask(id, kind, menu) {
            Ok(outcome) => return outcome,
            Err(failure) if failure.is_rejected_request() => failure,
            Err(failure) => return Some(self.fail_turn(&failure)),
        };
        self.published.publish_event(UiEventBody::Notice {
            level: "warn",
            message: format!(
                "the endpoint rejected the turn's request ({rejected}); compacting the conversation \
                 and asking the same question again"
            ),
        });
        self.roll_back_failed_turn();
        self.compact(id, true);
        self.open_turn(situation);
        match self.ask(id, kind, menu) {
            Ok(outcome) => outcome,
            Err(failure) => Some(self.fail_turn(&failure)),
        }
    }

    /// A turn the endpoint could not answer resolves to a wait, and leaves no trace in the history.
    fn fail_turn(&mut self, failure: &LlmError) -> (Terminal, Option<String>) {
        self.published.set_status(RunStatus::Error { message: failure.to_string() });
        self.published.publish_event(UiEventBody::Notice {
            level: "error",
            message: format!("the turn could not be completed: {failure}"),
        });
        // Nothing this turn appended was answered, so all of it goes.
        self.roll_back_failed_turn();
        (Terminal::Wait { ticks: FAILURE_WAIT_TICKS }, None)
    }

    /// The turn's requests, up to `GB_MAX_TOOL_STEPS`. `Err` is a failure the turn cannot survive.
    fn ask(&mut self, id: u64, kind: DecisionKind, menu: &[String]) -> Result<Option<(Terminal, Option<String>)>, LlmError> {
        let mut nudged = false;

        for step in 0..self.config.max_tool_steps {
            if self.is_stale(id) {
                return Ok(None);
            }

            self.published.set_status(RunStatus::AwaitingLlm { kind: kind.label() });
            let completion = {
                let request = chat_request(&self.config, self.history.to_vec());
                // A loop, so a parked turn asks the same question when the quota reopens.
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
                        // A dated 429 parks the game, then asks the same question again.
                        Err(LlmError::RateLimited { resets_at_ms: Some(until_ms), message }) => {
                            if !self.park_until(Watch::Turn(id), until_ms, Park::Quota, &message) {
                                break Err(LlmError::Cancelled);
                            }
                            // Back to `AwaitingLlm` before asking again.
                            self.published.set_status(RunStatus::AwaitingLlm { kind: kind.label() });
                        }
                        // Refused again: park rather than ask every two seconds, longer each time.
                        Err(refusal) if refusal.is_account_refusal() => {
                            self.refused_in_a_row += 1;
                            let Some(window) = self.refusal_park.window(self.refused_in_a_row) else {
                                break Err(refusal);
                            };
                            let until_ms = now_ms().saturating_add(window.as_millis() as u64);
                            let park = Park::Refused { in_a_row: self.refused_in_a_row };
                            if !self.park_until(Watch::Turn(id), until_ms, park, &refusal.to_string()) {
                                break Err(LlmError::Cancelled);
                            }
                            self.published.set_status(RunStatus::AwaitingLlm { kind: kind.label() });
                        }
                        settled => {
                            // Any other answer ends the streak; a cancellation says nothing.
                            if !matches!(settled, Err(LlmError::Cancelled)) {
                                self.refused_in_a_row = 0;
                            }
                            break settled;
                        }
                    }
                };
                match result {
                    Ok(completion) => completion,
                    Err(LlmError::Cancelled) => return Ok(None),
                    Err(failure) => return Err(failure),
                }
            };

            self.account_for(id, kind, &completion);
            self.history.push(Message::assistant(completion.content.clone(), completion.tool_calls.clone()));

            if completion.tool_calls.is_empty() {
                let truncated = completion.finish_reason.as_deref() == Some("length");
                if nudged {
                    return Ok(Some((self.give_up(id, match truncated {
                        true => "the model twice ran past the length limit without deciding",
                        false => "the model replied twice with no tool call",
                    }), None)));
                }
                nudged = true;
                self.history.push(Message::user(match truncated {
                    true => prompt::truncated_nudge(kind),
                    false => prompt::nudge(kind),
                }));
                continue;
            }

            let classified = tools::classify_reply(kind, &completion.tool_calls, menu);

            // Published after classification, so a rejected call arrives labelled as one.
            for (call, classification) in completion.tool_calls.iter().zip(&classified) {
                self.published.publish_event(UiEventBody::ToolCall {
                    turn: id,
                    id: call.id.clone(),
                    kind: classification.label(),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
            }

            // A terminal call ends the turn, and reads beside it answer a question already settled.
            if let Some(position) = classified.iter().position(|c| matches!(c, CallKind::Terminal(_))) {
                let CallKind::Terminal(decision) = &classified[position] else { unreachable!() };
                let decision = decision.clone();
                let ended_with = completion.tool_calls[position].function.name.clone();
                let chained = ended_with == tools::CHOOSE_ACTION;
                // Read off the calls: the emulator has no use for the summary.
                let summary = match chained {
                    true => tools::reply_summary(&completion.tool_calls, &classified),
                    false => tools::call_summary(&completion.tool_calls[position]),
                };
                for (index, call) in completion.tool_calls.iter().enumerate() {
                    let (content, ok) = match &classified[index] {
                        _ if index == position => (
                            "Accepted. The agent is carrying it out now; the next turn will tell you \
                             what happened."
                                .to_string(),
                            true,
                        ),
                        CallKind::Chained if chained => {
                            ("Accepted, to be taken after the one before it if that one lands.".to_string(), true)
                        }
                        // A write is kept, not dropped; a read beside the action answers nothing.
                        CallKind::File(file) if !matches!(file, FileCall::Read(_)) => self.apply_file(file),
                        CallKind::Issue { name, message } => {
                            (self.file_issue(id, kind, name, message, summary.as_deref()), true)
                        }
                        CallKind::Rejected(complaint) => (complaint.clone(), false),
                        _ => (format!("Not run — the turn ended with `{ended_with}` in the same message."), true),
                    };
                    self.publish_tool_result(id, call, ok, &content, None);
                    self.history.push(Message::tool_result(&call.id, content));
                }
                // After the tool results are appended, not before.
                if let Terminal::PressButtons { buttons } = &decision {
                    // `tools::classify_reply` enforces a summary, so the default is unreachable.
                    let why = summary.clone().unwrap_or_default();
                    let report = incident::Report::Press { buttons, why: &why };
                    self.record_report(id, kind, report, summary.as_deref());
                }
                return Ok(Some((decision, summary)));
            }

            // No terminal call, so this is a read step.
            let out_of_reads = step + 2 >= self.config.max_tool_steps;
            let Some(answers) = self.answer_calls(Some((id, kind)), &completion.tool_calls, &classified) else {
                self.history.pop();
                return Ok(None);
            };
            self.history.extend(answers);
            if out_of_reads {
                self.history.push(Message::user(prompt::OUT_OF_STEPS));
            }
        }

        Ok(Some((self.give_up(id, "the model used its whole tool budget without deciding"), None)))
    }

    /// Answer the calls of one reply that did not end its turn, in order: reads of the game go to
    /// the policy as one batch, and the pictures follow every tool result. `turn` is `None` for
    /// the summary request, whose answers the page is not shown. `None` if the batch was cancelled.
    fn answer_calls(
        &mut self,
        turn: Option<(u64, DecisionKind)>,
        calls: &[ToolCall],
        classified: &[CallKind],
    ) -> Option<Vec<Message>> {
        let reads: Vec<ToolCall> = calls
            .iter()
            .zip(classified)
            .filter(|(_, kind)| matches!(kind, CallKind::Read(_)))
            .map(|(call, _)| call.clone())
            .collect();
        let answers = match reads.is_empty() {
            true => Vec::new(),
            false => self.run_batch(turn.map(|(id, _)| id), reads)?,
        };
        let mut answers = answers.into_iter();
        let mut results: Vec<Message> = Vec::new();

        // Pictures cannot ride on a `tool` message, so they follow every tool result.
        let mut pictures: Vec<Message> = Vec::new();
        for (call, classification) in calls.iter().zip(classified) {
            // The picture this call answered with, for the page too.
            let mut png: Option<Vec<u8>> = None;
            let (content, ok) = match classification {
                CallKind::Read(read) => {
                    let answer = answers.next().unwrap_or_else(|| ToolAnswer::text(
                        "The agent returned no answer for this read."));
                    let content = match answer.map {
                        None => files::window(&read.node.path(), &answer.json, read.offset, read.limit),
                        // As `screenshot` below: the result says a picture follows.
                        Some(map) => match map_image::render(&map) {
                            // The ASCII grid is the safety net, not dead code.
                            None => format!(
                                "{}\n{}\nThis map could not be drawn, so here it is as characters \
                                 instead — one per square, {} wide, and you are the `P`:\n{map}",
                                files::MAP_PNG, answer.json, map.width),
                            Some(mut canvas) => {
                                if answer.is_dark {
                                    map_image::darken(&mut canvas);
                                }
                                let (width, height) = canvas.dimensions();
                                let caption = map_image::caption(&map, answer.is_dark);
                                // Encoded once, for the model's message and the page.
                                let encoded = map_image::encode(&canvas);
                                let url = screenshot::png_data_url(&encoded);
                                png = Some(encoded);
                                pictures.push(Message::user_with_image_detail(
                                    caption.clone(),
                                    url,
                                    // `high`: one 512 px tile turns a 1600 px map to mush.
                                    ImageDetail::High,
                                    protocol::image_tokens(ImageDetail::High, width, height),
                                ));
                                format!("{}\n{}\n{caption} It is attached to the message after \
                                         this one.", files::MAP_PNG, answer.json)
                            }
                        },
                    };
                    (content, true)
                }
                CallKind::Screenshot => {
                    if turn.is_some() {
                        self.published.set_status(RunStatus::RunningTool { name: files::READ_FILE.into() });
                    }
                    let frame = self.published.latest_frame();
                    let caption = screenshot::caption(frame.seq);
                    let encoded = screenshot::encode(&frame.pixels);
                    let url = screenshot::png_data_url(&encoded);
                    png = Some(encoded);
                    pictures.push(Message::user_with_image(caption.clone(), url));
                    (format!("{}\n{caption} It is attached to the message after this one.", files::SCREEN_PNG), true)
                }
                CallKind::File(file) => self.apply_file(file),
                CallKind::Issue { name, message } => match turn {
                    Some((id, kind)) => (self.file_issue(id, kind, name, message, None), true),
                    None => (compaction::PLAN_ONLY.to_string(), false),
                },
                CallKind::Rejected(complaint) => (complaint.clone(), false),
                CallKind::Terminal(_) | CallKind::Chained => unreachable!("handled above"),
            };
            if let Some((id, _)) = turn {
                self.publish_tool_result(id, call, ok, &content, png);
            }
            results.push(Message::tool_result(&call.id, content));
        }
        results.extend(pictures);
        Some(results)
    }

    /// Say on the page what one tool call answered, and hold its picture for the page to fetch.
    fn publish_tool_result(
        &self,
        turn: u64,
        call: &ToolCall,
        ok: bool,
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
            ok,
            content,
            image: png.is_some(),
        });
        if let Some(png) = png {
            self.published.put_tool_image(seq, png);
        }
    }

    /// Hand a batch to the policy and block until it comes back.
    fn run_batch(&mut self, turn: Option<u64>, calls: Vec<ToolCall>) -> Option<Vec<ToolAnswer>> {
        // Nothing may stop the emulator between here and the answer.
        if turn.is_some() {
            self.published.set_status(RunStatus::RunningTool { name: names(&calls) });
        }
        let answers = self.tool_calls.send(ToolBatch { turn, calls }).ok().and_then(|()| {
            // Blocking: this thread is supposed to wait.
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

    fn account_for(&mut self, id: u64, kind: DecisionKind, completion: &Completion) {
        let usage = completion.usage.unwrap_or_else(|| Usage::estimate(&self.history, completion));
        self.accounting.record(usage, &self.history);
        self.publish_usage(Some(id), kind.label(), usage);
    }

    fn publish_usage(&self, turn: Option<u64>, kind: &str, usage: Usage) {
        self.published.publish_event(UiEventBody::Usage {
            turn,
            kind: kind.to_string(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            estimated: usage.estimated,
        });
    }

    fn over_threshold(&self) -> bool {
        self.accounting.occupancy(&self.history) >= self.config.compact_above
    }

    /// Compact when the history is over `GB_COMPACT_ABOVE`, or regardless when `forced`, and write
    /// down what it did.
    fn compact(&mut self, id: u64, forced: bool) {
        let Some(note) = self.compact_if_needed(forced) else { return };
        if note.summary.is_some() || note.dropped > 0 {
            self.reminder = prompt::PlanReminder::Compacted;
        }
        self.history.note_compaction(id, &note);
        self.history.checkpoint(id, self.accounting.calibration());
    }

    /// One move: the summary of the history as it stands, then the system prompt, the summary and
    /// the last [`compaction::KEEP_TURNS`] turns, or fewer while those alone are over the threshold.
    fn compact_if_needed(&mut self, forced: bool) -> Option<CompactionNote> {
        if !forced && !self.over_threshold() {
            return None;
        }
        let resume = self.published.run_status();
        self.published.set_status(RunStatus::Compacting);
        let before = self.accounting.tokens_in(&self.history);

        let mut dropped = 0;
        let mut summary = None;
        if compaction::has_body(&self.history) {
            let opening = match self.summarise() {
                Summarised::Prose(prose) => {
                    let message = compaction::summary_message(&prose);
                    summary = Some(prose);
                    message
                }
                Summarised::Failed => compaction::dropped_message(),
                Summarised::Abandoned => {
                    self.published.set_status(resume);
                    return None;
                }
            };
            let whole = self.history.to_vec();
            for keep in (0..=compaction::KEEP_TURNS).rev() {
                let mut kept = whole.clone();
                dropped = compaction::replace_with(&mut kept, opening.clone(), keep);
                *self.history = kept;
                if !self.over_threshold() {
                    break;
                }
            }
        }

        let after = self.accounting.tokens_in(&self.history);
        self.published.publish_event(UiEventBody::Compacted { before, after, summarised: summary.is_some() });
        if self.over_threshold() {
            self.published.publish_event(UiEventBody::Notice {
                level: "error",
                message: format!(
                    "compaction could not bring the history under the threshold: it is still {after} \
                     tokens against a limit of {}, and there is nothing left in it that can be \
                     dropped safely. Every request from here may be over the window.",
                    self.accounting.limit(),
                ),
            });
        }
        self.published.set_status(resume);
        Some(CompactionNote { before, after, dropped, summary })
    }

    /// The story so far: asked of the whole history, then once more without its oldest half.
    fn summarise(&mut self) -> Summarised {
        let whole = self.history.to_vec();
        let failure = match self.ask_for_summary(whole.clone(), false) {
            Ok(prose) => return Summarised::Prose(prose),
            Err(Stop::Abandoned) => return Summarised::Abandoned,
            Err(Stop::Genuine(failure)) => failure,
        };
        self.published.publish_event(UiEventBody::Notice {
            level: "error",
            message: format!("could not summarise the history ({failure}); asking again without its oldest turns"),
        });
        let (halved, start_missing) = compaction::without_oldest_half(&whole);
        match self.ask_for_summary(halved, start_missing) {
            Ok(prose) => Summarised::Prose(prose),
            Err(Stop::Abandoned) => Summarised::Abandoned,
            Err(Stop::Genuine(failure)) => {
                self.published.publish_event(UiEventBody::Notice {
                    level: "error",
                    message: format!(
                        "could not summarise the history a second time ({failure}); dropping the \
                         earlier conversation without a summary"
                    ),
                });
                Summarised::Failed
            }
        }
    }

    /// The summary request, looping like a turn until the reply is text: reads are answered,
    /// `plan.md` may be written, and everything else is refused.
    fn ask_for_summary(&mut self, mut messages: Vec<Message>, start_missing: bool) -> Result<String, Stop> {
        messages.push(compaction::instruction(start_missing));
        for step in 0..self.config.max_tool_steps {
            self.published.set_status(RunStatus::Compacting);
            let request = chat_request(&self.config, messages);
            let completion = self.stream_summary(&request)?;
            let usage = completion.usage.unwrap_or_else(|| Usage::estimate(&request.messages, &completion));
            self.accounting.record(usage, &request.messages);
            self.publish_usage(None, "summary", usage);
            messages = request.messages;

            if completion.tool_calls.is_empty() {
                return match completion.content.trim().is_empty() {
                    true => Err(Stop::Genuine("the model returned an empty summary".to_string())),
                    false => Ok(completion.content),
                };
            }
            let classified: Vec<CallKind> = completion.tool_calls.iter().map(summary_call).collect();
            messages.push(Message::assistant(completion.content.clone(), completion.tool_calls.clone()));
            let answers = self.answer_calls(None, &completion.tool_calls, &classified).ok_or(Stop::Abandoned)?;
            messages.extend(answers);
            if step + 2 >= self.config.max_tool_steps {
                messages.push(Message::user(compaction::OUT_OF_READS));
            }
        }
        Err(Stop::Genuine(format!(
            "the model was still calling tools after {} requests",
            self.config.max_tool_steps,
        )))
    }

    /// One summary request, retried through every transient failure and parked as a turn is.
    fn stream_summary(&mut self, request: &ChatRequest) -> Result<Completion, Stop> {
        loop {
            if reset_waiting(&self.reset) {
                return Err(Stop::Abandoned);
            }
            let published = Arc::clone(&self.published);
            let reset = Arc::clone(&self.reset);
            let result = stream_with_retries(
                self.retry,
                self.endpoint.as_ref(),
                request,
                // Not published: the summary is bookkeeping, not the model talking.
                &mut |_| {},
                &|| reset_waiting(&reset),
                &mut |retry| {
                    published.publish_event(UiEventBody::Notice {
                        level: "warn",
                        message: format!(
                            "compaction attempt {}/{} failed ({}); retrying in {:?}",
                            retry.attempt, retry.of, retry.failure, retry.waiting,
                        ),
                    });
                },
            );
            let failure = match result {
                Ok(completion) => {
                    self.refused_in_a_row = 0;
                    return Ok(completion);
                }
                Err(LlmError::Cancelled) => return Err(Stop::Abandoned),
                Err(failure) => failure,
            };
            match &failure {
                LlmError::RateLimited { resets_at_ms: Some(until_ms), message } => {
                    if !self.park_until(Watch::Compaction, *until_ms, Park::Quota, message) {
                        return Err(Stop::Abandoned);
                    }
                }
                refusal if refusal.is_account_refusal() => {
                    self.refused_in_a_row += 1;
                    match self.refusal_park.window(self.refused_in_a_row) {
                        Some(window) => {
                            let until_ms = now_ms().saturating_add(window.as_millis() as u64);
                            let park = Park::Refused { in_a_row: self.refused_in_a_row };
                            if !self.park_until(Watch::Compaction, until_ms, park, &refusal.to_string()) {
                                return Err(Stop::Abandoned);
                            }
                        }
                        None => self.wait_to_ask_again(refusal)?,
                    }
                }
                transient if transient.is_retryable() || matches!(transient, LlmError::Timeout(_)) => {
                    self.refused_in_a_row = 0;
                    self.wait_to_ask_again(transient)?;
                }
                _ => {
                    self.refused_in_a_row = 0;
                    return Err(Stop::Genuine(failure.to_string()));
                }
            }
        }
    }

    /// The pause between two rounds of a summary request's retries, cut short by a reset.
    fn wait_to_ask_again(&self, failure: &LlmError) -> Result<(), Stop> {
        let waiting = self.retry.backoff_for(self.retry.attempts);
        self.published.publish_event(UiEventBody::Notice {
            level: "warn",
            message: format!(
                "the summary request failed ({failure}); asking again in {}",
                describe_wait(waiting.as_millis() as u64),
            ),
        });
        let until = std::time::Instant::now() + waiting;
        while std::time::Instant::now() < until {
            if reset_waiting(&self.reset) {
                return Err(Stop::Abandoned);
            }
            std::thread::sleep(PARK_SLICE.min(until - std::time::Instant::now()));
        }
        Ok(())
    }
}

/// Every request the worker sends, a turn's or a summary's: the same tools and settings, so all of
/// them share one cached prefix.
fn chat_request(config: &LlmConfig, messages: Vec<Message>) -> ChatRequest {
    ChatRequest {
        model: config.model.clone(),
        messages,
        tools: tools::catalogue(),
        parallel_tool_calls: Some(true),
        max_tokens: config.max_tokens,
        reasoning_effort: config.reasoning_effort.clone(),
        reasoning: config.reasoning_budget,
        temperature: config.temperature,
        stream: true,
        stream_options: StreamOptions { include_usage: true },
    }
}

/// What one call in a summary request is: reads are answered and `plan.md` may be written; an
/// action and any other write are refused, since the request asks for prose.
fn summary_call(call: &ToolCall) -> CallKind {
    if call.function.name == tools::CHOOSE_ACTION {
        return CallKind::Rejected(compaction::NOT_AN_ACTION.to_string());
    }
    match tools::classify_call(call) {
        CallKind::File(FileCall::Write { node, .. } | FileCall::Edit { node, .. }) if node != Node::Plan => {
            CallKind::Rejected(compaction::PLAN_ONLY.to_string())
        }
        CallKind::Issue { .. } => CallKind::Rejected(compaction::PLAN_ONLY.to_string()),
        other => other,
    }
}

fn reset_waiting(reset: &Resets) -> bool {
    reset.lock().map(|cell| cell.is_some()).unwrap_or(false)
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
        // The chain is on the line the page shows, or three actions read as one.
        Terminal::ChooseAction { id, then } => {
            let mut line = format!("choose_action {id}");
            if !then.is_empty() {
                line.push_str(&format!(", then {}", then.join(", ")));
            }
            line
        }
        // So is a take-over, since a scripted battle is otherwise invisible from outside.
        Terminal::ChooseBattleAction { id, take_over } => match take_over {
            true => format!("choose_action {id} (taking over this battle)"),
            false => format!("choose_action {id}"),
        },
        Terminal::PressButtons { buttons } => format!(
            "choose_action press:{}",
            buttons.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(","),
        ),
        Terminal::SetNickname { name } => match name {
            Some(name) => format!("choose_action {name} (the nickname)"),
            None => "choose_action NONE (keep the species name)".to_string(),
        },
        Terminal::BuyItem { item, then } => match item {
            Some(item) => match then.is_empty() {
                true => format!("choose_action buy {item}"),
                false => format!(
                    "choose_action buy {item}, then {}",
                    then.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", "),
                ),
            },
            None => "choose_action leave".to_string(),
        },
        Terminal::ForgetMove { slot } => match slot {
            Some(slot) => format!("choose_action forget:{slot}"),
            None => "choose_action keep".to_string(),
        },
        Terminal::MimicMove { slot } => format!("choose_action mimic:{slot}"),
        Terminal::Wait { ticks } => format!("choose_action wait*{ticks}"),
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

    struct Summarises;

    impl ChatEndpoint for Summarises {
        fn stream_completion(
            &self,
            _: &ChatRequest,
            _: &mut dyn FnMut(Fragment<'_>),
            _: &dyn Fn() -> bool,
        ) -> Result<Completion, LlmError> {
            Ok(Completion { content: "I am in Pallet Town.".into(), ..Completion::default() })
        }
    }

    fn worker(context_limit: u64, compact_above: f64) -> Worker {
        let config = LlmConfig {
            provider: crate::llm::config::Provider::OpenAi,
            base_url: "http://x".into(),
            api_key: "k".into(),
            model: "m".into(),
            context_limit,
            compact_above,
            temperature: None,
            max_tool_steps: 4,
            request_timeout: Duration::from_secs(crate::llm::config::DEFAULT_REQUEST_TIMEOUT_SECS),
            max_tokens: None,
            reasoning_effort: None,
            reasoning_budget: None,
            stuck_timeout: None,
        };
        let (worker, _) = channels(
            Box::new(Summarises),
            config,
            Published::new(),
            Plan::open(None),
            BattleScript::open(None),
            History::open(None),
        );
        worker
    }

    /// A history of `turns` turns of `bulk` prose each, behind the system prompt.
    fn turns(worker: &mut Worker, turns: usize, bulk: &str) {
        for turn in 1..=turns {
            worker.history.push(Message::user(format!("{}turn {turn}\n{bulk}", prompt::DECISION)));
            worker.history.push(Message::assistant("done".into(), vec![]));
        }
    }

    fn notices(published: &Published, run: impl FnOnce()) -> Vec<String> {
        let mut events = published.subscribe_events();
        run();
        std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event.body {
                UiEventBody::Notice { message, .. } => Some(message),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_compaction_keeps_the_summary_and_the_last_two_turns() {
        let bulk = "a long turn. ".repeat(600);
        let mut probe = worker(1, 1.0);
        turns(&mut probe, 40, &bulk);
        let full = probe.accounting.tokens_in(&probe.history);

        let mut worker = worker(full, 0.5);
        *worker.history = probe.history.to_vec();
        let was = worker.history.len();
        let note = worker.compact_if_needed(false).expect("a history at its limit compacts");

        assert!(note.summary.is_some());
        assert!(note.after < note.before);
        assert_eq!(worker.history.len(), 2 + 2 * 2, "the system prompt, the summary and two turns");
        assert!(worker.history[2].text().unwrap().contains("turn 39"));
        assert_eq!(note.dropped, was - (worker.history.len() - 1), "every message that left, and not the summary");
    }

    #[test]
    fn a_compaction_under_the_threshold_happens_only_when_forced() {
        let mut worker = worker(1_000_000, 0.85);
        turns(&mut worker, 4, "short");
        assert!(worker.compact_if_needed(false).is_none());
        let note = worker.compact_if_needed(true).expect("forced");
        assert!(note.summary.is_some());
        assert_eq!(worker.history.len(), 2 + 2 * 2);
    }

    /// The last two turns alone over the threshold: one is kept, then none.
    #[test]
    fn a_compaction_keeps_fewer_turns_while_those_alone_are_too_big() {
        let bulk = "a long turn. ".repeat(600);
        let mut probe = worker(1, 1.0);
        let system = probe.accounting.tokens_in(&probe.history);
        turns(&mut probe, 1, &bulk);
        let turn = probe.accounting.tokens_in(&probe.history) - system;

        // Two turns are over half the window, one is not.
        let mut worker = worker(2 * system + 3 * turn, 0.5);
        turns(&mut worker, 4, &bulk);
        worker.compact_if_needed(false).expect("compacts");
        assert_eq!(worker.history.len(), 2 + 2, "one turn kept");
        assert!(worker.history[2].text().unwrap().contains("turn 4"));

        // Not even one fits.
        let mut worker = self::worker(2 * system + turn, 0.5);
        turns(&mut worker, 4, &bulk);
        let said = notices(&Arc::clone(&worker.published), || {
            worker.compact_if_needed(false).expect("compacts");
        });
        assert_eq!(worker.history.len(), 2, "the system prompt and the summary");
        assert!(compaction::is_summary(&worker.history[1]));
        assert!(!worker.over_threshold(), "and that fits: {said:?}");
    }

    #[test]
    fn a_compaction_that_cannot_get_under_the_threshold_says_so_in_one_sentence() {
        let mut worker = worker(1, 0.5);
        let published = Arc::clone(&worker.published);
        let said = notices(&published, || {
            let note = worker.compact_if_needed(false).expect("a system prompt alone is over a one-token limit");
            assert!(note.summary.is_none(), "there was nothing to summarise");
        });
        let notice = said.iter().find(|m| m.starts_with("compaction could not bring the history under")).expect("the notice");
        assert!(!notice.contains("  "), "a `\\` was eaten out of a continued literal: {notice}");
    }
}
