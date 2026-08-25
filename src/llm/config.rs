//! **W4 / §7.1** — the configuration block, entirely from the environment.
//!
//! Environment rather than flags, and never exposed to the browser: the API key is the reason, and
//! once the key has to be an environment variable it is simpler for everything beside it to be one
//! too than to have two mechanisms. `--port` is the exception, because it is the one setting an
//! operator changes while debugging, and it overrides `GB_PORT`.
//!
//! | Var | Default | Meaning |
//! |---|---|---|
//! | `OPENAI_BASE_URL` | `https://api.openai.com/v1` | Any compatible endpoint |
//! | `OPENAI_API_KEY` | — | Required for `--policy llm` |
//! | `GB_MODEL` | — | Required |
//! | `GB_CONTEXT_LIMIT` | `128000` | The window W6's compaction triggers a fraction of |
//! | `GB_COMPACT_ABOVE` | `0.85` | That fraction. `0.2`–`0.95`; see [`LlmConfig::compact_above`] |
//! | `GB_TEMPERATURE` | `1.0` | |
//! | `GB_MAX_TOOL_STEPS` | `12` | Non-terminal calls per turn before a decision is forced |
//! | `GB_REQUEST_TIMEOUT_SECS` | `180` | How long an endpoint may take to answer; see [`LlmConfig::request_timeout`] |
//! | `GB_MAX_TOKENS` | `8192` | Ceiling on one completion; `0` removes it |
//! | `GB_REASONING_EFFORT` | — | Passed through as `reasoning_effort` when set (`none`/`low`/…) |
//! | `GB_STUCK_TIMEOUT_SECS` | `300` | **W9** — emulated seconds with the agent asking nothing before the watchdog does; `0` is off |
//! | `GB_PORT` | `8080` | Read in `cli.rs`, since it applies to `--policy random` too |
//! | `GB_RUN_DIR` | `runs` | Read in `web/mod.rs`, for the same reason (**W7**) |
//!
//! `GB_RUN_DIR` is **W7's and is read in `src/web/mod.rs`**, not here: it applies to
//! `--policy random` too, and the run directory is resolved before this block is — a missing API key
//! should be an error before a directory exists for a run that cannot start.
//!
//! `GB_PAUSE_WHILE_THINKING` (§2.1) was built in W4 and removed the same day. Freezing the emulator
//! while the model thinks is never what this thing is for — the live picture is the product, and a
//! frozen one is a worse watch than a slightly stale one under every circumstance we could name. It
//! was also the only setting that could deadlock a run: a tool batch is answered at the policy poll,
//! and the policy is only polled when `gb.run` advances the agent, so any pause that spanned a tool
//! round trip hung the run on the first `read_map`. Keeping a knob whose *on* position is a footgun,
//! for a behaviour nobody wants, is worse than not having it.

