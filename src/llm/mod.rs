//! **W4** — the LLM half of `docs/llm-web-playthrough-plan.md`: an OpenAI-compatible client, the
//! tool surface it is offered, and the worker thread that turns one decision point into one
//! completion.
//!
//! ```text
//!   emulator thread                         worker thread (this module)
//!   ───────────────                         ───────────────────────────
//!   LlmPolicy::pick_*  ──TurnRequest──────►  build messages, stream a completion
//!            ▲                                        │
//!            │                                 read tools?  ──ToolBatch──┐
//!            │                                        │                  │
//!   service_tools  ◄─────────────────────────────────────────────────────┘
//!            │      answers from ONE observed GameState, sends ToolBatchResult
//!            ▼
//!        Decision   ◄──TurnOutcome────────  a terminal tool call ends the turn
//! ```
//!
//! Two rules hold the whole thing up, and both are §7 of the plan:
//!
//! 1. **Every turn ends with exactly one terminal tool call.** Enforced by scoping the `tools` array
//!    to the decision kind being asked ([`tools::for_kind`]), by restating the contract in the system
//!    prompt *and* in every turn request ([`prompt`]), and by a nudge-then-force fallback in the
//!    worker.
//! 2. **A turn is keyed by the decision kind it answers**, and a poll for a different kind cancels
//!    it. That is what makes it safe for the emulator to keep running while the model thinks: a
//!    battle decision can never be applied to an overworld state.
//!
//! Nothing here is async. The worker is a plain `std::thread` blocking on a channel, and `ureq`
//! streams the response body through `impl Read`.

pub mod client;
pub mod config;
pub mod prompt;
pub mod protocol;
pub mod tools;
pub mod worker;

pub use config::LlmConfig;

/// Everything that can go wrong between here and the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// A non-2xx response. Carries the status because the retry policy is keyed on it.
    Http { status: u16, message: String },
    /// The connection, DNS, TLS, or a body that stopped arriving mid-stream.
    Transport(String),
    /// A 200 whose content was not what the protocol says. Retrying will not help.
    Protocol(String),
    /// The decision this turn was answering is no longer the question being asked (§7.3).
    Cancelled,
}

impl LlmError {
    /// Whether another attempt is worth making. Rate limits and server faults are transient by
    /// definition; a 400 means the request was wrong and will be wrong again.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http { status, .. } => *status == 408 || *status == 429 || *status >= 500,
            Self::Transport(_) => true,
            Self::Protocol(_) | Self::Cancelled => false,
        }
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http { status, message } => write!(f, "the endpoint returned {status}: {message}"),
            Self::Transport(detail) => write!(f, "could not reach the endpoint: {detail}"),
            Self::Protocol(detail) => write!(f, "the endpoint's response was malformed: {detail}"),
            Self::Cancelled => write!(f, "the turn was cancelled"),
        }
    }
}

impl std::error::Error for LlmError {}
