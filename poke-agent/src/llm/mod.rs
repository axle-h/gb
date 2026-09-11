//! The LLM turn loop: `LlmPolicy::pick_*` sends a `TurnRequest` to the worker thread, whose read
//! tools come back as a `ToolBatch` answered by `service_tools` from one observed `GameState`,
//! until a terminal tool call ends the turn as a `TurnOutcome`.

pub mod accounting;
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
pub mod todo;
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
}

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

    #[test]
    fn a_timeout_says_the_endpoint_took_the_request() {
        let said = format!("{}", LlmError::Timeout("waited 180s".into()));
        assert!(said.contains("took the request"), "{said}");
        assert!(!said.contains("could not reach"), "{said}");
    }
}