/// Everything the worker and the client need, resolved once at startup.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// The context window in tokens. W4 only reports against it; W6 compacts on it.
    pub context_limit: u64,
    /// **W6 / §9** — the occupancy at which the history is compacted, as a fraction of
    /// [`Self::context_limit`]. Both stages trigger here: eviction first, and summarisation only if
    /// eviction left it still over.
    ///
    /// Measured on the calibrated scale — see [`crate::llm::accounting`], whose whole reason for
    /// existing is that "85% full" has to mean the same thing before and after a message is removed.
    ///
    /// ⚠️ **What the remaining fraction has to pay for is not one message — it is a whole turn and a
    /// summary.** Compaction runs *between* turns, so once a turn has started the history grows
    /// unchecked until it ends: up to `GB_MAX_TOOL_STEPS` completions, their tool results and two
    /// screenshots. And stage 2's request carries the entire history *plus* room for the summary the
    /// model writes back, which on a reasoning model is the summary plus everything it thought on the
    /// way to it. Both of those are absolute token counts, so the headroom that matters is
    /// `(1 - compact_above) × context_limit` rather than the percentage: 15% of 128 k is 19 k and
    /// comfortable, 15% of 60 k is 9 k and merely adequate, 5% of 60 k is 3 k and will not fit a
    /// summary.
    ///
    /// Going over is not fatal — a failed summary falls back to [`trim_history`] and a failed turn
    /// resolves to a wait — but each one costs the run either its memory or a turn.
    ///
    /// [`trim_history`]: crate::llm::worker
    pub compact_above: f64,
    pub temperature: f32,
    /// Non-terminal tool calls a single turn may make before the worker forces `wait` (§7.3).
    pub max_tool_steps: usize,
    /// How long the endpoint may take to start answering, and to keep answering, before the request
    /// is abandoned as an [`LlmError::Timeout`](crate::llm::LlmError::Timeout).
    ///
    /// ⚠️ **Abandoning is not free, so this wants to be generous rather than tight.** A hosted API
    /// answers in milliseconds and a dead one never answers at all, which is the case the default was
    /// sized for. A local server is neither: it accepts the request, works on it, and keeps working
    /// after we hang up — llama.cpp prints "Stopping generation… (If the model is busy processing the
    /// prompt, it will finish first.)" — so every expiry here leaves a piece of work running that
    /// nobody will ever read, on a machine that may serve only one request at a time. Waiting longer
    /// costs a stalled turn; giving up early costs the same stalled turn *and* the endpoint's next
    /// few minutes.
    pub request_timeout: std::time::Duration,
    /// A ceiling on one completion, or `None` for whatever the endpoint does by default.
    ///
    /// ⚠️ **The context window is not a usable ceiling.** An uncapped reasoning model that falls into
    /// a repetition loop generates until the window is full — measured at ~26 000 tokens against
    /// turns that normally cost 24–2 000 — and on a single-slot local endpoint nothing else can be
    /// decided for as long as that takes. The default is deliberately far above any observed
    /// legitimate turn (a compaction summary, the longest thing the loop asks for, runs to a couple
    /// of thousand) so that hitting it means something has gone wrong rather than that the number is
    /// too small.
    pub max_tokens: Option<u32>,
    /// `reasoning_effort`, passed straight through when set. See [`ChatRequest::reasoning_effort`]
    /// for what the values actually do — it is the endpoint's vocabulary, not ours.
    ///
    /// [`ChatRequest::reasoning_effort`]: crate::llm::protocol::ChatRequest::reasoning_effort
    pub reasoning_effort: Option<String>,
    /// **W9 / §14** — how much *emulated* time the agent may go without reaching a decision point of
    /// any kind before the watchdog asks for a nudge on its behalf. `None` when
    /// `GB_STUCK_TIMEOUT_SECS=0`, which turns it off.
    ///
    /// Deliberately generous: normal play never approaches five minutes of game time without asking
    /// something, and this is insurance against an agent bug rather than a mechanism the design
    /// leans on. Every firing is a bug report.
    pub stuck_timeout: Option<std::time::Duration>,
}

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_CONTEXT_LIMIT: u64 = 128_000;
/// **W6 / §9.** 0.70 originally, which was never measured against anything — it was headroom chosen
/// on the assumption of a large window, where 30% is tens of thousands of tokens. What the headroom
/// actually has to cover is bounded and small (see [`LlmConfig::compact_above`]), so the cost of
/// 0.70 was real and paid every turn: a fifth of a paid-for window held empty, and a summarising
/// completion — the most expensive thing the loop does — bought sooner and more often than needed.
pub const DEFAULT_COMPACT_ABOVE: f64 = 0.85;
/// The range [`DEFAULT_COMPACT_ABOVE`] may be moved through. The ceiling is not superstition: above
/// it, the remaining window cannot hold the summary that compaction exists to produce, so the run
/// silently degrades to the last-resort trim. The floor keeps a typo like `0.05` from summarising
/// on every single turn.
pub const COMPACT_ABOVE_RANGE: std::ops::RangeInclusive<f64> = 0.2..=0.95;
pub const DEFAULT_TEMPERATURE: f32 = 1.0;
pub const DEFAULT_MAX_TOOL_STEPS: usize = 12;
/// Three minutes. Enough for any hosted endpoint and for a local one that is merely slow; see
/// [`LlmConfig::request_timeout`] for why the number wants to grow rather than shrink.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 180;
/// Generous by design — see [`LlmConfig::max_tokens`]. `GB_MAX_TOKENS=0` removes the cap entirely,
/// which is the pre-2026-08-12 behaviour and a footgun on any endpoint that serves one request at a
/// time.
pub const DEFAULT_MAX_TOKENS: u32 = 8192;
/// Five minutes of *emulated* time. **W9 / §14.**
pub const DEFAULT_STUCK_TIMEOUT_SECS: u64 = 300;

