//! `GET /version` — which build is answering.
//!
//! Four facts: the crate version, when the image was built, and the branch and short commit it was
//! built from. Only the first is knowable from inside the source tree ([`crate::cli::VERSION`], from
//! `CARGO_PKG_VERSION`); the other three are **injected by whatever built the image** and read from
//! the environment at startup — `GB_BUILD_DATE`, `GB_GIT_BRANCH`, `GB_GIT_SHA`, set as `ENV` in the
//! Dockerfile's runtime stage from build args CI fills in.
//!
//! ⚠️ **Runtime environment rather than `env!()` at compile time, and the reason is the build
//! cache.** Baking the three into the binary means the cargo layer's inputs include a timestamp that
//! changes on every run, so *every* CI build would pay a full cold `cargo build --release` —
//! BuildKit's `type=gha` cache holds layers but not the cache mounts the cargo registry and target
//! directory live on (`.github/workflows/container.yml` says so), so an invalidated stage 3 is the
//! whole crate from scratch. Setting them as `ENV` **after** the `COPY` of the binary costs nothing:
//! the only layers below it are metadata.
//!
//! A local `cargo run` sets none of them and reports them as `null` — which is the honest answer,
//! and the one thing a git fallback in `build.rs` could not give without either recompiling the
//! crate on every commit or going quietly stale between them. To fill them in by hand:
//!
//! ```shell
//! GB_GIT_SHA=$(git rev-parse --short HEAD) GB_GIT_BRANCH=$(git branch --show-current) \
//!   cargo run --release -- serve --policy random
//! ```
//!
//! ⚠️ **Deliberately not on the page.** The SPA is what a viewer watches the game on; which build is
//! serving it is an operator's question, and the answer is one `curl` away.

use axum::response::Json;

/// What build this is, as `/version` returns it.
///
/// Read once per request rather than cached in [`super::AppState`]: it is three `getenv`s, and a
/// value that cannot change under a live process is not worth a field that every other endpoint
/// carries around.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BuildInfo {
    /// `Cargo.toml`'s version — bumped by hand, see [`crate::cli::VERSION`].
    pub version: &'static str,
    /// When the image was built, as CI wrote it: RFC 3339, UTC. `None` outside a built image.
    pub build_date: Option<String>,
    /// The branch it was built from — `main` for anything CI has published.
    pub branch: Option<String>,
    /// The short commit hash. Long enough to paste at `git show`, and the prefix of the full SHA the
    /// image is also tagged with.
    pub commit: Option<String>,
}

impl BuildInfo {
    pub fn current() -> Self {
        Self {
            version: crate::cli::VERSION,
            build_date: injected("GB_BUILD_DATE"),
            branch: injected("GB_GIT_BRANCH"),
            commit: injected("GB_GIT_SHA"),
        }
    }

    /// One line for the startup banner, so `docker logs` answers the same question `/version` does.
    ///
    /// Everything after the crate version is dropped when it is absent, rather than printed as
    /// `unknown`: a bare `1.0.0` reads as a build from a working tree, which is what it is.
    pub fn summary(&self) -> String {
        let built = match (&self.branch, &self.commit) {
            (Some(branch), Some(commit)) => format!(" ({branch} {commit})"),
            (Some(branch), None) => format!(" ({branch})"),
            (None, Some(commit)) => format!(" ({commit})"),
            (None, None) => String::new(),
        };
        let date = self.build_date.as_ref().map(|date| format!(" built {date}")).unwrap_or_default();
        format!("{}{built}{date}", self.version)
    }
}

/// A build fact, or `None` if nothing set it.
///
/// Blank counts as unset for the reason it does in [`super::admin_token`]: an unsubstituted
/// placeholder in a manifest arrives as an empty string, and `"branch": ""` is a worse answer than
/// `null`.
fn injected(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

pub async fn version() -> Json<BuildInfo> {
    Json(BuildInfo::current())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The environment is process-global and the suite is threaded, so the parts that read it are
    /// not what these test — `injected` is trivial and [`BuildInfo::summary`] is where the shape
    /// lives.
    fn info(build_date: Option<&str>, branch: Option<&str>, commit: Option<&str>) -> BuildInfo {
        BuildInfo {
            version: "1.0.0",
            build_date: build_date.map(str::to_string),
            branch: branch.map(str::to_string),
            commit: commit.map(str::to_string),
        }
    }

    #[test]
    fn a_built_image_says_what_it_was_built_from() {
        let built = info(Some("2026-08-12T14:22:33Z"), Some("main"), Some("a1b2c3d"));
        assert_eq!(built.summary(), "1.0.0 (main a1b2c3d) built 2026-08-12T14:22:33Z");
    }

    /// ⚠️ A local build must not print `1.0.0 (unknown unknown) built unknown` — the absence *is* the
    /// information, and the parenthesis has to disappear with its contents.
    #[test]
    fn a_local_build_says_only_what_it_knows() {
        assert_eq!(info(None, None, None).summary(), "1.0.0");
        assert_eq!(info(None, Some("feature/x"), None).summary(), "1.0.0 (feature/x)");
        assert_eq!(info(None, None, Some("a1b2c3d")).summary(), "1.0.0 (a1b2c3d)");
    }

    /// The field names are the wire contract — `/version` is read by a person at a terminal and by
    /// whatever they pipe it into, and a rename is a break with no compile error behind it.
    #[test]
    fn the_json_names_the_four_fields_and_nulls_what_it_does_not_know() {
        let json = serde_json::to_value(info(Some("2026-08-12T14:22:33Z"), Some("main"), None)).unwrap();
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["build_date"], "2026-08-12T14:22:33Z");
        assert_eq!(json["branch"], "main");
        assert!(json["commit"].is_null(), "an absent fact is null, not a missing key: {json}");
    }
}
