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
//!   └─ send TurnOutcome
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

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::llm::client::{ChatEndpoint, RetryPolicy, stream_with_retries};
use crate::llm::config::LlmConfig;
use crate::llm::prompt;
use crate::llm::protocol::{ChatRequest, Message, StreamOptions, ToolCall, Usage};
use crate::llm::tools::{self, CallKind, DecisionKind, Terminal};
use crate::llm::LlmError;
use crate::web::published::{Published, UiEventBody, UsageView};

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
}

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

/// Occupancy at which the W4 stopgap starts dropping old turns, and the level it drops back to.
///
/// ⚠️ **This is not W6's compaction and does not pretend to be.** W6 summarises; this throws whole
/// turns away from the front of the history, oldest first. It is here because without *some* bound a
/// run stops working after an hour with a 400 from the endpoint, which would make W4's own
/// acceptance criterion unreachable. It cuts only at turn boundaries — a `user` message — so a
/// `tool_call` is never separated from its result.
const TRIM_ABOVE: f64 = 0.70;
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

    /// The conversation. Index 0 is the system prompt and is never removed.
    messages: Vec<Message>,
    /// The most recent reported context occupancy, in tokens.
    context_tokens: u64,
}

/// Build the worker and its counterpart handles. The thread is started by [`Worker::spawn`]; this is
/// separate so a test can drive [`Worker::run_one`] on its own thread and control the timing.
pub fn channels(
    endpoint: Box<dyn ChatEndpoint>,
    config: LlmConfig,
    published: Arc<Published>,
) -> (Worker, TurnHandles) {
    let (turn_tx, turn_rx) = mpsc::channel();
    let (outcome_tx, outcome_rx) = mpsc::channel();
    let (call_tx, call_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let generation = Arc::new(AtomicU64::new(0));

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
        messages: vec![Message::system(prompt::SYSTEM_PROMPT)],
        context_tokens: 0,
    };
    let handles = TurnHandles {
        turns: turn_tx,
        outcomes: outcome_rx,
        tool_calls: call_rx,
        tool_results: result_tx,
        generation,
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

    /// One turn, start to finish. Public so a test can step the worker without a thread.
    pub fn run_one(&mut self, request: TurnRequest) {
        let TurnRequest { id, kind, situation, headline } = request;
        self.published.publish_event(UiEventBody::TurnStarted { turn: id, kind: kind.label(), headline });

        self.messages.push(Message::user(situation));
        let outcome = self.decide(id, kind);
        match outcome {
            Some(decision) => {
                self.published.publish_event(UiEventBody::Decision {
                    turn: id,
                    summary: describe(&decision),
                    usage: (self.context_tokens > 0).then(|| UsageView {
                        context_tokens: self.context_tokens,
                        context_limit: self.config.context_limit,
                    }),
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
        self.trim_history();
    }

    /// `None` means the turn was cancelled and abandoned.
    fn decide(&mut self, id: u64, kind: DecisionKind) -> Option<Terminal> {
        let specs = tools::for_kind(kind);
        let mut nudged = false;

        for step in 0..self.config.max_tool_steps {
            if self.is_stale(id) {
                return None;
            }

            let completion = {
                let request = ChatRequest {
                    model: self.config.model.clone(),
                    messages: self.messages.clone(),
                    tools: specs.clone(),
                    parallel_tool_calls: true,
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
                        published.publish_event(UiEventBody::AssistantDelta {
                            turn: id,
                            text: delta.to_string(),
                        });
                    },
                    &|| generation.load(Ordering::SeqCst) != id,
                    &mut |retry| {
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

            self.account_for(completion.usage, &completion);
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
            if let Some(position) = classified.iter().position(|c| matches!(c, CallKind::Terminal(_))) {
                let CallKind::Terminal(decision) = &classified[position] else { unreachable!() };
                let decision = decision.clone();
                let ended_with = completion.tool_calls[position].function.name.clone();
                for (index, call) in completion.tool_calls.iter().enumerate() {
                    let content = if index == position {
                        "Accepted. The agent is carrying it out now; the next turn will tell you what happened."
                            .to_string()
                    } else {
                        format!("Not run — the turn ended with `{ended_with}` in the same message.")
                    };
                    self.messages.push(Message::tool_result(&call.id, content));
                }
                return Some(decision);
            }

            // No terminal call, so this is a read step. Anything rejected is answered here; anything
            // real goes to the policy as one batch.
            let last_step = step + 1 == self.config.max_tool_steps;
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

            for (call, classification) in completion.tool_calls.iter().zip(&classified) {
                let content = match classification {
                    CallKind::Read => answers.next().unwrap_or_else(|| {
                        "{\"error\": \"the agent returned no result for this call\"}".to_string()
                    }),
                    CallKind::Rejected(complaint) => complaint.clone(),
                    CallKind::Terminal(_) => unreachable!("handled above"),
                };
                self.messages.push(Message::tool_result(&call.id, content));
            }
            if last_step {
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

    fn account_for(&mut self, usage: Option<Usage>, completion: &crate::llm::protocol::Completion) {
        let usage = usage.unwrap_or_else(|| Usage::estimate(&self.messages, completion));
        self.context_tokens = usage.prompt_tokens + usage.completion_tokens;
    }

    /// The W4 stopgap described at [`TRIM_ABOVE`]. Drops whole turns from the front.
    fn trim_history(&mut self) {
        let limit = self.config.context_limit as f64;
        if (self.context_tokens as f64) < limit * TRIM_ABOVE {
            return;
        }
        let target = (limit * TRIM_TO) as u64;
        let mut held: u64 = self.messages.iter().map(Message::approximate_tokens).sum();
        // Estimated tokens and reported ones are different scales, so trim against the estimate of
        // the whole history rather than mixing the two.
        let mut dropped = 0;
        while held > target {
            // The next turn boundary after the system prompt. Cutting only here is what keeps every
            // `tool_call` with its result.
            let Some(boundary) = self.messages.iter().skip(1).position(is_turn_start).map(|i| i + 1) else {
                break;
            };
            let Some(next) = self.messages.iter().skip(boundary + 1).position(is_turn_start).map(|i| i + boundary + 1)
            else {
                break; // only one turn left; dropping it would leave nothing to answer
            };
            held -= self.messages[boundary..next].iter().map(Message::approximate_tokens).sum::<u64>();
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

fn is_turn_start(message: &Message) -> bool {
    message.role == crate::llm::protocol::Role::User
}

fn describe(decision: &Terminal) -> String {
    match decision {
        Terminal::ChooseAction { id } => format!("choose_action {id}"),
        Terminal::ChooseBattleAction { id } => format!("choose_battle_action {id}"),
        Terminal::Wait { ticks } => format!("wait {ticks} ticks"),
    }
}

/// Drop a trailing `user` message. Used when a request failed outright, so the question is not left
/// in the history unanswered — the next turn asks a fresher version of it anyway.
trait PopIfUser {
    fn pop_if_user(&mut self);
}

impl PopIfUser for Vec<Message> {
    fn pop_if_user(&mut self) {
        if self.last().is_some_and(is_turn_start) {
            self.pop();
        }
    }
}