impl LlmConfig {
    /// Read the block from the process environment.
    ///
    /// `Err` is a complete sentence naming the variable, because the overwhelmingly common failure is
    /// starting the container without one of the two required ones.
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(&|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] against an arbitrary lookup, so the parsing and the defaults are testable
    /// without touching the real environment — which is process-global and would make the tests
    /// order-dependent.
    pub fn from_lookup(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let required = |name: &str| -> Result<String, String> {
            match env(name).map(|value| value.trim().to_string()) {
                Some(value) if !value.is_empty() => Ok(value),
                _ => Err(format!("`--policy llm` needs {name} in the environment")),
            }
        };

        Ok(Self {
            // A trailing slash here and the request path would double it. Endpoints vary on whether
            // they forgive that; none of them mind it being absent.
            base_url: env("OPENAI_BASE_URL")
                .map(|url| url.trim().trim_end_matches('/').to_string())
                .filter(|url| !url.is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            api_key: required("OPENAI_API_KEY")?,
            model: required("GB_MODEL")?,
            context_limit: number(env, "GB_CONTEXT_LIMIT", DEFAULT_CONTEXT_LIMIT)?,
            // Rejected rather than clamped: a run started with `GB_COMPACT_ABOVE=95` meant to say
            // 0.95, and quietly playing on at 0.85 would hide that for the length of the run.
            compact_above: match number(env, "GB_COMPACT_ABOVE", DEFAULT_COMPACT_ABOVE)? {
                fraction if COMPACT_ABOVE_RANGE.contains(&fraction) => fraction,
                fraction => {
                    return Err(format!(
                        "`GB_COMPACT_ABOVE={fraction}` is outside {:?}–{:?}; it is the fraction of \
                         GB_CONTEXT_LIMIT the history is compacted at",
                        COMPACT_ABOVE_RANGE.start(),
                        COMPACT_ABOVE_RANGE.end(),
                    ));
                }
            },
            temperature: number(env, "GB_TEMPERATURE", DEFAULT_TEMPERATURE)?,
            max_tool_steps: number(env, "GB_MAX_TOOL_STEPS", DEFAULT_MAX_TOOL_STEPS)?,
            max_tokens: match number(env, "GB_MAX_TOKENS", DEFAULT_MAX_TOKENS)? {
                0 => None,
                cap => Some(cap),
            },
            // Not validated against a list: the accepted values belong to the endpoint, and refusing
            // one it would have taken is worse than passing through one it rejects — which it says
            // so, in a 400 whose body we keep.
            reasoning_effort: env("GB_REASONING_EFFORT")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            request_timeout: std::time::Duration::from_secs(number(
                env,
                "GB_REQUEST_TIMEOUT_SECS",
                DEFAULT_REQUEST_TIMEOUT_SECS,
            )?),
            // Zero is "off" rather than "fire on every tick", which is the only reading that makes
            // the variable a way to turn the watchdog off.
            stuck_timeout: match number(env, "GB_STUCK_TIMEOUT_SECS", DEFAULT_STUCK_TIMEOUT_SECS)? {
                0 => None,
                seconds => Some(std::time::Duration::from_secs(seconds)),
            },
        })
    }

    /// Where the completions live. Split out because it is the one string most likely to be wrong
    /// against a non-OpenAI endpoint, and an error saying which URL was tried is worth a great deal
    /// more than one that does not.
    pub fn completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

fn number<T>(env: &dyn Fn(&str) -> Option<String>, name: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr,
{
    match env(name).map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
        Some(value) => value.parse().map_err(|_| format!("`{name}={value}` is not a number")),
        None => Ok(default),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    const MINIMAL: &[(&str, &str)] = &[("OPENAI_API_KEY", "sk-test"), ("GB_MODEL", "gpt-test")];

    /// **Seven characters is the game's limit and a model id is nothing like seven characters**, so
    /// the whole question is what to throw away. Whole segments from the front, because the family
    /// and its version are what a viewer recognises — and because truncating the joined string
    /// instead invents version numbers (`gemma-3-12b` → `GEMMA31`).
    #[test]
    fn the_two_required_variables_are_the_only_two_required() {
        let config = LlmConfig::from_lookup(&lookup(MINIMAL)).expect("the defaults cover the rest");
        assert_eq!(config.base_url, DEFAULT_BASE_URL);
        assert_eq!(config.completions_url(), "https://api.openai.com/v1/chat/completions");
        assert_eq!(config.context_limit, DEFAULT_CONTEXT_LIMIT);
        assert_eq!(config.compact_above, DEFAULT_COMPACT_ABOVE);
        assert_eq!(config.request_timeout.as_secs(), DEFAULT_REQUEST_TIMEOUT_SECS);
        assert_eq!(config.max_tokens, Some(DEFAULT_MAX_TOKENS));
        assert_eq!(config.reasoning_effort, None, "the key is omitted unless it is asked for");
        assert_eq!(config.max_tool_steps, DEFAULT_MAX_TOOL_STEPS);
        assert_eq!(config.stuck_timeout, Some(std::time::Duration::from_secs(DEFAULT_STUCK_TIMEOUT_SECS)));
    }

    /// **W9.** Zero is the off switch, and it has to be *off* rather than "a timeout of zero", which
    /// would fire the watchdog on every tick of every run — a turn per 20 ms, and a bill to match.
    #[test]
    fn a_zero_stuck_timeout_turns_the_watchdog_off() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_STUCK_TIMEOUT_SECS", "0"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").stuck_timeout, None);

        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_STUCK_TIMEOUT_SECS", "45"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.stuck_timeout, Some(std::time::Duration::from_secs(45)));

        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_STUCK_TIMEOUT_SECS", "ages"));
        let failure = LlmConfig::from_lookup(&lookup(&pairs)).expect_err("not a number");
        assert!(failure.contains("GB_STUCK_TIMEOUT_SECS"), "{failure}");
    }

