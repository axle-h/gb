//! The configuration block, entirely from the environment.

/// Everything the worker and the client need, resolved once at startup.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// The context window in tokens.
    pub context_limit: u64,
    /// The occupancy at which the history is compacted, as a fraction of [`Self::context_limit`].
    /// Both stages trigger here: eviction first, and summarisation only if eviction left it still
    /// over.
    pub compact_above: f64,
    pub temperature: f32,
    pub max_tool_steps: usize,
    /// How long the endpoint may take to start answering, and to keep answering, before the
    /// request is abandoned as an [`LlmError::Timeout`](crate::llm::LlmError::Timeout).
    pub request_timeout: std::time::Duration,
    /// A ceiling on one completion, or `None` for whatever the endpoint does by default.
    pub max_tokens: Option<u32>,
    /// `reasoning_effort`, passed straight through when set. See
    /// `ChatRequest::reasoning_effort` for what the values actually do — it is the endpoint's
    /// vocabulary, not ours.
    pub reasoning_effort: Option<String>,
    /// How much *emulated* time the agent may go without reaching a decision point of any kind
    /// before the watchdog asks for a nudge on its behalf. `None` when `GB_STUCK_TIMEOUT_SECS=0`,
    /// which turns it off.
    pub stuck_timeout: Option<std::time::Duration>,
}

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_CONTEXT_LIMIT: u64 = 128_000;
pub const DEFAULT_COMPACT_ABOVE: f64 = 0.85;
/// The range [`DEFAULT_COMPACT_ABOVE`] may be moved through. The ceiling is not superstition:
/// above it, the remaining window cannot hold the summary that compaction exists to produce, so
/// the run silently degrades to the last-resort trim.
pub const COMPACT_ABOVE_RANGE: std::ops::RangeInclusive<f64> = 0.2..=0.95;
pub const DEFAULT_TEMPERATURE: f32 = 1.0;
pub const DEFAULT_MAX_TOOL_STEPS: usize = 12;
/// Three minutes. Enough for any hosted endpoint and for a local one that is merely slow; see
/// [`LlmConfig::request_timeout`] for why the number wants to grow rather than shrink.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 180;
/// Generous by design — see [`LlmConfig::max_tokens`].
pub const DEFAULT_MAX_TOKENS: u32 = 8192;
/// Five minutes of *emulated* time.
pub const DEFAULT_STUCK_TIMEOUT_SECS: u64 = 300;

impl LlmConfig {
    /// Read the block from the process environment.
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(&|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] against an arbitrary lookup, so the parsing and the defaults are
    /// testable without touching the real environment — which is process-global and would make
    /// the tests order-dependent.
    pub fn from_lookup(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let required = |name: &str| -> Result<String, String> {
            match env(name).map(|value| value.trim().to_string()) {
                Some(value) if !value.is_empty() => Ok(value),
                _ => Err(format!("`--policy llm` needs {name} in the environment")),
            }
        };

        Ok(Self {
            // A trailing slash here and the request path would double it.
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
            // Not validated against a list: the accepted values belong to the endpoint, and
            // refusing one it would have taken is worse than passing through one it rejects —
            // which it says so, in a 400 whose body we keep.
            reasoning_effort: env("GB_REASONING_EFFORT")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            request_timeout: std::time::Duration::from_secs(number(
                env,
                "GB_REQUEST_TIMEOUT_SECS",
                DEFAULT_REQUEST_TIMEOUT_SECS,
            )?),
            // Zero is "off" rather than "fire on every tick", which is the only reading that
            // makes the variable a way to turn the watchdog off.
            stuck_timeout: match number(env, "GB_STUCK_TIMEOUT_SECS", DEFAULT_STUCK_TIMEOUT_SECS)? {
                0 => None,
                seconds => Some(std::time::Duration::from_secs(seconds)),
            },
        })
    }

    /// Where the completions live. Split out because it is the one string most likely to be wrong
    /// against a non-OpenAI endpoint, and an error saying which URL was tried is worth a great
    /// deal more than one that does not.
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

/// Whether a resumed run picks its conversation back up — `GB_RESTORE_HISTORY`, on unless it is
/// set to `0`, `false`, `no` or `off`.
pub fn restore_history() -> bool {
    restores_history(std::env::var("GB_RESTORE_HISTORY").ok())
}

/// The reading, split out so it can be tested without touching the process environment — the same
/// reason [`LlmConfig::from_lookup`] exists.
fn restores_history(value: Option<String>) -> bool {
    match value {
        // Blank counts as unset, which is the shape a placeholder Secret takes — the same reading
        // `GB_ADMIN_TOKEN` already has, and the opposite of treating an empty string as "off" and
        // silently dropping every resumed conversation on the deployment.
        Some(value) => !matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        None => true,
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

    /// Seven characters is the game's limit and a model id is nothing like seven characters, so
    /// the whole question is what to throw away.
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

    /// The failure an operator actually hits, and it must name the variable rather than say
    /// "config".
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

    #[test]
    fn the_compaction_threshold_can_be_moved_but_not_off_the_end() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_COMPACT_ABOVE", "0.9"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").compact_above, 0.9);

        // The two shapes of typo that matter: a percentage written as one, and a fraction written
        // upside down.
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

    /// The patience knob.
    #[test]
    fn the_request_timeout_can_be_lengthened_for_a_slow_endpoint() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REQUEST_TIMEOUT_SECS", "900"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.request_timeout, std::time::Duration::from_secs(900));
    }

    /// Zero is "no ceiling", not "a ceiling of zero" — the same reading as
    /// `GB_STUCK_TIMEOUT_SECS`, and the only one that makes the variable a way to restore the
    /// endpoint's own default.
    #[test]
    fn a_zero_token_cap_removes_the_ceiling_rather_than_setting_it_to_nothing() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_MAX_TOKENS", "0"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").max_tokens, None);

        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_MAX_TOKENS", "2048"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").max_tokens, Some(2048));
    }

    /// Passed through verbatim and *not* validated: the vocabulary is the endpoint's.
    #[test]
    fn the_reasoning_effort_is_whatever_the_endpoint_calls_it() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REASONING_EFFORT", "none"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.reasoning_effort.as_deref(), Some("none"));

        // Blank is not a value: it is the variable being present in a template and never filled
        // in, which must read the same as unset or the endpoint gets an empty string it will
        // reject.
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

    /// The default has to be *on*, and blank has to read as unset.
    #[test]
    fn a_conversation_is_restored_unless_something_actually_says_not_to() {
        assert!(restores_history(None), "unset resumes the conversation");
        assert!(restores_history(Some(String::new())), "and so does a blank placeholder Secret");
        assert!(restores_history(Some("   ".into())));
        assert!(restores_history(Some("1".into())));
        assert!(restores_history(Some("yes".into())));

        for off in ["0", "false", "no", "off", "OFF", " False "] {
            assert!(!restores_history(Some(off.into())), "`{off}` should switch the restore off");
        }
    }
}
