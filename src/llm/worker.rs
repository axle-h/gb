//! **W4 / §7.3** — the turn loop, on a plain blocking `std::thread`.
//!
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
//!   └─ over GB_COMPACT_ABOVE of the context? → compact (W6 / §9)
//! ```
//!
//! **Cancellation is a generation counter checked at exactly two points**, because those are the
//! only two places a turn can be sitting: inside the SSE read (every line — see
//! [`protocol::read_stream`](crate::llm::protocol::read_stream)) and blocked on a tool result. No
//! `select!`, no async, no cancellation token.
//!
//! **Rollback is one step.** On cancellation the last assistant message — the one carrying tool calls
//! that were never serviced — is dropped, and the turn is abandoned. Every remaining `tool_call` in
//! the history already has its matching result, so the history is well-formed *by construction* and
//! the next request cannot 400. That guarantee is what makes single-step rollback sufficient, and it
//! rests on a batch being serviced **all-or-nothing** (§2.1): the whole batch is answered from one
//! observed `GameState` at one poll, so a partial batch cannot happen.

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
use crate::web::published::{Published, RunStatus, TodoView, UiEventBody, now_ms};

/// One question, from the policy to the worker.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    /// The generation this turn belongs to. It is stale the moment
    /// [`TurnHandles::generation`] moves past it.
    pub id: u64,
    pub kind: DecisionKind,
    /// The rendered user message — see [`prompt::situation`].
    pub situation: String,
    /// A one-line description for the UI, so a viewer sees what is being decided without the
    /// thousand tokens that were sent to decide it.
    pub headline: String,
    /// The ids this turn's situation offered, in the order it offered them.
    ///
    /// ⚠️ **Carried so `tools::classify` can refuse an id the model invented while the turn is still
    /// running.** Without it a bad id is accepted here, published as a decision and refused by
    /// `resolve_overworld` one thread later, which costs the whole turn rather than one completion.
    /// Empty for the kinds that have no menu.
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

/// Re-exported under the name the plan uses.
pub type Decision = Terminal;

/// Read tool calls from one assistant message, to be answered at one poll.
#[derive(Debug, Clone)]
pub struct ToolBatch {
    pub turn: u64,
    pub calls: Vec<ToolCall>,
}

/// One read tool's answer, as the emulator thread hands it back.
///
/// ⚠️ **`map` is data, not a picture.** `read_map` answers with a `MetaTileMap` and the *worker*
/// draws it — the emulator thread cannot afford to encode a PNG of up to three quarters of a million
/// pixels while the game is running. See [`crate::llm::map_image`]'s module note. The clone is one
/// the policy is already making once per poll, so this costs the emulator thread nothing new.
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
    /// The decision kind changed before the batch could be serviced. The tools were **not** run.
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
    /// **`POST /api/new-run`** — a pending "the game restarted" notice. See [`Restart`].
    pub restart: Restarts,
    /// The armed battle script, which the policy runs on its own thread rather than asking for.
    /// See [`crate::llm::battle_script::Live`].
    pub live_script: Arc<battle_script::Live>,
}

/// The game has been restarted underneath a live worker, so the conversation is now about a game
/// that no longer exists and has to start again.
///
/// ⚠️ **A shared cell rather than a channel, because the worker cannot select.** It blocks on
/// `turns.recv()`, so a second `Receiver` would need a `select` that `std::sync::mpsc` does not
/// have. This is consumed at the top of [`Worker::run_one`] instead, which is reached promptly
/// because the policy bumps the generation on its way past — cancelling whatever was in flight.
#[derive(Debug, Clone, Default)]
pub struct Restart {
    /// The **new** run directory, which is where the model's plan now lives. `None` keeps it in
    /// memory only, as the tests do.
    pub run_dir: Option<PathBuf>,
}

/// The cell a [`Restart`] waits in. Written by the policy, taken by the worker.
pub type Restarts = Arc<Mutex<Option<Restart>>>;

impl TurnHandles {
    /// Abandon whatever is in flight and claim the next turn id.
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
}

/// After a failure that is not the model's fault — the endpoint is down, the key is wrong — the turn
/// resolves to a wait of this many agent ticks (two seconds of game time) rather than one. Retrying
/// a dead endpoint fifty times a second would turn an outage into a log flood, and the game is not
/// going anywhere.
const FAILURE_WAIT_TICKS: u16 = 100;

/// The longest [`Worker::park_until`] will stop the run for, however far off the endpoint says its
/// quota reopens. A daily cap is at most 24 h away; anything beyond that is a header we have
/// misread, and the answer to that is to try again rather than to sit out the week.
const MAX_PARK: Duration = Duration::from_secs(25 * 60 * 60);

/// How finely the park is chopped. Long enough to cost nothing over hours, short enough that a
/// `POST /api/new-run` arriving mid-park is felt as promptly as it would be mid-turn.
const PARK_SLICE: Duration = Duration::from_millis(200);

