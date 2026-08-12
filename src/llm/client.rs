//! **W4 / §7.1** — the HTTP half: one blocking `POST /chat/completions` with `stream: true`.
//!
//! `ureq` rather than `reqwest`, and blocking rather than async, for one reason: `Body::into_reader()`
//! hands back a plain `impl Read`, so consuming the SSE stream is a `BufReader::lines()` loop on the
//! worker's own thread. `reqwest::blocking` would start a second tokio runtime inside a process that
//! already has one for `axum` and does not want the emulator anywhere near either.
//!
//! The retry policy lives here too but is **not** inside the client: [`stream_with_retries`] is a
//! free function over the [`ChatEndpoint`] trait, so the mock server in the tests exercises the same
//! backoff the real endpoint does, and so the sleep between attempts can be interrupted by the same
//! cancellation signal that interrupts a stream.

use std::io::BufReader;
use std::time::Duration;

use crate::llm::LlmError;
use crate::llm::config::LlmConfig;
use crate::llm::protocol::{ChatRequest, Completion, Fragment, describe_error_body, read_stream};

/// One streamed chat completion. A trait so the worker can be driven by a scripted endpoint in tests
/// without a socket, and so W6's accounting has one place to wrap.
pub trait ChatEndpoint: Send {
    /// Stream one completion.
    ///
    /// `on_delta` receives each [`Fragment`] — prose and reasoning on separate channels — as it
    /// arrives. `cancelled` is consulted on every line of
    /// the response — see §7.3: it is one of the two points a turn can be abandoned, and returning
    /// [`LlmError::Cancelled`] drops the reader, which aborts the request.
    fn stream_completion(
        &self,
        request: &ChatRequest,
        on_delta: &mut dyn FnMut(Fragment<'_>),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Completion, LlmError>;
}

/// How long to wait for the endpoint to *start* answering. A completion itself has no deadline —
/// a long one is a model thinking, not a fault — but a connection that never produces a first byte
/// would otherwise hang the worker for the rest of the run.
const TIMEOUT_CONNECT: Duration = Duration::from_secs(30);
const TIMEOUT_FIRST_BYTE: Duration = Duration::from_secs(180);
/// The gap between two chunks of a response already in progress. Generous, because a model pausing
/// mid-sentence to think is normal; finite, because a half-open TCP connection looks exactly like it.
const TIMEOUT_BETWEEN_CHUNKS: Duration = Duration::from_secs(180);

pub struct OpenAiClient {
    agent: ureq::Agent,
    url: String,
    authorization: String,
}

impl OpenAiClient {
    pub fn new(config: &LlmConfig) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // ⚠️ Off, deliberately. ureq's default turns a 4xx into `Error::StatusCode` *and throws
            // the body away* — and the body is where an OpenAI-compatible endpoint says which
            // parameter it did not like, which is the single most useful thing in the whole error.
            .http_status_as_error(false)
            .timeout_connect(Some(TIMEOUT_CONNECT))
            .timeout_recv_response(Some(TIMEOUT_FIRST_BYTE))
            .timeout_recv_body(Some(TIMEOUT_BETWEEN_CHUNKS))
            .user_agent("gb-pokemon-agent/0.1")
            .build()
            .into();
        Self {
            agent,
            url: config.completions_url(),
            authorization: format!("Bearer {}", config.api_key),
        }
    }
}

impl ChatEndpoint for OpenAiClient {
    fn stream_completion(
        &self,
        request: &ChatRequest,
        on_delta: &mut dyn FnMut(Fragment<'_>),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Completion, LlmError> {
        let body = serde_json::to_string(request)
            .map_err(|e| LlmError::Protocol(format!("could not encode the request: {e}")))?;

        let response = self
            .agent
            .post(&self.url)
            .header("Authorization", &self.authorization)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            // Identity, though `ureq` would decompress transparently: a compressed SSE body only
            // reaches us in whole decoder blocks, which turns a token-by-token stream into a
            // sputtering one for no bandwidth worth having on a link carrying a video feed anyway.
            .header("Accept-Encoding", "identity")
            .send(body)
            .map_err(|e| LlmError::Transport(format!("POST {} failed: {e}", self.url)))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let message = response
                .into_body()
                .read_to_string()
                .map(|body| describe_error_body(&body))
                .unwrap_or_else(|e| format!("(the error body could not be read: {e})"));
            return Err(LlmError::Http { status, message });
        }

        read_stream(BufReader::new(response.into_body().into_reader()), on_delta, cancelled)
    }
}

// ── Retries ──────────────────────────────────────────────────────────────────────────────────────

/// How finely the backoff sleep is chopped, so cancelling a turn during one is felt promptly rather
/// than up to half a minute later.
const SLEEP_SLICE: Duration = Duration::from_millis(50);

/// Exponential backoff, capped. Deterministic — no jitter — because there is exactly one client in
/// this process and jitter exists to de-correlate a fleet.
///
/// A struct rather than three constants so the tests can set `base` to zero. Sleeping for real in a
/// unit test buys nothing: what is worth asserting is the *plan* the UI is shown, and a test that
/// spends a second proving `Duration::from_secs(1)` has been added to every future `cargo test`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts, not retries: `1` would mean never retrying.
    pub attempts: u32,
    pub base: Duration,
    pub max: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { attempts: 5, base: Duration::from_secs(1), max: Duration::from_secs(32) }
    }
}

