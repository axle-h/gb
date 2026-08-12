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

pub mod accounting;
pub mod client;
pub mod compaction;
pub mod config;
pub mod prompt;
pub mod map_image;
pub mod protocol;
pub mod screenshot;
pub mod todo;
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
    /// The endpoint accepted the request and did not answer inside `GB_REQUEST_TIMEOUT_SECS`.
    ///
    /// ⚠️ **Deliberately not a [`Self::Transport`], and deliberately not retryable.** The two look
    /// alike and are opposites: a connection that never opened consumed no work at the far end, so
    /// another attempt is free; a request that was *accepted* is being worked on, and llama.cpp says
    /// so when we hang up — "Stopping generation... (If the model is busy processing the prompt, it
    /// will finish first.)". On an endpoint that serves one request at a time, a retry therefore
    /// queues **behind the very request it is replacing** and cannot be faster than waiting would
    /// have been. It can only add a second piece of work nobody is waiting for.
    Timeout(String),
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
            // See the variant's own note: the far end still has this request.
            Self::Timeout(_) => false,
            Self::Protocol(_) | Self::Cancelled => false,
        }
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http { status, message } => write!(f, "the endpoint returned {status}: {message}"),
            Self::Transport(detail) => write!(f, "could not reach the endpoint: {detail}"),
            Self::Timeout(detail) => {
                write!(f, "the endpoint took the request and did not answer: {detail}")
            }
            Self::Protocol(detail) => write!(f, "the endpoint's response was malformed: {detail}"),
            Self::Cancelled => write!(f, "the turn was cancelled"),
        }
    }
}

impl std::error::Error for LlmError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ The distinction this whole variant exists for. A connection that never opened and a
    /// request the endpoint is still working on are both "no answer yet", and treating them the same
    /// is what turned one slow turn into five abandoned generations queued behind each other on a
    /// server that runs one at a time.
    #[test]
    fn a_timeout_is_not_retried_but_a_broken_connection_is() {
        assert!(!LlmError::Timeout("no answer".into()).is_retryable());
        assert!(LlmError::Transport("connection refused".into()).is_retryable());

        // The rest of the table is unchanged: rate limits and server faults are transient, a 400 is
        // the request being wrong and will be wrong again.
        assert!(LlmError::Http { status: 429, message: String::new() }.is_retryable());
        assert!(LlmError::Http { status: 503, message: String::new() }.is_retryable());
        assert!(!LlmError::Http { status: 400, message: String::new() }.is_retryable());
        assert!(!LlmError::Cancelled.is_retryable());
    }

    /// The message reaches the operator through a `Notice` and the transcript, so it has to say which
    /// of the two happened rather than "could not reach the endpoint" for both.
    #[test]
    fn a_timeout_says_the_endpoint_took_the_request() {
        let said = format!("{}", LlmError::Timeout("waited 180s".into()));
        assert!(said.contains("took the request"), "{said}");
        assert!(!said.contains("could not reach"), "{said}");
    }
}
