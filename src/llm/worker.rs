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
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::llm::accounting::Accounting;
use crate::llm::client::{ChatEndpoint, RetryPolicy, stream_with_retries};
use crate::llm::compaction;
use crate::llm::config::LlmConfig;
use crate::llm::todo::TodoList;
use crate::llm::prompt;
use crate::llm::screenshot;
use crate::llm::protocol::{ChatRequest, Completion, Fragment, Message, StreamOptions, ToolCall, Usage};
use crate::llm::tools::{self, CallKind, DecisionKind, Terminal};
use crate::llm::LlmError;
use crate::web::published::{Published, RunStatus, TodoView, UiEventBody};

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

#[derive(Debug, Clone)]
pub enum ToolBatchResult {
    /// One entry per call, in the order they were sent.
    Answered(Vec<String>),
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

/// Where [`Worker::trim_history`] drops back to. That function is W4's stopgap, kept as W6's **last
/// resort**: it throws whole turns away from the front of the history rather than summarising them,
/// and it runs only when a summarisation could not be had — the endpoint was down, or the model
/// returned nothing. Losing the oldest turns is much worse than a summary and much better than a
/// run that 400s on every request from here on.
const TRIM_TO: f64 = 0.50;

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

    /// The conversation. Index 0 is the system prompt; it is never removed and, since **W6b**'s
    /// plan moved out of it, never rewritten either — see [`prompt::system_message`].
    messages: Vec<Message>,
    /// **W6b / §10** — the model's plan. Answered here rather than at the policy poll: none of it
    /// needs the emulator.
    todo: TodoList,
    /// What the page was last told the plan is, so [`Self::publish_todo`] can be called from every
    /// moment it might have changed without publishing the same list twice.
    published_plan: Option<Vec<TodoView>>,
    /// **W6** — tokens reported, tokens spent, and how far our own estimate is from the endpoint's.
    accounting: Accounting,
    /// **`POST /api/new-run`** — taken at the top of every turn. See [`Restart`].
    restart: Restarts,
}

/// Build the worker and its counterpart handles. The thread is started by [`Worker::spawn`]; this is
/// separate so a test can drive [`Worker::run_one`] on its own thread and control the timing.
pub fn channels(
    endpoint: Box<dyn ChatEndpoint>,
    config: LlmConfig,
    published: Arc<Published>,
    todo: TodoList,
) -> (Worker, TurnHandles) {
    let (turn_tx, turn_rx) = mpsc::channel();
    let (outcome_tx, outcome_rx) = mpsc::channel();
    let (call_tx, call_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let generation = Arc::new(AtomicU64::new(0));
    let restart: Restarts = Arc::new(Mutex::new(None));

    let accounting = Accounting::new(config.context_limit);
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
        messages: vec![prompt::system_message()],
        todo,
        published_plan: None,
        accounting,
        restart: Arc::clone(&restart),
    };
    let handles = TurnHandles {
        turns: turn_tx,
        outcomes: outcome_rx,
        tool_calls: call_rx,
        tool_results: result_tx,
        generation,
        restart,
    };
    (worker, handles)
}

impl Worker {
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
        // Index 0 is the system prompt and is never removed, so the history *is* this vec.
        self.messages = vec![prompt::system_message()];
        self.accounting = Accounting::new(self.config.context_limit);
        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: "the game restarted — the conversation starts again from the system prompt"
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
        let TurnRequest { id, kind, situation, headline } = request;
        self.published.publish_event(UiEventBody::TurnStarted { turn: id, kind: kind.label(), headline });

