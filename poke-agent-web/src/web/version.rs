//! `GET /version`: which build is answering, from what CI injects into the environment.

use axum::response::Json;

/// What build this is, as `/version` returns it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BuildInfo {
    /// `Cargo.toml`'s version — bumped by hand, see [`crate::cli::VERSION`].
    pub version: &'static str,
    /// When the image was built, as CI wrote it: RFC 3339, UTC.
    pub build_date: Option<String>,
    pub branch: Option<String>,
    /// The short commit hash.
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

    /// One line for the startup banner, so the log answers what `/version` does.
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
fn injected(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

pub async fn version() -> Json<BuildInfo> {
    Json(BuildInfo::current())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// An unknown fact is left out, parenthesis and all.
    #[test]
    fn a_local_build_says_only_what_it_knows() {
        assert_eq!(info(None, None, None).summary(), "1.0.0");
        assert_eq!(info(None, Some("feature/x"), None).summary(), "1.0.0 (feature/x)");
        assert_eq!(info(None, None, Some("a1b2c3d")).summary(), "1.0.0 (a1b2c3d)");
    }

    /// The field names are the wire contract, and a rename breaks it with no compile error.
    #[test]
    fn the_json_names_the_four_fields_and_nulls_what_it_does_not_know() {
        let json = serde_json::to_value(info(Some("2026-08-12T14:22:33Z"), Some("main"), None)).unwrap();
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["build_date"], "2026-08-12T14:22:33Z");
        assert_eq!(json["branch"], "main");
        assert!(json["commit"].is_null(), "an absent fact is null, not a missing key: {json}");
    }
}