    /// The failure an operator actually hits, and it must name the variable rather than say "config".
    #[test]
    fn a_missing_or_blank_requirement_names_itself() {
        for (missing, present) in [("OPENAI_API_KEY", "GB_MODEL"), ("GB_MODEL", "OPENAI_API_KEY")] {
            for blank in ["", "   "] {
                let pairs = [(present, "x"), (missing, blank)];
                let env = lookup(&pairs);
                let failure = LlmConfig::from_lookup(&env).expect_err("a requirement is missing");
                assert!(failure.contains(missing), "{failure}");
            }
        }
    }

    /// **W6 / §9.** The threshold moves, and an unusable one is refused rather than clamped: the
    /// window it is a fraction of can be as small as a local model's 60 k, where the last few percent
    /// are the only room the summarising completion has to be written in.
    #[test]
    fn the_compaction_threshold_can_be_moved_but_not_off_the_end() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_COMPACT_ABOVE", "0.9"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").compact_above, 0.9);

        // The two shapes of typo that matter: a percentage written as one, and a fraction written
        // upside down. Both would otherwise be a run that compacts every turn or never at all.
        for bad in ["90", "0.05", "1.0", "-0.5"] {
            let mut pairs = MINIMAL.to_vec();
            pairs.push(("GB_COMPACT_ABOVE", bad));
            let env = lookup(&pairs);
            let failure = LlmConfig::from_lookup(&env).expect_err("`{bad}` is not a usable fraction");
            assert!(failure.contains("GB_COMPACT_ABOVE"), "{failure}");
        }

        assert!(
            COMPACT_ABOVE_RANGE.contains(&DEFAULT_COMPACT_ABOVE),
            "the default has to be a value the variable would accept",
        );
    }

    /// The patience knob. Its whole purpose is to be raised for a local endpoint, so the test that
    /// matters is that a big number survives the parse rather than that the default is 180.
    #[test]
    fn the_request_timeout_can_be_lengthened_for_a_slow_endpoint() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REQUEST_TIMEOUT_SECS", "900"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.request_timeout, std::time::Duration::from_secs(900));
    }

    /// ⚠️ Zero is "no ceiling", not "a ceiling of zero" — the same reading as `GB_STUCK_TIMEOUT_SECS`,
    /// and the only one that makes the variable a way to restore the endpoint's own default.
    #[test]
    fn a_zero_token_cap_removes_the_ceiling_rather_than_setting_it_to_nothing() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_MAX_TOKENS", "0"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").max_tokens, None);

        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_MAX_TOKENS", "2048"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").max_tokens, Some(2048));
    }

    /// Passed through verbatim and *not* validated: the vocabulary is the endpoint's. On LM Studio
    /// with gemma-4, `none` is the only value that measurably does anything.
    #[test]
    fn the_reasoning_effort_is_whatever_the_endpoint_calls_it() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REASONING_EFFORT", "none"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.reasoning_effort.as_deref(), Some("none"));

        // Blank is not a value: it is the variable being present in a template and never filled in,
        // which must read the same as unset or the endpoint gets an empty string it will reject.
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REASONING_EFFORT", "   "));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").reasoning_effort, None);
    }

    /// A trailing slash in `OPENAI_BASE_URL` is the single most common way to get a 404 out of a
    /// self-hosted endpoint, and it costs one `trim_end_matches` to make impossible.
    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_up() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("OPENAI_BASE_URL", "http://localhost:11434/v1/"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.completions_url(), "http://localhost:11434/v1/chat/completions");
    }

    #[test]
    fn the_optional_settings_parse_and_report_what_they_reject() {
        let mut pairs = MINIMAL.to_vec();
        pairs.extend([
            ("GB_CONTEXT_LIMIT", "32000"),
            ("GB_TEMPERATURE", "0.2"),
            ("GB_MAX_TOOL_STEPS", "4"),
        ]);
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.context_limit, 32_000);
        assert_eq!(config.temperature, 0.2);
        assert_eq!(config.max_tool_steps, 4);

        for (name, bad) in [("GB_CONTEXT_LIMIT", "lots"), ("GB_MAX_TOOL_STEPS", "a few")] {
            let mut pairs = MINIMAL.to_vec();
            pairs.push((name, bad));
            let env = lookup(&pairs);
            let failure = LlmConfig::from_lookup(&env).expect_err("the value is nonsense");
            assert!(failure.contains(name) && failure.contains(bad), "{failure}");
        }
    }
}