impl RetryPolicy {
    pub fn backoff_for(&self, attempt: u32) -> Duration {
        let doubled = self
            .base
            .saturating_mul(1u32.checked_shl(attempt.saturating_sub(1)).unwrap_or(u32::MAX));
        doubled.min(self.max)
    }
}

/// What happened between two attempts, for the UI. A rate limit that resolves itself in eight
/// seconds should still be *visible*: an invisible one looks like the model thinking slowly, and the
/// two want completely different responses from whoever is watching.
pub struct Retry<'a> {
    pub attempt: u32,
    pub of: u32,
    pub waiting: Duration,
    pub failure: &'a LlmError,
    /// Whether the failed attempt had already streamed prose into the UI, so the replacement's text
    /// will read as a repetition unless the UI says a restart happened.
    pub already_spoke: bool,
}

/// [`ChatEndpoint::stream_completion`] with exponential backoff on the transient failures — 429,
/// 5xx, and a connection that broke.
///
/// Cancellation wins over retrying, both during a stream and during a backoff: a turn whose question
/// is already stale should not be waiting sixteen seconds to ask it again.
pub fn stream_with_retries(
    policy: RetryPolicy,
    endpoint: &dyn ChatEndpoint,
    request: &ChatRequest,
    on_delta: &mut dyn FnMut(Fragment<'_>),
    cancelled: &dyn Fn() -> bool,
    on_retry: &mut dyn FnMut(Retry<'_>),
) -> Result<Completion, LlmError> {
    for attempt in 1..=policy.attempts {
        let mut spoke = false;
        let result = endpoint.stream_completion(
            request,
            &mut |delta| {
                spoke = true;
                on_delta(delta);
            },
            cancelled,
        );

        let failure = match result {
            Ok(completion) => return Ok(completion),
            Err(failure) => failure,
        };
        if attempt == policy.attempts || !failure.is_retryable() || cancelled() {
            return Err(failure);
        }

        let waiting = policy.backoff_for(attempt);
        on_retry(Retry { attempt, of: policy.attempts, waiting, failure: &failure, already_spoke: spoke });
        if !sleep_unless_cancelled(waiting, cancelled) {
            return Err(LlmError::Cancelled);
        }
    }
    unreachable!("the loop returns on its last attempt")
}

/// Returns `false` if the wait was cut short by cancellation.
fn sleep_unless_cancelled(total: Duration, cancelled: &dyn Fn() -> bool) -> bool {
    let mut slept = Duration::ZERO;
    while slept < total {
        if cancelled() {
            return false;
        }
        let slice = SLEEP_SLICE.min(total - slept);
        std::thread::sleep(slice);
        slept += slice;
    }
    !cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::llm::protocol::{Message, StreamOptions};

    fn request() -> ChatRequest {
        ChatRequest {
            model: "test".into(),
            messages: vec![Message::user("go")],
            tools: Vec::new(),
            parallel_tool_calls: Some(true),
            temperature: 1.0,
            stream: true,
            stream_options: StreamOptions { include_usage: true },
        }
    }

    /// An endpoint that replays a script of outcomes, one per attempt.
    struct Scripted {
        outcomes: RefCell<Vec<Result<Completion, LlmError>>>,
        /// Text each attempt emits before its outcome, to exercise the `already_spoke` reporting.
        says: &'static str,
        attempts: std::cell::Cell<u32>,
    }

    impl ChatEndpoint for Scripted {
        fn stream_completion(
            &self,
            _request: &ChatRequest,
            on_delta: &mut dyn FnMut(Fragment<'_>),
            _cancelled: &dyn Fn() -> bool,
        ) -> Result<Completion, LlmError> {
            self.attempts.set(self.attempts.get() + 1);
            if !self.says.is_empty() {
                on_delta(Fragment::Content(self.says));
            }
            self.outcomes.borrow_mut().remove(0)
        }
    }

    // Only ever used from one thread; the bound is on the trait for the real worker's sake.
    unsafe impl Send for Scripted {}

    fn scripted(says: &'static str, outcomes: Vec<Result<Completion, LlmError>>) -> Scripted {
        Scripted { outcomes: RefCell::new(outcomes), says, attempts: std::cell::Cell::new(0) }
    }

    fn completion(content: &str) -> Completion {
        Completion { content: content.into(), ..Completion::default() }
    }

    /// Zero base: the tests assert on the *plan* the UI is shown, never on a wall clock.
    fn instant() -> RetryPolicy {
        RetryPolicy { base: Duration::ZERO, ..RetryPolicy::default() }
    }

    #[test]
    fn the_backoff_doubles_and_is_capped() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.backoff_for(1), Duration::from_secs(1));
        assert_eq!(policy.backoff_for(2), Duration::from_secs(2));
        assert_eq!(policy.backoff_for(4), Duration::from_secs(8));
        assert_eq!(policy.backoff_for(30), policy.max, "no overflow, and no unbounded wait");
    }

    /// 429 then 200. The interesting part is not that it retried but that the UI was told — and told
    /// that the reply it had already started reading is going to start again.
    #[test]
    fn a_rate_limit_is_retried_and_reported() {
        let endpoint = scripted("thinking…", vec![
            Err(LlmError::Http { status: 429, message: "slow down".into() }),
            Ok(completion("done")),
        ]);
        let mut retries = Vec::new();
        let completion = stream_with_retries(
            RetryPolicy { base: Duration::ZERO, ..RetryPolicy::default() },
            &endpoint,
            &request(),
            &mut |_| {},
            &|| false,
            &mut |retry| {
                retries.push((retry.attempt, retry.of, retry.already_spoke, format!("{}", retry.failure)));
            },
        )
        .expect("the second attempt succeeds");

        assert_eq!(completion.content, "done");
        assert_eq!(endpoint.attempts.get(), 2);
        assert_eq!(retries.len(), 1);
        assert_eq!((retries[0].0, retries[0].1), (1, 5));
        assert!(retries[0].2, "the failed attempt had already streamed prose");
        assert!(retries[0].3.contains("429"), "{}", retries[0].3);
    }

    /// A 400 is the request being wrong. Trying it four more times only spends four more seconds
    /// being wrong.
    #[test]
    fn a_client_error_is_not_retried() {
        let endpoint = scripted("", vec![Err(LlmError::Http { status: 400, message: "bad tool schema".into() })]);
        let failure = stream_with_retries(instant(), &endpoint, &request(), &mut |_| {}, &|| false, &mut |_| {})
            .expect_err("400 is fatal");
        assert!(matches!(failure, LlmError::Http { status: 400, .. }), "{failure}");
        assert_eq!(endpoint.attempts.get(), 1);
    }

    /// Cancellation beats retrying: the question is already stale, so waiting to ask it again is
    /// pure delay.
    #[test]
    fn cancellation_stops_the_retry_loop_rather_than_backing_off() {
        let endpoint = scripted("", vec![Err(LlmError::Transport("connection reset".into()))]);
        let failure = stream_with_retries(instant(), &endpoint, &request(), &mut |_| {}, &|| true, &mut |_| {})
            .expect_err("cancelled");
        assert_eq!(failure, LlmError::Transport("connection reset".into()),
                   "the real failure is reported, not masked by the cancellation");
        assert_eq!(endpoint.attempts.get(), 1, "no second attempt was made");
    }

    /// The budget is finite. A permanently broken endpoint must stop the turn rather than hold the
    /// policy at `None` forever, which the agent cannot distinguish from a model thinking.
    #[test]
    fn a_persistent_fault_gives_up_after_the_last_attempt() {
        let outcomes = (0..5).map(|_| Err(LlmError::Http { status: 503, message: "down".into() })).collect();
        let endpoint = scripted("", outcomes);
        let failure = stream_with_retries(instant(), &endpoint, &request(), &mut |_| {}, &|| false, &mut |_| {})
            .expect_err("still down");
        assert!(matches!(failure, LlmError::Http { status: 503, .. }), "{failure}");
        assert_eq!(endpoint.attempts.get(), 5);
    }

    #[test]
    fn a_backoff_sleep_is_cut_short_by_cancellation() {
        let started = std::time::Instant::now();
        assert!(!sleep_unless_cancelled(Duration::from_secs(30), &|| true));
        assert!(started.elapsed() < Duration::from_secs(1), "it waited: {:?}", started.elapsed());
    }
}
