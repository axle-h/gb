//! Command-line parsing, hand-rolled over `std::env::args`. `USAGE` names every flag and variable.

/// The crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const USAGE: &str = "\
poke-agent-web — serve a Pokémon Red run: the game, the streams and the web UI

USAGE:
    poke-agent-web [OPTIONS]    Serve the web UI and play under the chosen policy
    poke-agent-web --help       Print this message

OPTIONS:
    --port <PORT>               Port to listen on [default: $GB_PORT, else 8080]
    --policy <POLICY>           What plays the game [default: $GB_POLICY, else llm]:
                                  llm            a model over an OpenAI-compatible API
                                  random         random legal choices; no API key, no spend
                                  deterministic  the scripted route the full_playthrough test
                                                 plays, from a fresh save to the Hall of Fame,
                                                 at whatever speed the emulator is running.
                                                 Needs no API key either
    --new-run                   Start the game from the beginning, in a new run directory,
                                instead of resuming the newest resumable run

ENVIRONMENT (--policy llm):
    OPENAI_API_KEY, GB_MODEL    Required
    OPENAI_BASE_URL             Any OpenAI-compatible endpoint [default: api.openai.com/v1]
    GB_CONTEXT_LIMIT            The model's context window, in tokens [default: 128000]
    GB_COMPACT_ABOVE            How full the context gets before the history is compacted
                                [default: 0.85]
    GB_TEMPERATURE, GB_MAX_TOOL_STEPS
    GB_REQUEST_TIMEOUT_SECS     How long the endpoint may take to answer [default: 180]
    GB_MAX_TOKENS               Ceiling on one completion [default: 8192; 0 removes it]
    GB_REASONING_EFFORT         Sent as reasoning_effort when set; none turns thinking off
    GB_STUCK_TIMEOUT_SECS       Emulated seconds with the agent asking nothing at all before
                                the watchdog asks on its behalf [default: 300; 0 turns it off]
    GB_RESTORE_HISTORY          Resume a run's conversation as well as its save [default: 1;
                                0 starts the conversation over]

ENVIRONMENT (any policy):
    GB_PORT                     Port to listen on; --port wins
    GB_POLICY                   What plays the game: llm, random or deterministic; --policy wins
    GB_RUN_DIR                  Where runs are kept [default: ./runs]
    GB_STATUS_HZ                How often the status panel is sampled [default: 2]
    GB_HARDWARE                 Which Game Boy the cartridge runs on: dmg or cgb [default: dmg].
                                cgb is compatibility mode, so Pokemon Red comes out red-tinted
                                exactly as it does on real hardware, and the video stream costs
                                about 1.6x as much for the extra colours
    GB_AUDIO_BITRATE            What /api/audio's Opus stream targets, in bits per second
                                [default: 24000; 0 turns sound off]. Nothing is encoded until a
                                viewer turns the page's speaker on
    GB_BUILD_DATE, GB_GIT_BRANCH, GB_GIT_SHA
                                Set by the container image, not by you: the build date, branch
                                and short commit that GET /version serves and poke-agent-web
                                prints on the way up. Unset outside an image, and reported as null
                                rather than guessed at
    GB_ADMIN_TOKEN              Enables the three admin endpoints. /reset-game and POST
                                /api/new-run start the game over in a fresh run directory
                                without restarting the process; POST /api/clear keeps the run
                                and throws away what the model remembers of it, the
                                conversation and the plan. /reset-game takes the token as an
                                HTTP Basic password, the other two as the X-GB-Token header.
                                Unset — the default — and all three 404
";

/// What the process was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The web UI, served from this process.
    Serve {
        port: u16,
        policy: ServePolicy,
        /// Start the game from the beginning in a new directory rather than resuming.
        new_run: bool,
    },
    /// `--help`.
    Help,
}

/// Who makes the decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServePolicy {
    /// An LLM over an OpenAI-compatible API.
    Llm,
    /// Random legal choices.
    Random,
    /// `DeterministicPolicy` on the queue and fresh save `full_playthrough` runs.
    Deterministic,
}

impl ServePolicy {
    /// The spellings `--policy` and `GB_POLICY` share.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "llm" => Some(Self::Llm),
            "random" => Some(Self::Random),
            "deterministic" => Some(Self::Deterministic),
            _ => None,
        }
    }

    /// What a rejection offers instead, in both messages.
    const EXPECTED: &'static str = "`llm`, `random` or `deterministic`";
}

