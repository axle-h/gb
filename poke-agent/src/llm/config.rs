//! The LLM configuration, read once at startup and entirely from the environment.

#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    /// `OPENAI_BASE_URL`: any OpenAI-compatible endpoint, the public one by default.
    pub base_url: String,
    /// `OPENAI_API_KEY`, required.
    pub api_key: String,
    /// `GB_MODEL`, required.
    pub model: String,
    /// `GB_CONTEXT_LIMIT`: the context window in tokens.
    pub context_limit: u64,
    /// `GB_COMPACT_ABOVE`: the fraction of [`Self::context_limit`] at which the history is
    /// compacted, by eviction first and summarisation only if that leaves it still over.
    pub compact_above: f64,
    /// `GB_TEMPERATURE`.
    pub temperature: f32,
    /// `GB_MAX_TOOL_STEPS`: the most completions one turn may take.
    pub max_tool_steps: usize,
    /// `GB_REQUEST_TIMEOUT_SECS`: how long the endpoint may take to start, and to keep, answering
    /// before the request is abandoned as an [`LlmError::Timeout`](crate::llm::LlmError::Timeout).
    pub request_timeout: std::time::Duration,
    /// `GB_MAX_TOKENS`: a ceiling on one completion, or `None` for the endpoint's default.
    pub max_tokens: Option<u32>,
    /// `GB_REASONING_EFFORT`, sent as `reasoning_effort` when set, in the endpoint's vocabulary.
    pub reasoning_effort: Option<String>,
    /// `GB_STUCK_TIMEOUT_SECS`: emulated time without any decision point before the watchdog asks
    /// for a nudge; `None` when set to `0`, which turns it off.
    pub stuck_timeout: Option<std::time::Duration>,
}

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_CONTEXT_LIMIT: u64 = 128_000;
pub const DEFAULT_COMPACT_ABOVE: f64 = 0.85;
/// Above this ceiling the remaining window cannot hold the compaction summary, and the run
/// silently degrades to the last-resort trim.
pub const COMPACT_ABOVE_RANGE: std::ops::RangeInclusive<f64> = 0.2..=0.95;
pub const DEFAULT_TEMPERATURE: f32 = 1.0;
pub const DEFAULT_MAX_TOOL_STEPS: usize = 12;
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 180;
pub const DEFAULT_MAX_TOKENS: u32 = 8192;
/// In emulated seconds.
pub const DEFAULT_STUCK_TIMEOUT_SECS: u64 = 300;

impl LlmConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(&|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] against any lookup, so tests need not touch the process environment.
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
            // Rejected, not clamped: `GB_COMPACT_ABOVE=95` meant 0.95, and 0.85 would hide it.
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
            // Not validated: the endpoint owns the values and rejects a bad one in a 400 we keep.
            reasoning_effort: env("GB_REASONING_EFFORT")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            request_timeout: std::time::Duration::from_secs(number(
                env,
                "GB_REQUEST_TIMEOUT_SECS",
                DEFAULT_REQUEST_TIMEOUT_SECS,
            )?),
            stuck_timeout: match number(env, "GB_STUCK_TIMEOUT_SECS", DEFAULT_STUCK_TIMEOUT_SECS)? {
                0 => None,
                seconds => Some(std::time::Duration::from_secs(seconds)),
            },
        })
    }

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

/// `GB_RESTORE_HISTORY`: whether a resumed run keeps its conversation; on unless `0`, `false`,
/// `no` or `off`.
pub fn restore_history() -> bool {
    restores_history(std::env::var("GB_RESTORE_HISTORY").ok())
}

fn restores_history(value: Option<String>) -> bool {
    match value {
        // Blank reads as unset, the shape a placeholder Secret takes.
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

        // A percentage written as one, and a fraction written upside down.
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

    #[test]
    fn the_request_timeout_can_be_lengthened_for_a_slow_endpoint() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REQUEST_TIMEOUT_SECS", "900"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.request_timeout, std::time::Duration::from_secs(900));
    }

    #[test]
    fn a_zero_token_cap_removes_the_ceiling_rather_than_setting_it_to_nothing() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_MAX_TOKENS", "0"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").max_tokens, None);

        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_MAX_TOKENS", "2048"));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").max_tokens, Some(2048));
    }

    #[test]
    fn the_reasoning_effort_is_whatever_the_endpoint_calls_it() {
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REASONING_EFFORT", "none"));
        let config = LlmConfig::from_lookup(&lookup(&pairs)).expect("valid");
        assert_eq!(config.reasoning_effort.as_deref(), Some("none"));

        // Blank is an unfilled template and must read as unset.
        let mut pairs = MINIMAL.to_vec();
        pairs.push(("GB_REASONING_EFFORT", "   "));
        assert_eq!(LlmConfig::from_lookup(&lookup(&pairs)).expect("valid").reasoning_effort, None);
    }

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