/// A duration as a viewer would say it. Used in the notice that announces a park, so it is read once
/// per park rather than per tick, and the page renders its own live countdown from `until_ms`.
fn describe_wait(ms: u64) -> String {
    let seconds = ms / 1000;
    match (seconds / 3600, (seconds % 3600) / 60, seconds % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// How much of a tool's answer the page and the transcript keep. The model is always sent all of it;
/// this is the copy that is broadcast to every viewer and appended to `transcript.jsonl`, and a
/// `read_map` answer runs to a few kilobytes of JSON. Long enough to read a party or a route in full.
const MAX_TOOL_RESULT: usize = 2_000;

/// Where [`Worker::trim_history`] drops back to. That function is W4's stopgap, kept as W6's **last
/// resort**: it throws whole turns away from the front of the history rather than summarising them,
/// and it runs only when a summarisation could not be had — the endpoint was down, or the model
/// returned nothing. Losing the oldest turns is much worse than a summary and much better than a
/// run that 400s on every request from here on.
const TRIM_TO: f64 = 0.50;

/// How many overworld turns an unchanged plan may sit at before a fresh copy is appended.
///
/// ⚠️ **The plan is emitted on change, so a model that never edits it never sees it near the tail** —
/// and that is exactly the model this is for. Both deployed runs were it: 258 turns with a single
/// `todo_set` on turn 1 and no edit after it, and 2430 turns before that with sixteen `todo_set`s
/// and one `todo_complete`. The plan ends up buried under the whole conversation, so the list the
/// model is meant to be revising is the least recent thing in the request.
///
/// ⚠️ **The number is set by history growth, not by cache cost, and that is a change.** It used to
/// be 15 on the argument that re-emitting *moved* the plan — removing the old copy and paying a
/// re-prefill of everything after it, a couple of thousand uncached tokens a time. [`Worker::sync_plan`]
/// no longer moves anything (see its ⚠️), so what a refresh actually costs is one ~150-token message
/// prefilled once and cached from then on. What it buys against is history growth: a turn is one to
/// two thousand tokens, so a copy every tenth overworld turn is on the order of 1% more context to
/// carry and to compact. Every turn would be ~10%, which is a compaction bought sooner for a message
/// the per-turn [`prompt::PLAN_UNCHANGED`] note already covers.
pub const PLAN_REFRESH_TURNS: u32 = 10;

pub struct Worker {
    endpoint: Box<dyn ChatEndpoint>,
    config: LlmConfig,
    published: Arc<Published>,
    retry: RetryPolicy,

    generation: Arc<AtomicU64>,
    turns: Receiver<TurnRequest>,
    outcomes: Sender<TurnOutcome>,
    tool_calls: Sender<ToolBatch>,
    tool_results: Receiver<ToolBatchResult>,

    /// The conversation, and the two files it is kept in — see [`crate::llm::history`]. Index 0 is
    /// the system prompt; it is never removed and, since **W6b**'s plan moved out of it, never
    /// rewritten either — see [`prompt::system_message`].
    ///
    /// It reads as the `Vec<Message>` it used to be, through `Deref`. That is sound because
    /// persistence here is checkpoint-based rather than write-through: nothing intercepts a
    /// mutation, [`Self::run_one`] simply writes the vector down once a turn.
    history: History,
    /// **W6b / §10** — the model's plan. Answered here rather than at the policy poll: none of it
    /// needs the emulator.
    todo: TodoList,
    /// The model's battle script, and the cell the policy reads it through. Answered here for the
    /// reason the plan is: validation runs the script six times over hand-built states and none of
    /// it touches the emulator. See [`crate::llm::battle_script`].
    battle_script: BattleScript,
    live_script: Arc<battle_script::Live>,
    /// What the page was last told the plan is, so [`Self::publish_todo`] can be called from every
    /// moment it might have changed without publishing the same list twice.
    published_plan: Option<Vec<TodoView>>,
    /// Turns since the plan message was last (re)placed at the tail of the history — see
    /// [`Self::sync_plan`] and [`PLAN_REFRESH_TURNS`].
    turns_since_plan: u32,
    /// **W6** — tokens reported, tokens spent, and how far our own estimate is from the endpoint's.
    accounting: Accounting,
    /// **`POST /api/new-run`** — taken at the top of every turn. See [`Restart`].
    restart: Restarts,
    /// Where the `press_buttons` records go — see [`crate::llm::incident`].
    ///
    /// ⚠️ **`CurrentRun` rather than a `PathBuf`, and `None` rather than a default.** The cell
    /// follows a reset on its own, so a record written after `POST /api/new-run` lands in the run
    /// that is playing rather than the one that was checkpointed and set aside. `None` is every
    /// test and the in-process worker in `LlmPolicy`: no run directory, nothing recorded.
    run: Option<Arc<CurrentRun>>,
}

/// Build the worker and its counterpart handles. The thread is started by [`Worker::spawn`]; this is
/// separate so a test can drive [`Worker::run_one`] on its own thread and control the timing.
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
    let restart: Restarts = Arc::new(Mutex::new(None));
    // ⚠️ **Armed from the file at construction, not on the first `set_battle_script`.** A resumed
    // run's script is on disk and the model has no reason to send it again, so a cell that started
    // empty would fight the rest of the run one paid turn at a time and nothing would say why.
    let live_script = Arc::new(battle_script::Live::default());
    if battle_script.armed() {
        live_script.arm(battle_script.source().map(str::to_string));
    }

    // ⚠️ **The calibration comes back with the conversation, and nothing else does.** A restored
    // history is measured on the endpoint's scale or not at all: see `Accounting::resumed`, which
    // also says why the token totals must *not* be restored with it.
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
        turns_since_plan,
        accounting,
        restart: Arc::clone(&restart),
        run: None,
    };
    let handles = TurnHandles {
        turns: turn_tx,
        outcomes: outcome_rx,
        tool_calls: call_rx,
        tool_results: result_tx,
        generation,
        restart,
        live_script,
    };
    (worker, handles)
}

