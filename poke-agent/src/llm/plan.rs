//! The model's plan: free text, and the one thing it writes that outlives the conversation.
//! ```text
//! $GB_RUN_DIR/<run-id>/plan-history.jsonl      every version of `plan.md`; the plan is the last
//! ```

use std::path::Path;

use crate::run::files;
use crate::run::versions::{self, Version, Versions};

/// 10 KB: room for a long plan, small enough that reading it every turn is cheap.
pub const MAX_PLAN: usize = 10_000;

pub struct Plan {
    versions: Versions,
}

impl Plan {
    /// Never fails: an unreadable history is an empty plan. A run from before the history has its
    /// `plan.md`, or before that its `todo.json`, made the first version, once.
    pub fn open(run_dir: Option<&Path>) -> Self {
        if let Some(run_dir) = run_dir {
            migrate(run_dir);
        }
        Self { versions: Versions::open(run_dir.map(|dir| dir.join(files::PLAN_HISTORY))) }
    }

    /// `POST /api/clear`: an empty version, so the history keeps what was cleared.
    pub fn cleared(run_dir: Option<&Path>) -> Self {
        let mut plan = Self::open(run_dir);
        plan.versions.record("", None);
        plan
    }

    pub fn text(&self) -> &str {
        self.versions.text()
    }

    /// Which version [`Self::text`] is; 0 before the first.
    pub fn version(&self) -> u64 {
        self.versions.version()
    }

    /// Replace the whole plan, or say why not with nothing changed.
    pub fn write(&mut self, text: &str, turn: Option<u64>) -> Result<(), String> {
        if text.len() > MAX_PLAN {
            return Err(format!(
                "Not written, and nothing was changed: that is {} bytes and plan.md holds at most \
                 {MAX_PLAN}. Delete what is done and say the rest more briefly.",
                text.len(),
            ));
        }
        self.versions.record(text, turn);
        Ok(())
    }
}

/// An older run's plan file becomes the first version, and goes once the history holds it.
fn migrate(run_dir: &Path) {
    let history = run_dir.join(files::PLAN_HISTORY);
    let plan = run_dir.join(files::LEGACY_PLAN);
    let todo = run_dir.join(files::LEGACY_TODO);
    if !history.exists() {
        let legacy = std::fs::read_to_string(&plan)
            .ok()
            .map(|text| (text, &plan))
            .or_else(|| std::fs::read(&todo).ok().map(|bytes| (render_legacy(&bytes), &todo)));
        if let Some((text, from)) = legacy.filter(|(text, _)| !text.is_empty()) {
            let first = Version { version: 1, at: versions::modified_ms(from), turn: None, text };
            if let Err(failure) = versions::append(&history, &first) {
                eprintln!("plan: {failure}");
                return;
            }
        }
    }
    versions::remove_legacy(&plan);
    versions::remove_legacy(&todo);
}

