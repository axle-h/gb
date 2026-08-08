//! Command-line parsing — hand-rolled over `std::env::args`, no `clap`.
//!
//! The surface is three commands' worth of flags and is not expected to grow much; a dependency
//! that pulls in a derive macro and a builder API to parse `--port 8080` would cost more than it
//! saves. What it does buy, being a module rather than a few lines in `main`, is [`parse`] being
//! testable without spawning a process.
//!
//! ```text
//! gb                        SDL UI, ConsolePolicy on stdin   (requires the `sdl` feature)
//! gb serve [--port 8080]    web UI + LlmPolicy
//! gb serve --policy random  web UI, RandomPolicy — no API key, the video-pipeline harness
//! ```

pub const USAGE: &str = "\
gb — Game Boy emulator and Pokémon Red LLM agent

USAGE:
    gb                          Run the SDL2 desktop UI, driven from stdin (requires `sdl`)
    gb serve [OPTIONS]          Serve the web UI and play under the chosen policy
    gb --help                   Print this message

SERVE OPTIONS:
    --port <PORT>               Port to listen on [default: $GB_PORT, else 8080]
    --policy <llm|random>       What plays the game [default: llm]

ENVIRONMENT (--policy llm):
    OPENAI_API_KEY, GB_MODEL    Required
    OPENAI_BASE_URL             Any OpenAI-compatible endpoint [default: api.openai.com/v1]
    GB_CONTEXT_LIMIT, GB_TEMPERATURE, GB_MAX_TOOL_STEPS
";

/// What the process was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Today's behaviour, and what a bare `gb` still means: the SDL2 window driven by
    /// `ConsolePolicy` on stdin.
    Ui,
    /// The web UI, served from this process.
    Serve { port: u16, policy: ServePolicy },
    /// `--help`. Separate from a parse error because it exits zero and prints to stdout.
    Help,
}

/// Who makes the decisions under `gb serve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServePolicy {
    /// An LLM over an OpenAI-compatible API. Needs an API key in the environment.
    Llm,
    /// Random legal choices. Needs nothing, which is what makes it the harness for exercising the
    /// video pipeline and the web UI without spending tokens.
    Random,
}

pub const DEFAULT_PORT: u16 = 8080;

/// Parse already-split arguments, **excluding** the program name.
///
/// `Err` carries a complete message ready to print — the specific complaint followed by [`USAGE`] —
/// so callers never have to compose one.
pub fn parse<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parse_with_env(args, &|name| std::env::var(name).ok())
}

/// [`parse`] against an explicit environment.
///
/// The environment is process-global, which would make every test here order-dependent against
/// whatever `GB_PORT` the shell happened to export — so the tests pass their own, and `parse`
/// supplies the real one.
pub fn parse_with_env<I, S>(args: I, env: &dyn Fn(&str) -> Option<String>) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
    let fail = |msg: String| Err(format!("{msg}\n\n{USAGE}"));

    let mut rest = args.iter().map(String::as_str);
    let Some(first) = rest.next() else { return Ok(Command::Ui) };
    match first {
        "--help" | "-h" | "help" => return Ok(Command::Help),
        "serve" => {}
        other => return fail(format!("unknown command `{other}`")),
    }

    // `GB_PORT` is the container's way of setting this (§7.1); `--port` is the operator's, and wins.
    let mut port = match env("GB_PORT").map(|value| value.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(value) => match value.parse::<u16>() {
            Ok(0) | Err(_) => return fail(format!("`GB_PORT={value}` is not a port number")),
            Ok(parsed) => parsed,
        },
        None => DEFAULT_PORT,
    };
    let mut policy = ServePolicy::Llm;
    while let Some(flag) = rest.next() {
        // Every flag takes a value, so a missing one is always the same mistake and reports the same
        // way rather than silently defaulting.
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
            "--policy" => match value {
                "llm" => policy = ServePolicy::Llm,
                "random" => policy = ServePolicy::Random,
                other => return fail(format!("`--policy {other}` — expected `llm` or `random`")),
            },
            other => return fail(format!("unknown option `{other}`")),
        }
    }
    Ok(Command::Serve { port, policy })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests must not see whatever the shell exported.
    fn parse<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> Result<Command, String> {
        parse_with_env(args, &|_| None)
    }

    #[test]
    fn bare_invocation_is_the_desktop_ui() {
        assert_eq!(parse(Vec::<String>::new()), Ok(Command::Ui));
    }

    #[test]
    fn serve_defaults_to_the_llm_on_8080() {
        assert_eq!(parse(["serve"]), Ok(Command::Serve { port: DEFAULT_PORT, policy: ServePolicy::Llm }));
    }

    #[test]
    fn serve_flags_parse_in_any_order() {
        let expected = Command::Serve { port: 9000, policy: ServePolicy::Random };
        assert_eq!(parse(["serve", "--port", "9000", "--policy", "random"]), Ok(expected.clone()));
        assert_eq!(parse(["serve", "--policy", "random", "--port", "9000"]), Ok(expected));
    }

    #[test]
    fn help_is_not_an_error() {
        for flag in ["--help", "-h", "help"] {
            assert_eq!(parse([flag]), Ok(Command::Help), "{flag}");
        }
    }

    /// Every rejection prints the usage under it, so a mistyped flag is self-correcting.
    #[test]
    fn bad_input_is_rejected_with_usage() {
        let rejected = [
            vec!["nonsense"],
            vec!["serve", "--port"],
            vec!["serve", "--port", "not-a-number"],
            vec!["serve", "--port", "70000"],
            vec!["serve", "--port", "0"],
            vec!["serve", "--policy", "magic-8-ball"],
            vec!["serve", "--colour", "red"],
        ];
        for args in rejected {
            let error = parse(&args).expect_err(&format!("{args:?} should not parse"));
            assert!(error.contains(USAGE), "{args:?} rejected without usage: {error}");
        }
    }

    /// **W4 / §7.1.** The container sets the port through the environment; a person debugging sets it
    /// on the command line, and theirs wins.
    #[test]
    fn gb_port_is_the_default_and_the_flag_overrides_it() {
        let env = |name: &str| (name == "GB_PORT").then(|| "9999".to_string());
        assert_eq!(
            parse_with_env(["serve"], &env),
            Ok(Command::Serve { port: 9999, policy: ServePolicy::Llm }),
        );
        assert_eq!(
            parse_with_env(["serve", "--port", "7000"], &env),
            Ok(Command::Serve { port: 7000, policy: ServePolicy::Llm }),
        );

        // A nonsense `GB_PORT` is reported rather than silently ignored: a container that quietly
        // binds 8080 when it was told 80 is a much longer afternoon than one that will not start.
        for bad in ["0", "port80", "70000"] {
            let env = |name: &str| (name == "GB_PORT").then(|| bad.to_string());
            let error = parse_with_env(["serve"], &env).expect_err("{bad} is not a port");
            assert!(error.contains("GB_PORT") && error.contains(bad), "{error}");
        }
    }
}