impl Worker {
    /// Point the `press_buttons` records at a run directory. Without it nothing is recorded, which
    /// is what every test wants and what `gb serve` never does.
    pub fn with_run(mut self, run: Arc<CurrentRun>) -> Self {
        self.run = Some(run);
        self
    }

    /// File a report: the screen, the run's state, a save state and the last few turns of
    /// conversation. See [`crate::llm::incident`] for what is in it and why.
    ///
    /// ⚠️ **Nothing here may fail a turn.** For a press the decision is already made and the game is
    /// waiting for it; for an issue the turn is still running and owes the model a tool result. A
    /// full disk is worth a line on stderr and no more, in both cases.
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
    ///
    /// ⚠️ **The answer must not read like a fix.** The model has just said the agent is wrong; if
    /// the reply sounds like something changed it will wait for the change, and the next turn will
    /// look identical. So it says what actually happened — filed, nobody is coming, keep playing —
    /// which is also what [`tools::report_issue_spec`] promised.
    ///
    /// ⚠️ **The conversation slice this records ends one message earlier than a press's does**, and
    /// it cannot do otherwise: the string returned here *is* the tool result, so it has to be built
    /// before the result it becomes can be appended. The assistant message carrying the report is in
    /// the slice — it is pushed before classification — so what is missing is only this constant
    /// sentence, which a person reading the record already knows. A press has no such constraint,
    /// which is why it is deliberately recorded after its results land.
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

    /// Run the loop on a new thread. It ends when the policy is dropped, which closes the channel.
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

