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
//! | `GB_CONTEXT_LIMIT` | `128000` | The window W6's compaction triggers at 70% of |
//! | `GB_TEMPERATURE` | `1.0` | |
//! | `GB_MAX_TOOL_STEPS` | `12` | Non-terminal calls per turn before a decision is forced |
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
    pub temperature: f32,
    /// Non-terminal tool calls a single turn may make before the worker forces `wait` (§7.3).
    pub max_tool_steps: usize,
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
pub const DEFAULT_TEMPERATURE: f32 = 1.0;
pub const DEFAULT_MAX_TOOL_STEPS: usize = 12;
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
            temperature: number(env, "GB_TEMPERATURE", DEFAULT_TEMPERATURE)?,
            max_tool_steps: number(env, "GB_MAX_TOOL_STEPS", DEFAULT_MAX_TOOL_STEPS)?,
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

    #[test]
    fn the_two_required_variables_are_the_only_two_required() {
        let config = LlmConfig::from_lookup(&lookup(MINIMAL)).expect("the defaults cover the rest");
        assert_eq!(config.base_url, DEFAULT_BASE_URL);
        assert_eq!(config.completions_url(), "https://api.openai.com/v1/chat/completions");
        assert_eq!(config.context_limit, DEFAULT_CONTEXT_LIMIT);
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