pub const DEFAULT_PORT: u16 = 8080;

/// Parse already-split arguments, excluding the program name.
pub fn parse<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parse_with_env(args, &|name| std::env::var(name).ok())
}

/// [`parse`] against an explicit environment.
pub fn parse_with_env<I, S>(args: I, env: &dyn Fn(&str) -> Option<String>) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
    let fail = |msg: String| Err(format!("{msg}\n\n{USAGE}"));

    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h" | "help")) {
        return Ok(Command::Help);
    }
    let mut rest = args.iter().map(String::as_str);

    let mut port = match env("GB_PORT").map(|value| value.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(value) => match value.parse::<u16>() {
            Ok(0) | Err(_) => return fail(format!("`GB_PORT={value}` is not a port number")),
            Ok(parsed) => parsed,
        },
        None => DEFAULT_PORT,
    };
    // `GB_POLICY` is the container's setting and `--policy` the operator's, which wins.
    let mut policy = match env("GB_POLICY").map(|value| value.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(value) => match ServePolicy::parse(&value) {
            Some(parsed) => parsed,
            None => return fail(format!("`GB_POLICY={value}` is not {}", ServePolicy::EXPECTED)),
        },
        None => ServePolicy::Llm,
    };
    let mut new_run = false;
    while let Some(flag) = rest.next() {
        // The one switch, taken before a value is demanded.
        if flag == "--new-run" {
            new_run = true;
            continue;
        }
        let value = match rest.next() {
            Some(value) => value,
            None => return fail(format!("`{flag}` needs a value")),
        };
        match flag {
            "--port" => match value.parse::<u16>() {
                Ok(0) => return fail("`--port 0` would bind an arbitrary port".to_string()),
                Ok(parsed) => port = parsed,
                Err(_) => return fail(format!("`--port {value}` is not a port number")),
            },
            "--policy" => match ServePolicy::parse(value) {
                Some(parsed) => policy = parsed,
                None => {
                    return fail(format!("`--policy {value}` — expected {}", ServePolicy::EXPECTED));
                }
            },
            other => return fail(format!("unknown option `{other}`")),
        }
    }
    Ok(Command::Serve { port, policy, new_run })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests must not see whatever the shell exported.
    fn parse<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> Result<Command, String> {
        parse_with_env(args, &|_| None)
    }

    #[test]
    fn a_bare_invocation_defaults_to_the_llm_on_8080() {
        assert_eq!(
            parse(Vec::<String>::new()),
            Ok(Command::Serve { port: DEFAULT_PORT, policy: ServePolicy::Llm, new_run: false }),
        );
    }

    #[test]
    fn serve_flags_parse_in_any_order() {
        let expected = Command::Serve { port: 9000, policy: ServePolicy::Random, new_run: false };
        assert_eq!(parse(["--port", "9000", "--policy", "random"]), Ok(expected.clone()));
        assert_eq!(parse(["--policy", "random", "--port", "9000"]), Ok(expected));
    }

    #[test]
    fn new_run_is_a_switch_and_not_a_setting() {
        let expected = Command::Serve { port: 9000, policy: ServePolicy::Random, new_run: true };
        assert_eq!(parse(["--new-run", "--port", "9000", "--policy", "random"]), Ok(expected.clone()));
        assert_eq!(parse(["--port", "9000", "--new-run", "--policy", "random"]), Ok(expected.clone()));
        assert_eq!(parse(["--port", "9000", "--policy", "random", "--new-run"]), Ok(expected));
    }

    #[test]
    fn help_is_not_an_error() {
        for flag in ["--help", "-h", "help"] {
            assert_eq!(parse([flag]), Ok(Command::Help), "{flag}");
        }
    }

    #[test]
    fn the_usage_names_every_flag_and_variable() {
        for name in [
            "--port", "--policy", "--new-run", "--help",
            "GB_PORT", "GB_POLICY", "GB_RUN_DIR", "GB_STATUS_HZ", "GB_HARDWARE", "GB_AUDIO_BITRATE",
            "OPENAI_API_KEY", "GB_MODEL", "OPENAI_BASE_URL",
            "GB_CONTEXT_LIMIT", "GB_TEMPERATURE", "GB_MAX_TOOL_STEPS", "GB_STUCK_TIMEOUT_SECS",
            // Every spelling `--policy` and `GB_POLICY` accept: `--help` is the only discovery.
            "llm", "random", "deterministic",
        ] {
            assert!(USAGE.contains(name), "`{name}` is accepted but `--help` does not mention it");
        }
    }

    /// The flag and the variable accept the same names, through `ServePolicy::parse`.
    #[test]
    fn every_policy_is_spelled_the_same_on_the_command_line_and_in_the_environment() {
        for (name, expected) in [
            ("llm", ServePolicy::Llm),
            ("random", ServePolicy::Random),
            ("deterministic", ServePolicy::Deterministic),
        ] {
            assert_eq!(
                parse(["--policy", name]),
                Ok(Command::Serve { port: DEFAULT_PORT, policy: expected, new_run: false }),
                "--policy {name}",
            );
            let env = |var: &str| (var == "GB_POLICY").then(|| name.to_string());
            assert_eq!(
                parse_with_env(Vec::<String>::new(), &env),
                Ok(Command::Serve { port: DEFAULT_PORT, policy: expected, new_run: false }),
                "GB_POLICY={name}",
            );
        }
    }

    /// The ConfigMap sets it; the operator overrides it.
    #[test]
    fn gb_policy_is_the_default_and_the_flag_overrides_it() {
        let env = |name: &str| (name == "GB_POLICY").then(|| "random".to_string());
        assert_eq!(
            parse_with_env(Vec::<String>::new(), &env),
            Ok(Command::Serve { port: DEFAULT_PORT, policy: ServePolicy::Random, new_run: false }),
        );
        assert_eq!(
            parse_with_env(["--policy", "llm"], &env),
            Ok(Command::Serve { port: DEFAULT_PORT, policy: ServePolicy::Llm, new_run: false }),
        );

        // Blank is what a placeholder looks like in a Deployment, and means unset.
        for blank in ["", "   "] {
            let env = |name: &str| (name == "GB_POLICY").then(|| blank.to_string());
            assert_eq!(
                parse_with_env(Vec::<String>::new(), &env),
                Ok(Command::Serve { port: DEFAULT_PORT, policy: ServePolicy::Llm, new_run: false }),
                "GB_POLICY={blank:?}",
            );
        }

        // Anything else is refused, naming the variable and the value.
        let env = |name: &str| (name == "GB_POLICY").then(|| "magic-8-ball".to_string());
        let error = parse_with_env(Vec::<String>::new(), &env).expect_err("not a policy");
        assert!(error.contains("GB_POLICY") && error.contains("magic-8-ball"), "{error}");
        assert!(error.contains(USAGE), "{error}");
    }

    /// Every rejection prints the usage under it, so a mistyped flag is self-correcting.
    #[test]
    fn bad_input_is_rejected_with_usage() {
        let rejected = [
            vec!["nonsense"],
            vec!["--port"],
            vec!["--port", "not-a-number"],
            vec!["--port", "70000"],
            vec!["--port", "0"],
            vec!["--policy", "magic-8-ball"],
            vec!["--colour", "red"],
        ];
        for args in rejected {
            let error = parse(&args).expect_err(&format!("{args:?} should not parse"));
            assert!(error.contains(USAGE), "{args:?} rejected without usage: {error}");
        }
    }

    #[test]
    fn gb_port_is_the_default_and_the_flag_overrides_it() {
        let env = |name: &str| (name == "GB_PORT").then(|| "9999".to_string());
        assert_eq!(
            parse_with_env(Vec::<String>::new(), &env),
            Ok(Command::Serve { port: 9999, policy: ServePolicy::Llm, new_run: false }),
        );
        assert_eq!(
            parse_with_env(["--port", "7000"], &env),
            Ok(Command::Serve { port: 7000, policy: ServePolicy::Llm, new_run: false }),
        );

        for bad in ["0", "port80", "70000"] {
            let env = |name: &str| (name == "GB_PORT").then(|| bad.to_string());
            let error = parse_with_env(Vec::<String>::new(), &env).expect_err("{bad} is not a port");
            assert!(error.contains("GB_PORT") && error.contains(bad), "{error}");
        }
    }
}