    /// Start the conversation again, about the game that is playing now.
    ///
    /// The three things thrown away are the three that are about the old game: the history, the
    /// plan read out of the old run directory, and the token accounting that was measuring that
    /// history. Everything else — the endpoint, the config, the retry policy, the channels — belongs
    /// to the *process*, and rebuilding any of it would mean rebuilding this thread.
    fn apply_restart(&mut self, restart: Restart) {
        self.todo = TodoList::open(restart.run_dir.as_deref());
        self.publish_todo();
        // ⚠️ **Reopened against the new directory, and the cell re-armed from what it finds.**
        // Without this a `POST /api/new-run` leaves the *old* game's script deciding the new game's
        // battles, and `set_battle_script` writing into a run that has been set aside.
        self.battle_script = BattleScript::open(restart.run_dir.as_deref());
        self.live_script.arm(match self.battle_script.armed() {
            true => self.battle_script.source().map(str::to_string),
            false => None,
        });
        // ⚠️ **`fresh`, never `open`.** The two differ in exactly one way and it is the whole point
        // of having both: `open` would read the new run directory back, and this is the one call
        // site where reading is wrong. Today `RunDir::open`'s fresh path always mints an empty
        // directory, so both would behave the same by luck rather than by construction.
        self.history = History::fresh(restart.run_dir.as_deref());
        // The history the counter was measured against is gone, and `sync_plan` finds no plan in the
        // fresh one, so it appends immediately — leaving a stale count would make the *next* refresh
        // fall due at the wrong time.
        self.turns_since_plan = 0;
        self.accounting = Accounting::new(self.config.context_limit);
        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: "the game restarted; the conversation starts again from the system prompt"
                .to_string(),
        });
    }

    /// One turn, start to finish. Public so a test can step the worker without a thread.
    pub fn run_one(&mut self, request: TurnRequest) {
        // ⚠️ **Before the situation is appended, not after.** This is the moment the history for the
        // next turn is chosen, and after a restart the old history describes a game that no longer
        // exists — a party, a map and a TODO list belonging to a run that has been checkpointed and
        // set aside.
        if let Some(restart) = self.restart.lock().ok().and_then(|mut cell| cell.take()) {
            self.apply_restart(restart);
        }
        // ⚠️ **The policy disarms in memory and this is what makes it durable.** The emulator thread
        // cannot write the run directory — one writer per run, the rule `transcript` and `history`
        // both keep — so it leaves the reason here and the file learns about it at the top of the
        // very turn the failure caused. Anything later and a restart re-arms a broken script.
        if let Some(why) = self.live_script.take_failure() {
            self.battle_script.disarm(&why);
            self.published.publish_event(UiEventBody::Notice {
                level: "warn",
                message: format!("the battle script was disarmed: {why}"),
            });
        }
        let TurnRequest { id, kind, situation, headline, menu } = request;
        self.published.publish_event(UiEventBody::TurnStarted { turn: id, kind: kind.label(), headline });

        let carried = self.sync_plan(kind);
        self.publish_todo();
        // ⚠️ **The turn that does not carry the plan has to say so.** The copy in the history is
        // still there, but it is behind however many turns have passed since it last changed, and a
        // model reading a fifty-turn conversation does not treat a message that far back as a live
        // instruction. One line at the bottom of the situation costs nothing at the cache — the
        // situation is new tokens every turn either way — and it is the only reminder on a turn that
        // is not an overworld one, since those never refresh.
        self.history.push(Message::user(match carried {
            true => situation,
            false => format!("{situation}\n{}\n", prompt::PLAN_UNCHANGED),
        }));
        let outcome = self.decide(id, kind, &menu);
        // ⚠️ **Before the outcome is sent, not at the end of the turn.** The moment the emulator
        // thread has a `TurnOutcome` it may act on it, and an action that wins the game has
        // `hall_of_fame::archive` copy this whole directory on the very next tick — so everything
        // below the send races that copy and usually loses. Publishing the `Decision` is below it,
        // and so is `compact_if_needed`, which can be an entire summarising completion. Writing
        // first makes durability precede visibility and closes the race by construction: it is the
        // same argument that made the archiver *follow* the transcript rather than `fs::copy` it.
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
            // Abandoned. The policy has already moved on to a different kind — it bumped the
            // generation, which is how we found out — so there is nothing to send it.
            None => {
                self.published.publish_event(UiEventBody::TurnCancelled {
                    turn: id,
                    reason: "the game moved on to a different decision".to_string(),
                });
            }
        }
        // ⚠️ An error stays on the board until the next turn starts. Flicking straight back to
        // `Playing` would make a run that is failing every request look, to anyone watching, exactly
        // like a run that is playing quietly.
        if !matches!(self.published.run_status(), RunStatus::Error { .. }) {
            self.published.set_status(RunStatus::Playing);
        }
        // ⚠️ **The log is flushed above, before this runs, and that ordering is what makes the
        // watermark sound**: everything a compaction is about to destroy has already been written
        // down. `note_compaction` puts the watermark back, and the second checkpoint stores the
        // shortened history so a restart resumes on the compacted one rather than replaying the
        // turns it just paid a completion to summarise away.
        if let Some(note) = self.compact_if_needed() {
            self.history.note_compaction(id, &note);
            self.history.checkpoint(id, self.accounting.calibration(), self.turns_since_plan);
        }
    }

    /// **W6b / §10** — put the model's plan in front of it, in exactly one place, at the cheapest
    /// moment.
    ///
    /// Called immediately before a turn's situation is appended, so the plan is the second-newest
    /// message when the request goes out: recent enough to be read as current, and never the last
    /// thing, which the contract has to be.
    ///
    /// ⚠️ **Nothing here ever removes or rewrites a message: the history is append-only, and that is
    /// the property the whole prompt cache rests on.** A hosted endpoint caches on the *prefix*, so
    /// the cost of any edit is a re-prefill of everything after the edited position — and a message
    /// removed from the middle of a fifty-thousand-token conversation is not a cheap edit, it is a
    /// couple of thousand uncached tokens, every time the model touches its own list.
    ///
    /// This used to remove the stale copy so there would only ever be one. ⚠️ **Measured, the two are
    /// within about 20% of each other and leaving is the cheaper side** — the arithmetic is worth
    /// writing down because "one copy" sounds obviously right and is not. The plan is 1283 bytes
    /// (~320 tokens, `probe_turn_requests`) and sits immediately before the *previous* turn's
    /// situation, so removing it re-prefills one turn: the deployed run compacted 38 times across
    /// 2427 decisions, i.e. ~64 turns to grow a history from ~5 k to `GB_CONTEXT_LIMIT` × 0.85, so a
    /// turn is ~1250 tokens. Leaving the copy instead costs those 320 tokens on every request until
    /// compaction takes it, which averages half a cycle — ~32 requests — and they are *cached*, worth
    /// a tenth of an uncached one on this endpoint. So ~1250 against ~1020, plus the ~4% of the
    /// context that a cycle's worth of stale copies occupies and therefore compacts sooner.
    ///
    /// ⚠️ **The tie is broken on structure rather than on the 20%.** Appending has no exceptions to
    /// get wrong: after the system prompt this conversation only ever grows at the end, so there is
    /// no position in it whose cache anything has to reason about. The removal's advantage also
    /// depends entirely on the endpoint's cached-token discount, which is a number we do not
    /// control — at a 2× discount rather than 10× the removal wins outright.
    ///
    /// ⚠️ **Which makes "the plan" the *last* copy, not the only one.** `rposition`, and
    /// [`TodoList::render`](crate::llm::todo::TodoList::render) says in the message itself that it
    /// replaces any earlier one — a model reading four `## Your plan` blocks needs to be told which
    /// wins, and chronology is the answer a conversation already implies.
    ///
    /// Three cases, and the common one costs nothing at all:
    ///
    /// - the newest copy already says this, and no refresh is due → **do nothing**;
    /// - it says something else, or [`PLAN_REFRESH_TURNS`] has come round → append a fresh copy;
    /// - there is no copy — a compaction took it, or this is the first turn → append one.
    ///
    /// The alternative both of these replaced was re-rendering the plan into message 0, which
    /// invalidated the entire conversation every time the model touched its own list. See
    /// [`prompt::system_message`].
    ///
    /// Returns whether the plan is at the tail of the history for *this* request — false means the
    /// newest copy the model can see is further back and unchanged, which is what
    /// [`prompt::PLAN_UNCHANGED`] is appended to the situation to say.
    fn sync_plan(&mut self, kind: DecisionKind) -> bool {
        let plan = prompt::plan_message(&self.todo);
        // ⚠️ **The periodic refresh is an overworld thing; the edit-driven one is not.** A refresh
        // buys the model a fresh look at a list it has not touched, and there is nothing to do about
        // that list in the middle of a battle, on a naming screen or at a mart — those turns are one
        // question with one answer, and a re-prefill bought there is a re-prefill wasted. An *edit*
        // is different: the todo tools are offered on every kind (`non_terminal_names` chains them
        // unconditionally), so a plan changed during a battle has to be corrected in the history at
        // once or the next overworld turn reads a stale one.
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

    /// Service one battle-script call, returning the sentence the model is shown.
    ///
    /// ⚠️ **The cell is re-armed from the *file's* view rather than from the call**, so the policy
    /// can only ever be running a script that was validated and written down. A `set` that was
    /// refused leaves both exactly as they were.
    fn apply_battle_script(&mut self, call: tools::BattleScriptCall) -> String {
        match call {
            tools::BattleScriptCall::Docs => battle_script::DOCS.to_string(),
            tools::BattleScriptCall::Read => self.battle_script.read(),
            tools::BattleScriptCall::Set(source) => {
                let answer = self.battle_script.set(source.as_deref());
                self.live_script.arm(match self.battle_script.armed() {
                    true => self.battle_script.source().map(str::to_string),
                    false => None,
                });
                answer
            }
        }
    }

    /// One TODO call, applied and published. The UI gets the whole list — a viewer reads it as what
    /// the run is trying to do — while [`TodoList::render`] gives the model the shorter version.
    fn apply_todo(&mut self, call: crate::llm::todo::TodoCall) -> String {
        let answer = self.todo.apply(call);
        self.publish_todo();
        answer
    }

    /// Tell the page, if there is anything new to tell it.
    ///
    /// ⚠️ **The dedupe is what makes it safe to call from everywhere**, and it has to be, because
    /// there are three unrelated moments the page can be behind: the model edited the list; a
    /// `POST /api/new-run` swapped the list for the new run's; and — the one that is easy to miss —
    /// **the process opened a run that already had a plan on disk**. That last case has no event of
    /// its own at all, so without a publish from the first turn a resumed run would show an empty
    /// panel until the model next happened to touch its own list, which can be an hour.
    fn publish_todo(&mut self) {
        let items: Vec<TodoView> = self.todo.items().iter().map(TodoView::from).collect();
        if self.published_plan.as_ref() == Some(&items) {
            return;
        }
        self.published_plan = Some(items.clone());
        self.published.publish_event(UiEventBody::Plan { items });
    }

    /// Stop the run until `until_ms`, and stop the emulator with it. `false` if the turn was
    /// cancelled while waiting, in which case the caller must abandon it.
    ///
    /// ⚠️ **This is the one place the emulator is deliberately paused, and it is safe for exactly
    /// the reason the general rule forbids it.** `CLAUDE.md`'s ⚠️ is that a pause spanning an LLM
    /// *tool call* deadlocks the run: a tool batch is answered by `Policy::service_tools`, which only
    /// runs when `gb.run` advances the agent, so a paused emulator can never answer one. Here there
    /// is nothing outstanding to answer — the request failed before any tool was called, and the
    /// next thing this turn does is ask again — so nothing is waiting on the emulator and the pause
    /// cannot deadlock anything. Anything that parks *while a tool batch is in flight* would.
    ///
    /// ⚠️ **The release is unconditional and must stay that way.** A return that leaves
    /// `set_throttled_until` set stops the emulator for the rest of the process, and the page would
    /// show a paused screen that no reset ever clears.
    fn park_until(&mut self, id: u64, until_ms: u64, message: &str) -> bool {
        let now = now_ms();
        // ⚠️ Clamped, because the wait is driven by a number the *endpoint* chose. A header that is
        // wrong, or a unit sniffed wrongly out of it (`protocol::reset_at_ms`), would otherwise park
        // the run past the heat death of the sun with no way back but a restart.
        let until_ms = until_ms.min(now.saturating_add(MAX_PARK.as_millis() as u64));
        if until_ms <= now {
            return true;
        }

        self.published.publish_event(UiEventBody::Notice {
            level: "warn",
            message: format!(
                "the endpoint's quota is spent, so the run is paused for {}; the game is stopped and \
                 nothing is lost. {message}",
                describe_wait(until_ms - now),
            ),
        });
        self.published.set_status(RunStatus::Throttled { until_ms, message: message.to_string() });
        // ⚠️ Last, after the status: the emulator thread reads this one every tick, and a page that
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
                message: "the quota window reopened; the run is resuming where it stopped".to_string(),
            });
        }
        !cancelled
    }

    /// `None` means the turn was cancelled and abandoned.
    ///
    /// The second half of the pair is the model's own account of what it just decided and why
    /// ([`tools::call_summary`]) — absent when the model left the argument out, and absent by
    /// construction on the forced wait, which is the loop's decision rather than the model's.
    fn decide(&mut self, id: u64, kind: DecisionKind, menu: &[String]) -> Option<(Terminal, Option<String>)> {
        let specs = tools::for_kind(kind);
        let mut nudged = false;

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
                // ⚠️ **A loop, so a parked turn asks the *same* question when the quota reopens.**
                // That is only sound because the emulator is stopped for the duration (see
                // `park_until`): the situation this request describes is still on screen when we
                // wake, however many hours later, so re-sending it is not stale, it is the point.
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
                        // The quota is spent and the endpoint dated its reopening: stop asking, stop
                        // the game with it, and put the same question again when it opens.
                        // `park_until` answers `false` only if the turn was cancelled while waiting.
                        Err(LlmError::RateLimited { resets_at_ms: Some(until_ms), message }) => {
                            if !self.park_until(id, until_ms, &message) {
                                break Err(LlmError::Cancelled);
                            }
                            // ⚠️ Back to `AwaitingLlm` *before* asking again. The status is only set
                            // at the top of this loop's parent, so without this the page keeps the
                            // PAUSED plate up over a game that has already resumed, until the first
                            // token of the reply happens to arrive and move it to `Streaming`.
                            self.published.set_status(RunStatus::AwaitingLlm { kind: kind.label() });
                        }
                        settled => break settled,
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
                        // The request is still the last thing in the history and was never answered.
                        // Drop it so the next turn does not open on a dangling question.
                        self.history.pop_if_user();
                        return Some((Terminal::Wait { ticks: FAILURE_WAIT_TICKS }, None));
                    }
                }
            };

            self.account_for(&completion);
            self.history.push(Message::assistant(completion.content.clone(), completion.tool_calls.clone()));

            if completion.tool_calls.is_empty() {
                // §7.5's fallback. One nudge quoting the rule, then the rule is enforced for it.
                // `length` means `GB_MAX_TOKENS` stopped it mid-thought rather than the model
                // choosing to say nothing, which is a different correction to ask for.
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

            // Published *after* classification, not before, so each call arrives at the page already
            // labelled — a rejected call reads as rejected rather than as one that never answered.
            for (call, classification) in completion.tool_calls.iter().zip(&classified) {
                self.published.publish_event(UiEventBody::ToolCall {
                    turn: id,
                    id: call.id.clone(),
                    kind: classification.label(),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
            }

            // A message that mixes reads with a terminal call ends the turn: the model has already
            // committed, so running the reads would be answering a question it stopped asking. They
            // still get a result message, because every `tool_call` needs one.
            //
            // ⚠️ **W6b's TODO calls are the exception, and it is not a detail.** A read is a
            // question whose answer is worthless once the turn is over; `todo_set` is a *side
            // effect the model asked for*. "Remember this, and go north" is a completely
            // natural thing to say in one message — dropping the first half of it silently loses
            // exactly the long-horizon intent §10 exists to keep. Found by watching a mock do it on
            // its very first turn.
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
                        // ⚠️ **Filed, not dropped — the same exception TODO calls get, for the same
                        // reason.** "The menu will not let me do X, so I am doing Y instead" is the
                        // single most natural way to use this tool, and it arrives as one message
                        // with the report and the terminal call in it. Dropping the report because
                        // the turn ended would delete exactly the complaints worth reading.
                        CallKind::Issue(message) => {
                            self.file_issue(id, kind, message, summary.as_deref())
                        }
                        // ⚠️ **The same exception a third time.** "Install this script, and walk
                        // north" is one message, and dropping the first half because the turn ended
                        // would silently lose the thing the model went and read the docs for.
                        CallKind::BattleScript(call) => self.apply_battle_script(call.clone()),
                        CallKind::Rejected(complaint) => complaint.clone(),
                        _ => format!("Not run — the turn ended with `{ended_with}` in the same message."),
                    };
                    self.publish_tool_result(id, call, &classified[index], &content, None);
                    self.history.push(Message::tool_result(&call.id, content));
                }
                // ⚠️ **After the tool results are appended, not before.** The record carries the last
                // few turns of the conversation, and a slice taken above this loop would end with an
                // assistant message whose calls nothing had answered yet.
                if let Terminal::PressButtons { buttons } = &decision {
                    // `why` is enforced by `tools::classify` for the one kind that offers the tool,
                    // so the `unwrap_or_default` is unreachable rather than a tolerated absence.
                    let why = tools::call_reason(&completion.tool_calls[position]).unwrap_or_default();
                    let report = incident::Report::Press { buttons, why: &why };
                    self.record_report(id, kind, report, summary.as_deref());
                }
                return Some((decision, summary));
            }

            // No terminal call, so this is a read step. Anything rejected is answered here; anything
            // real goes to the policy as one batch — except `screenshot`, which this thread answers
            // itself from the frame the host already published.
            //
            // ⚠️ **`+ 2`, not `+ 1`, and that is the whole point of the warning.** It says "call a
            // terminal tool *now*", so it has to be appended before the request that can still act
            // on it — appended on the final iteration it is a sentence the model only ever reads on
            // the next turn, after this one has already been forced to a wait. The budget therefore
            // buys `max_tool_steps - 1` rounds of reading and one round of "decide with what you
            // have", which is what a model that over-reads actually needs.
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
                        // ⚠️ §7.3's one-step rollback: drop the assistant message whose calls were
                        // never serviced. Everything left has its results.
                        self.history.pop();
                        return None;
                    }
                },
            };
            let mut answers = answers.into_iter();

            // ⚠️ Pictures cannot ride on a `tool` message (see `Message::user_with_image`), so they
            // are collected and appended *after* every tool result. Interleaving them would put a
            // `user` message between an assistant's `tool_calls` and their answers, which several
            // endpoints reject outright.
            let mut pictures: Vec<Message> = Vec::new();
            for (call, classification) in completion.tool_calls.iter().zip(&classified) {
                // The encoded picture, when this call answered with one, so the page can be offered
                // the same image the model was — see `publish_tool_result`.
                let mut png: Option<Vec<u8>> = None;
                let content = match classification {
                    CallKind::Read => {
                        let answer = answers.next().unwrap_or_else(|| ToolAnswer::text(
                            "{\"error\": \"the agent returned no result for this call\"}"));
                        match answer.map {
                            None => answer.json,
                            // ⚠️ Same shape as `screenshot` below, and for the same reason: the tool
                            // result is text saying a picture follows, and the picture is a `user`
                            // message appended after every result.
                            Some(map) => match map_image::render(&map) {
                                // ⚠️ **The ASCII grid is the safety net, not dead code.** A map with
                                // no `MapMetadata` cannot be drawn, and answering `read_map` with
                                // names and no terrain at all would be worse than answering it the
                                // old way. `Display for MetaTileMap` is still what every dump and
                                // probe prints, so this costs nothing to keep.
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
                                    // page's ring. `data_url` would compress it a second time.
                                    let encoded = map_image::encode(&canvas);
                                    let url = screenshot::png_data_url(&encoded);
                                    png = Some(encoded);
                                    pictures.push(Message::user_with_image_detail(
                                        caption.clone(),
                                        url,
                                        // ⚠️ `high`: a map is up to 1600 px on a side, and one
                                        // 512x512 tile would turn forty squares of terrain to mush.
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

    /// Say on the page what one tool call answered, and park its picture where the page can fetch it.
    ///
    /// ⚠️ **The content is cut here, not at the client.** Every one of these is broadcast to every
    /// viewer *and* appended to `transcript.jsonl` for the length of the run, and a `read_map`
    /// answer is a few kilobytes of JSON — so a truncation the client applies is a truncation that
    /// has already been paid for twice. What is cut is said in the text, because a JSON object that
    /// simply stops looks like a bug in the encoder.
    ///
    /// ⚠️ **The picture is filed under the seq of the event that announces it**, which is why the
    /// publish has to happen before the `put`: the seq does not exist until the event is sent. A
    /// viewer that asks for one that has fallen off the ring gets a 404 and shows the caption, which
    /// is the whole answer the model got in text anyway.
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

    /// Hand a batch to the policy and block until it comes back. `None` is [`ToolBatchResult::Cancelled`]
    /// or a policy that has gone away.
    fn run_batch(&mut self, id: u64, calls: Vec<ToolCall>) -> Option<Vec<ToolAnswer>> {
        // ⚠️ **Nothing may stop the emulator between here and the answer.** The batch is serviced by
        // `Policy::service_tools`, which only runs when `gb.run` advances the agent — so anything
        // that pauses emulation across this round trip hangs the run on the first `read_map`. That is
        // what killed `GB_PAUSE_WHILE_THINKING`; see `src/llm/config.rs`.
        self.published.set_status(RunStatus::RunningTool { name: names(&calls) });
        let answers = self.tool_calls.send(ToolBatch { turn: id, calls }).ok().and_then(|()| {
            // Blocking, and that is the point: this thread is *supposed* to wait. It does one request
            // at a time and has nothing else to do. The wait is at most one agent tick — 20 ms of
            // emulated time — because the policy answers at the next poll.
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

    /// Fold one response into [`Accounting`]. Called **before** the assistant message is appended,
    /// so `self.history` is still exactly what the endpoint counted — which is what makes the
    /// reported figure usable as a calibration.
    fn account_for(&mut self, completion: &Completion) {
        let usage = completion.usage.unwrap_or_else(|| Usage::estimate(&self.history, completion));
        self.accounting.record(usage, &self.history);
    }

    /// **W6 / §9** — the two-stage compaction, run after every turn.
    ///
    /// Stage 1 is free and often enough: a run that looks at the screen regularly is carrying most of
    /// its context in pictures it has already acted on. Stage 2 costs a completion, so it runs only
    /// when stage 1 left the history still over the line.
    /// Returns what it did, so the caller can write it down. ⚠️ **The `Compacted` event is published
    /// from the same note the log line is built from**, so the page and the file cannot end up
    /// disagreeing about a compaction that neither can be asked to re-run.
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
            // A summary could not be had — the endpoint is down, or the model returned nothing. The
            // trim below is then the whole of the compaction, which is worse than a summary and far
            // better than a history that no longer fits.
        }
        // Still over: the summary was refused, or what it kept is itself too big. Dropping the oldest
        // turns is the last resort, and it leaves the summary alone (`compaction::is_summary`).
        if self.accounting.occupancy(&self.history) >= self.config.compact_above {
            self.trim_history();
        }

        let after = self.accounting.tokens_in(&self.history);
        let summarised = summary.is_some();
        self.published.publish_event(UiEventBody::Compacted { before, after, images_evicted, summarised });
        self.published.set_status(resume);
        Some(CompactionNote {
            before,
            after,
            images_evicted,
            // What the history *lost*, which is not `was - len()`: `apply_summary` adds the summary
            // back, so the two added messages would understate the drop by exactly that much.
            dropped: was.saturating_sub(self.history.len()),
            summary,
        })
    }

    /// One extra completion, asking the model to write the story so far. `None` if it could not be
    /// had, in which case the caller falls back to [`Self::trim_history`].
    ///
    /// ⚠️ **Not cancellable, deliberately.** Every other blocking point in this file gives way to a
    /// change of decision kind; this one cannot, because abandoning it leaves the history over the
    /// limit and the *next* request is then the one that 400s. The cost of that choice is one
    /// completion's worth of latency on a game that is not waiting for anything — the emulator keeps
    /// running throughout, as it does while any turn is in flight.
    fn summarise(&mut self) -> Option<String> {
        let request = compaction::summary_request(&self.config, &self.history);
        let published = Arc::clone(&self.published);
        let result = stream_with_retries(
            self.retry,
            self.endpoint.as_ref(),
            &request,
            // Not published as an `AssistantDelta`: the summary is bookkeeping, and a thousand words
            // of it in the conversation pane would read as the model talking to itself.
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

    /// The last resort described at [`TRIM_TO`]: drop whole turns from the front until the history
    /// is back under half the window.
    ///
    /// It cuts only at turn boundaries — see [`compaction::is_turn_start`] — so a `tool_call` is
    /// never separated from its result, and it never drops the last turn, which would leave the
    /// model with nothing to answer.
    fn trim_history(&mut self) {
        let target = (self.accounting.limit() as f64 * TRIM_TO) as u64;
        // Index 0 is the system prompt; index 1 is the summary, if a stage 2 has ever run. Neither is
        // a turn, and dropping the summary would throw away every turn it stands for.
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
}

/// What the status shows while a batch is out. One name reads better than one; four read worse than
/// a count.
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
        Terminal::ChooseBattleAction { id } => format!("choose_battle_action {id}"),
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

/// Drop a trailing `user` message. Used when a request failed outright, so the question is not left
/// in the history unanswered — the next turn asks a fresher version of it anyway.
///
/// ⚠️ It tests for a **turn start**, not merely for a `user` message: W5's picture and W6's evicted
/// picture are both `user` messages in the middle of a turn, and popping either would leave the tool
/// result they belong beside without the context that explains it.
trait PopIfUser {
    fn pop_if_user(&mut self);
}

impl PopIfUser for Vec<Message> {
    fn pop_if_user(&mut self) {
        if self.last().is_some_and(compaction::is_turn_start) {
            self.pop();
        }
    }
}