        self.sync_plan();
        self.publish_todo();
        self.messages.push(Message::user(situation));
        let outcome = self.decide(id, kind);
        match outcome {
            Some(decision) => {
                self.published.publish_event(UiEventBody::Decision {
                    turn: id,
                    summary: describe(&decision),
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
        self.compact_if_needed();
    }

    /// **W6b / §10** — put the model's plan in front of it, in exactly one place, at the cheapest
    /// moment.
    ///
    /// Called immediately before a turn's situation is appended, so the plan is the second-newest
    /// message when the request goes out: recent enough to be read as current, and never the last
    /// thing, which the contract has to be.
    ///
    /// ⚠️ **The whole design is "only when it changed", and that is what makes it cheap.** Three
    /// cases, and the common one costs nothing:
    ///
    /// - the copy in the history already says this → **do nothing**, so the history grows purely by
    ///   append and the endpoint's prefix cache is intact all the way back to the system prompt;
    /// - the copy says something else → remove it and append a fresh one, which invalidates the
    ///   cache from wherever that copy sat. It sat at the last turn the plan changed, so a model
    ///   that edits often pays little and a model that edits rarely pays rarely;
    /// - there is no copy — a compaction took it, or this is the first turn → append one.
    ///
    /// The alternative it replaced was re-rendering the plan into message 0, which invalidated the
    /// entire conversation every time the model touched its own list. See [`prompt::system_message`].
    fn sync_plan(&mut self) {
        let plan = prompt::plan_message(&self.todo);
        match self.messages.iter().position(|message| prompt::is_plan(message)) {
            Some(at) if self.messages[at] == plan => return,
            Some(at) => {
                self.messages.remove(at);
            }
            None => {}
        }
        self.messages.push(plan);
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

    /// `None` means the turn was cancelled and abandoned.
    fn decide(&mut self, id: u64, kind: DecisionKind) -> Option<Terminal> {
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
                    messages: self.messages.clone(),
                    tools: specs.clone(),
                    parallel_tool_calls: Some(true),
                    temperature: self.config.temperature,
                    stream: true,
                    stream_options: StreamOptions { include_usage: true },
                };
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
                                if retry.already_spoke { " — the reply will start again" } else { "" },
                            ),
                        });
                    },
                );
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
                        self.messages.pop_if_user();
                        return Some(Terminal::Wait { ticks: FAILURE_WAIT_TICKS });
                    }
                }
            };

            self.account_for(&completion);
            self.messages.push(Message::assistant(completion.content.clone(), completion.tool_calls.clone()));

            if completion.tool_calls.is_empty() {
                // §7.5's fallback. One nudge quoting the rule, then the rule is enforced for it.
                if nudged {
                    return Some(self.give_up(id, "the model replied twice with no tool call"));
                }
                nudged = true;
                self.messages.push(Message::user(prompt::nudge(kind)));
                continue;
            }

            for call in &completion.tool_calls {
                self.published.publish_event(UiEventBody::ToolCall {
                    turn: id,
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
            }

            let classified: Vec<CallKind> =
                completion.tool_calls.iter().map(|call| tools::classify(kind, call)).collect();

            // A message that mixes reads with a terminal call ends the turn: the model has already
            // committed, so running the reads would be answering a question it stopped asking. They
            // still get a result message, because every `tool_call` needs one.
            //
            // ⚠️ **W6b's TODO calls are the exception, and it is not a detail.** A read is a
            // question whose answer is worthless once the turn is over; `todo_add` is a *side
            // effect the model asked for*. "Remember this, and go north" is a completely
            // natural thing to say in one message — dropping the first half of it silently loses
            // exactly the long-horizon intent §10 exists to keep. Found by watching a mock do it on
            // its very first turn.
            if let Some(position) = classified.iter().position(|c| matches!(c, CallKind::Terminal(_))) {
                let CallKind::Terminal(decision) = &classified[position] else { unreachable!() };
                let decision = decision.clone();
                let ended_with = completion.tool_calls[position].function.name.clone();
                for (index, call) in completion.tool_calls.iter().enumerate() {
                    let content = match &classified[index] {
                        _ if index == position => {
                            "Accepted. The agent is carrying it out now; the next turn will tell you \
                             what happened."
                                .to_string()
                        }
                        CallKind::Todo(call) => self.apply_todo(call.clone()),
                        CallKind::Rejected(complaint) => complaint.clone(),
                        _ => format!("Not run — the turn ended with `{ended_with}` in the same message."),
                    };
                    self.messages.push(Message::tool_result(&call.id, content));
                }
                return Some(decision);
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
                        self.messages.pop();
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
                let content = match classification {
                    CallKind::Read => answers.next().unwrap_or_else(|| {
                        "{\"error\": \"the agent returned no result for this call\"}".to_string()
                    }),
                    CallKind::Screenshot => {
                        self.published.set_status(RunStatus::RunningTool { name: "screenshot".into() });
                        let frame = self.published.latest_frame();
                        let caption = screenshot::caption(frame.seq);
                        pictures.push(Message::user_with_image(
                            caption.clone(),
                            screenshot::data_url(&frame.pixels),
                        ));
                        format!("{caption} It is attached to the message after this one.")
                    }
                    CallKind::Todo(call) => self.apply_todo(call.clone()),
                    CallKind::Rejected(complaint) => complaint.clone(),
                    CallKind::Terminal(_) => unreachable!("handled above"),
                };
                self.messages.push(Message::tool_result(&call.id, content));
            }
            self.messages.extend(pictures);
            if out_of_reads {
                self.messages.push(Message::user(prompt::OUT_OF_STEPS));
            }
        }

        Some(self.give_up(id, "the model used its whole tool budget without deciding"))
    }

    /// Hand a batch to the policy and block until it comes back. `None` is [`ToolBatchResult::Cancelled`]
    /// or a policy that has gone away.
    fn run_batch(&mut self, id: u64, calls: Vec<ToolCall>) -> Option<Vec<String>> {
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
    /// so `self.messages` is still exactly what the endpoint counted — which is what makes the
    /// reported figure usable as a calibration.
    fn account_for(&mut self, completion: &Completion) {
        let usage = completion.usage.unwrap_or_else(|| Usage::estimate(&self.messages, completion));
        self.accounting.record(usage, &self.messages);
    }

    /// **W6 / §9** — the two-stage compaction, run after every turn.
    ///
    /// Stage 1 is free and often enough: a run that looks at the screen regularly is carrying most of
    /// its context in pictures it has already acted on. Stage 2 costs a completion, so it runs only
    /// when stage 1 left the history still over the line.
    fn compact_if_needed(&mut self) {
        if self.accounting.occupancy(&self.messages) < self.config.compact_above {
            return;
        }
        let resume = self.published.run_status();
        self.published.set_status(RunStatus::Compacting);
        let before = self.accounting.tokens_in(&self.messages);

        let images_evicted = compaction::evict_images(&mut self.messages, compaction::KEEP_IMAGES);
        let mut summarised = false;
        let still_over = self.accounting.occupancy(&self.messages) >= self.config.compact_above;
        if still_over && compaction::worth_summarising(&self.messages, compaction::KEEP_MESSAGES) {
            if let Some(summary) = self.summarise() {
                compaction::apply_summary(&mut self.messages, &summary, compaction::KEEP_MESSAGES);
                summarised = true;
            }
            // A summary could not be had — the endpoint is down, or the model returned nothing. The
            // trim below is then the whole of the compaction, which is worse than a summary and far
            // better than a history that no longer fits.
        }
        // Still over: the summary was refused, or what it kept is itself too big. Dropping the oldest
        // turns is the last resort, and it leaves the summary alone (`compaction::is_summary`).
        if self.accounting.occupancy(&self.messages) >= self.config.compact_above {
            self.trim_history();
        }

        let after = self.accounting.tokens_in(&self.messages);
        self.published.publish_event(UiEventBody::Compacted { before, after, images_evicted, summarised });
        self.published.set_status(resume);
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
        let request = compaction::summary_request(&self.config, &self.messages);
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
        let first = 1 + usize::from(self.messages.get(1).is_some_and(compaction::is_summary));
        let mut dropped = 0;
        while self.accounting.tokens_in(&self.messages) > target {
            let Some(boundary) =
                self.messages.iter().skip(first).position(compaction::is_turn_start).map(|i| i + first)
            else {
                break;
            };
            let Some(next) = self
                .messages
                .iter()
                .skip(boundary + 1)
                .position(compaction::is_turn_start)
                .map(|i| i + boundary + 1)
            else {
                break; // only one turn left; dropping it would leave nothing to answer
            };
            self.messages.drain(boundary..next);
            dropped += 1;
        }
        if dropped > 0 {
            self.published.publish_event(UiEventBody::Notice {
                level: "info",
                message: format!("context is full — dropped the {dropped} oldest turns"),
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
        Terminal::ChooseAction { id } => format!("choose_action {id}"),
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
        Terminal::BuyItem { item } => match item {
            Some(item) => format!("buy_item {item}"),
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
