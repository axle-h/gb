//! The LLM turn loop: `LlmPolicy::pick_*` sends a `TurnRequest` to the worker thread, whose read
//! tools come back as a `ToolBatch` answered by `service_tools` from one observed `GameState`,
//! until a terminal tool call ends the turn as a `TurnOutcome`.

pub mod accounting;
pub mod anthropic;
pub mod battle_report;
pub mod battle_script;
pub mod client;
pub mod compaction;
pub mod config;
pub mod incident;
pub mod guide;
pub mod history;
pub mod prompt;
pub mod map_image;
pub mod protocol;
pub mod screenshot;
pub mod plan;
pub mod files;
pub mod tools;
pub mod worker;

pub use config::LlmConfig;

/// Everything that can go wrong between here and the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// A non-2xx response.
    Http { status: u16, message: String },
    /// The connection, DNS, TLS, or a body that stopped arriving mid-stream.
    Transport(String),
    /// The endpoint accepted the request and did not answer inside `GB_REQUEST_TIMEOUT_SECS`.
    Timeout(String),
    /// A 429, with the moment the quota reopens when the endpoint said so (Unix milliseconds).
    RateLimited { resets_at_ms: Option<u64>, message: String },
    /// A 200 whose content was not what the protocol says.
    Protocol(String),
    Cancelled,
}

impl LlmError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http { status, .. } => *status == 408 || *status == 429 || *status >= 500,
            Self::Transport(_) => true,
            // `stream_with_retries` spends no attempts on a reset beyond the backoff's reach.
            Self::RateLimited { .. } => true,
            // The endpoint still has this request.
            Self::Timeout(_) => false,
            Self::Protocol(_) | Self::Cancelled => false,
        }
    }

    /// 401, 402 or 403: the account was refused, not the request. The only failures
    /// [`RefusalPark`](crate::llm::worker::RefusalPark) counts as refusals.
    pub fn is_account_refusal(&self) -> bool {
        matches!(self, Self::Http { status: 401..=403, .. })
    }

    /// Any other 4xx but 408 and 429: this request is wrong, a prompt too long among them, and
    /// asking it again unchanged will not help.
    pub fn is_rejected_request(&self) -> bool {
        matches!(self, Self::Http { status: 400..=499, .. })
            && !self.is_account_refusal()
            && !self.is_retryable()
    }

    /// A rejected request that says the prompt outgrew the window: a 413, or a body naming the
    /// context or the prompt's length. The one rejection a compaction can cure.
    pub fn is_context_overflow(&self) -> bool {
        match self {
            Self::Http { status: 413, .. } => true,
            Self::Http { message, .. } if self.is_rejected_request() => {
                let message = message.to_ascii_lowercase();
                OVERFLOW_PHRASES.iter().any(|phrase| message.contains(phrase))
            }
            _ => false,
        }
    }
}

/// How the providers word a prompt that is over the window: Anthropic, OpenAI, and the local
/// servers that copy one or the other.
const OVERFLOW_PHRASES: &[&str] = &[
    "prompt is too long",
    "context_length_exceeded",
    "maximum context",
    "context length",
    "context window",
    "context size",
    "too many tokens",
];

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http { status, message } => write!(f, "the endpoint returned {status}: {message}"),
            Self::Transport(detail) => write!(f, "could not reach the endpoint: {detail}"),
            Self::Timeout(detail) => {
                write!(f, "the endpoint took the request and did not answer: {detail}")
            }
            Self::RateLimited { resets_at_ms: None, message } => {
                write!(f, "the endpoint is rate limiting us: {message}")
            }
            Self::RateLimited { resets_at_ms: Some(at), message } => {
                write!(f, "the endpoint is rate limiting us until {at} (unix ms): {message}")
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

    #[test]
    fn a_timeout_is_not_retried_but_a_broken_connection_is() {
        assert!(!LlmError::Timeout("no answer".into()).is_retryable());
        assert!(LlmError::Transport("connection refused".into()).is_retryable());

        assert!(LlmError::Http { status: 429, message: String::new() }.is_retryable());
        assert!(LlmError::Http { status: 503, message: String::new() }.is_retryable());
        assert!(!LlmError::Http { status: 400, message: String::new() }.is_retryable());
        assert!(!LlmError::Cancelled.is_retryable());
    }

    /// Every 4xx is exactly one of retried, an account refusal and a rejected request.
    #[test]
    fn a_4xx_is_retried_refused_or_rejected_and_never_two_of_them() {
        for status in 400..500 {
            let failure = LlmError::Http { status, message: String::new() };
            let classes = [failure.is_retryable(), failure.is_account_refusal(), failure.is_rejected_request()];
            assert_eq!(classes.iter().filter(|class| **class).count(), 1, "{status}: {classes:?}");
        }
        assert!(LlmError::Http { status: 402, message: String::new() }.is_account_refusal());
        assert!(LlmError::Http { status: 413, message: String::new() }.is_rejected_request());
        assert!(!LlmError::Http { status: 500, message: String::new() }.is_rejected_request());
        assert!(!LlmError::Protocol(String::new()).is_rejected_request());
    }

    #[test]
    fn only_a_rejection_shaped_like_an_overflow_is_one() {
        let http = |status, message: &str| LlmError::Http { status, message: message.into() };
        for overflow in [
            http(413, "request too large"),
            http(400, "prompt is too long: 200000 tokens > 128000 maximum [invalid_request_error]"),
            http(400, "This model's maximum context length is 128000 tokens. (context_length_exceeded)"),
            http(400, "the request exceeds the available context size, try increasing it"),
        ] {
            assert!(overflow.is_context_overflow(), "{overflow}");
        }
        for other in [
            http(400, "temperature: unsupported for this model"),
            http(404, "model: claude-x [not_found_error]"),
            http(422, "tools.0.name: string too long"),
            http(500, "maximum context length"),
            http(402, "prompt is too long"),
            LlmError::Protocol("prompt is too long".into()),
        ] {
            assert!(!other.is_context_overflow(), "{other}");
        }
    }

    #[test]
    fn a_timeout_says_the_endpoint_took_the_request() {
        let said = format!("{}", LlmError::Timeout("waited 180s".into()));
        assert!(said.contains("took the request"), "{said}");
        assert!(!said.contains("could not reach"), "{said}");
    }
}
