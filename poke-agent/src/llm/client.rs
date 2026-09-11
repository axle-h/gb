//! The HTTP half: one blocking `POST /chat/completions` with `stream: true`.

use std::io::BufReader;
use std::time::Duration;

use crate::llm::LlmError;
use crate::llm::config::LlmConfig;
use crate::llm::protocol::{
    ChatRequest, Completion, Fragment, describe_error_body, read_stream, reset_at_ms,
};
use crate::published::now_ms;

/// One streamed chat completion.
pub trait ChatEndpoint: Send {
    fn stream_completion(
        &self,
        request: &ChatRequest,
        on_delta: &mut dyn FnMut(Fragment<'_>),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Completion, LlmError>;
}

/// How long to wait for the endpoint to start answering.
const TIMEOUT_CONNECT: Duration = Duration::from_secs(30);

pub struct OpenAiClient {
    agent: ureq::Agent,
    url: String,
    authorization: String,
}

impl OpenAiClient {
    pub fn new(config: &LlmConfig) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // A non-2xx is read below, for its body and its rate-limit headers.
            .http_status_as_error(false)
            .timeout_connect(Some(TIMEOUT_CONNECT))
            // Both are `GB_REQUEST_TIMEOUT_SECS`: the wait to start answering, and any gap after.
            .timeout_recv_response(Some(config.request_timeout))
            .timeout_recv_body(Some(config.request_timeout))
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
            // A compressed SSE body arrives in whole decoder blocks rather than token by token.
            .header("Accept-Encoding", "identity")
            .send(body)
            .map_err(|e| match e {
                // The endpoint has the request and is sitting on it, unlike never reaching it.
                ureq::Error::Timeout(_) => {
                    LlmError::Timeout(format!("POST {} was not answered: {e}", self.url))
                }
                e => LlmError::Transport(format!("POST {} failed: {e}", self.url)),
            })?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // Read before the body, because reading the body consumes the response.
            let header = |name: &str| {
                response.headers().get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
            };
            let (retry_after, reset) = (header("retry-after"), header("x-ratelimit-reset"));
            let message = response
                .into_body()
                .read_to_string()
                .map(|body| describe_error_body(&body))
                .unwrap_or_else(|e| format!("(the error body could not be read: {e})"));
            if status == 429 {
                let resets_at_ms = reset_at_ms(retry_after.as_deref(), reset.as_deref(), now_ms());
                return Err(LlmError::RateLimited { resets_at_ms, message });
            }
            return Err(LlmError::Http { status, message });
        }

        read_stream(BufReader::new(response.into_body().into_reader()), on_delta, cancelled)
    }
}

// ── Retries ──────────────────────────────────────────────────────────────────────────────────────

/// How finely the backoff sleep is chopped, so a cancelled turn is felt promptly.
const SLEEP_SLICE: Duration = Duration::from_millis(50);

/// Exponential backoff, capped, with no jitter: there is one client in this process.
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

/// What happened between two attempts, so the UI can tell a rate limit from a slow model.
pub struct Retry<'a> {
    pub attempt: u32,
    pub of: u32,
    pub waiting: Duration,
    pub failure: &'a LlmError,
    /// The failed attempt had streamed prose, so the retry's text would read as a repeat.
    pub already_spoke: bool,
}

/// [`ChatEndpoint::stream_completion`] with backoff on the transient failures.
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
        // A rate limit dated beyond the backoff's reach goes straight up: each retry would be
        // another request against a quota that has run out.
        if let LlmError::RateLimited { resets_at_ms: Some(at), .. } = &failure {
            if *at > now_ms().saturating_add(policy.max.as_millis() as u64) {
                return Err(failure);
            }
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
            max_tokens: None,
            reasoning_effort: None,
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

    /// Zero base: the tests assert on the plan the UI is shown, never on a wall clock.
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

    #[test]
    fn a_rate_limit_dated_beyond_the_backoff_is_not_retried_at_all() {
        let hour_away = now_ms() + 60 * 60 * 1000;
        let endpoint = scripted("", vec![
            Err(LlmError::RateLimited { resets_at_ms: Some(hour_away), message: "50/day".into() }),
            Ok(completion("never reached")),
        ]);
        let mut retries = 0;
        let failure = stream_with_retries(instant(), &endpoint, &request(), &mut |_| {}, &|| false, &mut |_| {
            retries += 1;
        })
        .expect_err("a spent daily quota is not something to retry into");

        assert_eq!(endpoint.attempts.get(), 1, "the quota must not be spent finding out again");
        assert_eq!(retries, 0, "and the UI is not told to expect a retry that is not coming");
        match failure {
            LlmError::RateLimited { resets_at_ms, .. } => assert_eq!(resets_at_ms, Some(hour_away)),
            other => panic!("the deadline has to survive to the caller: {other}"),
        }
    }

    #[test]
    fn a_rate_limit_within_reach_of_the_backoff_is_still_retried() {
        for resets_at_ms in [None, Some(now_ms() + 2_000)] {
            let endpoint = scripted("", vec![
                Err(LlmError::RateLimited { resets_at_ms, message: "20/min".into() }),
                Ok(completion("done")),
            ]);
            let completion =
                stream_with_retries(instant(), &endpoint, &request(), &mut |_| {}, &|| false, &mut |_| {})
                    .expect("the second attempt succeeds");
            assert_eq!(completion.content, "done", "{resets_at_ms:?}");
            assert_eq!(endpoint.attempts.get(), 2, "{resets_at_ms:?}");
        }
    }

    #[test]
    fn a_client_error_is_not_retried() {
        let endpoint = scripted("", vec![Err(LlmError::Http { status: 400, message: "bad tool schema".into() })]);
        let failure = stream_with_retries(instant(), &endpoint, &request(), &mut |_| {}, &|| false, &mut |_| {})
            .expect_err("400 is fatal");
        assert!(matches!(failure, LlmError::Http { status: 400, .. }), "{failure}");
        assert_eq!(endpoint.attempts.get(), 1);
    }

    #[test]
    fn cancellation_stops_the_retry_loop_rather_than_backing_off() {
        let endpoint = scripted("", vec![Err(LlmError::Transport("connection reset".into()))]);
        let failure = stream_with_retries(instant(), &endpoint, &request(), &mut |_| {}, &|| true, &mut |_| {})
            .expect_err("cancelled");
        assert_eq!(failure, LlmError::Transport("connection reset".into()),
                   "the real failure is reported, not masked by the cancellation");
        assert_eq!(endpoint.attempts.get(), 1, "no second attempt was made");
    }

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
