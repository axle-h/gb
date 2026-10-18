//! The model's plan: free text, and the one thing it writes that outlives the conversation.
//! ```text
//! $GB_RUN_DIR/<run-id>/plan.md      what the model last wrote to `plan.md`, verbatim
//! ```

use std::path::{Path, PathBuf};

use crate::run::files;

/// 10 KB: room for a long plan, small enough that reading it every turn is cheap.
pub const MAX_PLAN: usize = 10_000;

pub struct Plan {
    /// `None` for a run with no directory, which persists nothing.
    path: Option<PathBuf>,
    text: String,
}

impl Plan {
    /// Never fails: an unreadable file is an empty plan. A run from before `plan.md` existed has
    /// its `todo.json` rendered into one, once.
    pub fn open(run_dir: Option<&Path>) -> Self {
        let Some(run_dir) = run_dir else {
            return Self { path: None, text: String::new() };
        };
        let path = run_dir.join(files::PLAN);
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Self { path: Some(path), text };
        }
        let legacy = run_dir.join(files::LEGACY_TODO);
        let text = std::fs::read(&legacy).ok().map(|bytes| render_legacy(&bytes)).unwrap_or_default();
        let plan = Self { path: Some(path), text };
        if legacy.exists() && plan.persist() {
            let _ = std::fs::remove_file(&legacy);
        }
        plan
    }

    /// `POST /api/clear`: an empty plan, and the file deleted with it.
    pub fn cleared(run_dir: Option<&Path>) -> Self {
        let path = run_dir.map(|dir| dir.join(files::PLAN));
        for doomed in [path.clone(), run_dir.map(|dir| dir.join(files::LEGACY_TODO))].into_iter().flatten() {
            match std::fs::remove_file(&doomed) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => eprintln!("could not delete {}: {e}", doomed.display()),
            }
        }
        Self { path, text: String::new() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replace the whole plan, or say why not with nothing changed.
    pub fn write(&mut self, text: &str) -> Result<(), String> {
        if text.len() > MAX_PLAN {
            return Err(format!(
                "Not written, and nothing was changed: that is {} bytes and plan.md holds at most \
                 {MAX_PLAN}. Delete what is done and say the rest more briefly.",
                text.len(),
            ));
        }
        self.text = text.to_string();
        self.persist();
        Ok(())
    }

    fn persist(&self) -> bool {
        let Some(path) = self.path.as_ref() else { return false };
        match crate::run::write_atomically(path, self.text.as_bytes()) {
            Ok(()) => true,
            Err(failure) => {
                eprintln!("plan: {failure}");
                false
            }
        }
    }
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

    #[test]
    fn the_plan_survives_the_process_that_wrote_it() {
        let scratch = Scratch::new("plan");
        Plan::open(Some(&scratch.0)).write("1. beat Brock\n2. buy potions\n").expect("fits");
        let plan = Plan::open(Some(&scratch.0));
        assert_eq!(plan.text(), "1. beat Brock\n2. buy potions\n");
        assert!(scratch.0.join(files::PLAN).is_file());
    }

    #[test]
    fn a_plan_over_the_cap_is_refused_and_changes_nothing() {
        let mut plan = Plan::open(None);
        plan.write("keep this").expect("fits");
        let refusal = plan.write(&"x".repeat(MAX_PLAN + 1)).expect_err("over the cap");
        assert!(refusal.contains("nothing was changed"), "{refusal}");
        assert_eq!(plan.text(), "keep this");
    }

    #[test]
    fn an_older_runs_todo_list_is_rendered_into_the_plan_once() {
        let scratch = Scratch::new("plan-legacy");
        let legacy = r#"[{"id":3,"text":"beat Brock","done":true},{"id":7,"text":"reach Cerulean"}]"#;
        std::fs::write(scratch.0.join(files::LEGACY_TODO), legacy).expect("a todo list");

        let mut plan = Plan::open(Some(&scratch.0));
        assert_eq!(plan.text(), "1. (done) beat Brock\n2. reach Cerulean\n");
        assert!(!scratch.0.join(files::LEGACY_TODO).exists(), "rendered once, so it goes");

        plan.write("").expect("fits");
        assert_eq!(Plan::open(Some(&scratch.0)).text(), "", "an emptied plan stays empty");
    }

    #[test]
    fn a_cleared_plan_takes_the_file_with_it() {
        let scratch = Scratch::new("plan-cleared");
        Plan::open(Some(&scratch.0)).write("beat Brock").expect("fits");
        std::fs::write(scratch.0.join(files::LEGACY_TODO), "[]").expect("a todo list");
        let plan = Plan::cleared(Some(&scratch.0));
        assert_eq!(plan.text(), "");
        assert!(!scratch.0.join(files::PLAN).exists());
        assert!(!scratch.0.join(files::LEGACY_TODO).exists());
        assert_eq!(Plan::open(Some(&scratch.0)).text(), "");
    }
}