/// `todo.json`'s `[{ id, text, done }]` as numbered lines, finished ones marked.
fn render_legacy(bytes: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct Item {
        text: String,
        #[serde(default)]
        done: bool,
    }
    let items: Vec<Item> = serde_json::from_slice(bytes).unwrap_or_default();
    items
        .iter()
        .enumerate()
        .map(|(index, item)| match item.done {
            true => format!("{}. (done) {}\n", index + 1, item.text),
            false => format!("{}. {}\n", index + 1, item.text),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Scratch;

    fn lines(dir: &Path) -> Vec<Version> {
        versions::page(&dir.join(files::PLAN_HISTORY), usize::MAX, None)
    }

    #[test]
    fn the_plan_survives_the_process_that_wrote_it() {
        let scratch = Scratch::new("plan");
        Plan::open(Some(&scratch.0)).write("1. beat Brock\n2. buy potions\n", Some(7)).expect("fits");
        let plan = Plan::open(Some(&scratch.0));
        assert_eq!(plan.text(), "1. beat Brock\n2. buy potions\n");
        assert_eq!(plan.version(), 1);
        assert_eq!(lines(&scratch.0)[0].turn, Some(7), "the line says which turn wrote it");
        assert!(!scratch.0.join(files::LEGACY_PLAN).exists(), "no current-version file");
    }

    #[test]
    fn every_version_is_kept_and_a_rewrite_of_the_same_text_is_not_one() {
        let scratch = Scratch::new("plan-versions");
        let mut plan = Plan::open(Some(&scratch.0));
        plan.write("1. beat Brock", Some(1)).expect("fits");
        plan.write("1. beat Brock", Some(2)).expect("fits");
        plan.write("1. beat Misty", Some(3)).expect("fits");
        let texts: Vec<String> = lines(&scratch.0).into_iter().map(|v| v.text).collect();
        assert_eq!(texts, ["1. beat Brock", "1. beat Misty"]);
    }

    #[test]
    fn a_plan_over_the_cap_is_refused_and_changes_nothing() {
        let mut plan = Plan::open(None);
        plan.write("keep this", None).expect("fits");
        let refusal = plan.write(&"x".repeat(MAX_PLAN + 1), None).expect_err("over the cap");
        assert!(refusal.contains("nothing was changed"), "{refusal}");
        assert_eq!(plan.text(), "keep this");
    }

    #[test]
    fn an_older_runs_todo_list_is_rendered_into_the_first_version_once() {
        let scratch = Scratch::new("plan-legacy");
        let legacy = r#"[{"id":3,"text":"beat Brock","done":true},{"id":7,"text":"reach Cerulean"}]"#;
        std::fs::write(scratch.0.join(files::LEGACY_TODO), legacy).expect("a todo list");

        let mut plan = Plan::open(Some(&scratch.0));
        assert_eq!(plan.text(), "1. (done) beat Brock\n2. reach Cerulean\n");
        assert!(!scratch.0.join(files::LEGACY_TODO).exists(), "rendered once, so it goes");
        assert_eq!(lines(&scratch.0).len(), 1);

        plan.write("", None).expect("fits");
        assert_eq!(Plan::open(Some(&scratch.0)).text(), "", "an emptied plan stays empty");
    }

    /// The file a run kept before the history becomes its first line, dated when it was written.
    #[test]
    fn an_older_runs_plan_file_becomes_the_first_version_once() {
        let scratch = Scratch::new("plan-legacy-md");
        let legacy = scratch.0.join(files::LEGACY_PLAN);
        std::fs::write(&legacy, "1. beat Brock\n").expect("a plan");
        // Both, as a run that crashed between rendering its todo list and deleting it left them.
        std::fs::write(scratch.0.join(files::LEGACY_TODO), "[]").expect("a todo list");
        let written = versions::modified_ms(&legacy);

        let plan = Plan::open(Some(&scratch.0));
        assert_eq!(plan.text(), "1. beat Brock\n");
        assert!(!legacy.exists() && !scratch.0.join(files::LEGACY_TODO).exists(), "both go");
        let first = lines(&scratch.0);
        assert_eq!(first, [Version { version: 1, at: written, turn: None, text: "1. beat Brock\n".into() }]);

        // Once: reopening appends nothing, and a stray old file does not overwrite the history.
        std::fs::write(&legacy, "stale").expect("a stray plan");
        assert_eq!(Plan::open(Some(&scratch.0)).text(), "1. beat Brock\n");
        assert_eq!(lines(&scratch.0).len(), 1);
        assert!(!legacy.exists());
    }

    #[test]
    fn a_cleared_plan_is_an_empty_version_and_the_earlier_ones_stay() {
        let scratch = Scratch::new("plan-cleared");
        Plan::open(Some(&scratch.0)).write("beat Brock", Some(4)).expect("fits");
        let plan = Plan::cleared(Some(&scratch.0));
        assert_eq!((plan.text(), plan.version()), ("", 2));
        assert_eq!(Plan::open(Some(&scratch.0)).text(), "");
        let texts: Vec<(String, Option<u64>)> = lines(&scratch.0).into_iter().map(|v| (v.text, v.turn)).collect();
        assert_eq!(texts, [("beat Brock".to_string(), Some(4)), (String::new(), None)]);

        // Clearing an empty plan has nothing to record.
        Plan::cleared(Some(&scratch.0));
        assert_eq!(lines(&scratch.0).len(), 2);
    }
}
