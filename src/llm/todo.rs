//! **W6b / §10** — the model's TODO list: the one thing it writes that outlives the conversation.
//!
//! ```text
//! $GB_RUN_DIR/<run-id>/todo.json      [{ id, text, done }]
//! ```
//!
//! Everything else the model knows is in the message history, and the message history is
//! **destroyed on purpose** every time the context fills up (§9) and does not exist at all after a
//! restart — so "beat Brock, then take the ferry" has to live somewhere neither reaches.
//!
//! ⚠️ **This used to be two mechanisms and is deliberately one.** Beside it lived a `memories/`
//! directory: freeform notes under a name, `memory_write`/`memory_read`, indexed by first line into
//! the same prompt block. Two tools' worth of schema in every request, two places for the same
//! sentence to live, and a decision to make about which — for a role the compaction summary already
//! fills better, since it is written by the model with the whole history in front of it rather than
//! a paragraph at a time. What only a TODO list does is survive a *process* restart, and it does
//! that whether or not it is also a memory directory. So an item is allowed to be long enough to
//! carry its own reason ([`MAX_TEXT`]) — "come back to Route 12 with the Poké Flute, the Snorlax
//! blocks the only path south" is one item, not an item plus a note.
//!
//! ⚠️ **The list is not rendered into the system prompt, and that is a cost decision rather than a
//! layout one.** It used to be, which meant `todo_add` rewrote message 0 — and a prefix cache is
//! keyed on the prefix, so every edit the model made to its own plan invalidated the *entire*
//! conversation for the next request. On a hosted endpoint that is the cache discount; on a local
//! server it is re-prefilling the whole history before a single new token. It now rides in a
//! message of its own near the tail, emitted by [`crate::llm::worker`] only when it has actually
//! changed — see [`crate::llm::prompt::plan_message`].

use std::path::{Path, PathBuf};

use crate::run::files;

/// How many items the list holds. Small on purpose: it is in every request, and a plan nobody can
/// read at a glance is not a plan.
pub const MAX_ITEMS: usize = 32;
/// Long enough for the intent *and* the reason it exists, because there is no longer a note beside
/// it to hold the second half. See the module's second ⚠️.
pub const MAX_TEXT: usize = 200;
/// How many finished items are still shown to the model. The UI shows every one — a viewer reads
/// them as progress — but in a context window a done item is answered noise, and the compaction
/// summary is what carries the achievements forward.
const SHOW_DONE: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TodoItem {
    pub id: u32,
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

impl From<&TodoItem> for crate::web::published::TodoView {
    fn from(item: &TodoItem) -> Self {
        Self { id: item.id, text: item.text.clone(), done: item.done }
    }
}

/// One tool call against the list, parsed. Answered on the **worker thread** — none of this needs
/// the emulator, so unlike a read it costs no round trip through `service_tools`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoCall {
    Add { text: String },
    Complete { id: u32 },
}

pub struct TodoList {
    /// `None` for a run with no directory — the tests, and any future caller that wants the tools
    /// without the persistence. Everything still works; nothing survives the process.
    path: Option<PathBuf>,
    items: Vec<TodoItem>,
    next_id: u32,
}

impl TodoList {
    /// Open the list in a run directory.
    ///
    /// Never fails: an unreadable `todo.json` starts empty. Losing the plan is bad; refusing to
    /// play because of it is worse.
    pub fn open(run_dir: Option<&Path>) -> Self {
        let Some(run_dir) = run_dir else {
            return Self { path: None, items: Vec::new(), next_id: 1 };
        };
        let path = run_dir.join(files::TODO);
        let items: Vec<TodoItem> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let next_id = items.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        Self { path: Some(path), items, next_id }
    }

    /// Service one call, returning the sentence the model is shown as the tool result.
    pub fn apply(&mut self, call: TodoCall) -> String {
        match call {
            TodoCall::Add { text } => self.add(&text),
            TodoCall::Complete { id } => self.complete(id),
        }
    }

    /// Every item, for the UI. The model's copy is [`Self::render`], which is shorter.
    pub fn items(&self) -> &[TodoItem] {
        &self.items
    }

    fn add(&mut self, text: &str) -> String {
        let text = truncated(text.trim(), MAX_TEXT);
        if text.is_empty() {
            return "An empty TODO is not a TODO.".to_string();
        }
        // Dropping *done* items first is what stops a long run hitting the cap because of things it
        // finished an hour ago.
        if self.items.len() >= MAX_ITEMS {
            if let Some(position) = self.items.iter().position(|item| item.done) {
                self.items.remove(position);
            } else {
                return format!(
                    "Your TODO list is full ({MAX_ITEMS} items) and none of them are done. Finish \
                     something with `todo_complete` first."
                );
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(TodoItem { id, text: text.clone(), done: false });
        self.persist();
        format!("Added TODO {id}: {text}")
    }

    fn complete(&mut self, id: u32) -> String {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return format!("There is no TODO {id}. The list is in the turn you were just sent.");
        };
        if item.done {
            return format!("TODO {id} was already done.");
        }
        item.done = true;
        let text = item.text.clone();
        self.persist();
        format!("Done: {text}")
    }

    /// The block that goes into a turn as a message of its own (§10). Open items in full — that is
    /// the whole point of it — and only the last [`SHOW_DONE`] finished ones, so a long run's
    /// completed work does not become the context problem the list exists to solve.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str(PLAN_HEADING);
        out.push_str(
            "\n\nThis is the only thing you write that survives a context compaction and a restart \
             of the program. Keep it current: add what you mean to do next and why, and complete \
             items as you finish them.\n\n",
        );

        let (done, open): (Vec<&TodoItem>, Vec<&TodoItem>) =
            self.items.iter().partition(|item| item.done);
        if open.is_empty() && done.is_empty() {
            out.push_str("(empty — `todo_add` is how a plan outlives this conversation.)\n");
            return out;
        }
        if open.is_empty() {
            out.push_str("(nothing outstanding.)\n");
        }
        for item in &open {
            out.push_str(&format!("- [ ] {} — {}\n", item.id, item.text));
        }
        // Oldest-first among the ones shown, so the tail still reads in the order it happened.
        let hidden = done.len().saturating_sub(SHOW_DONE);
        if hidden > 0 {
            out.push_str(&format!("\n({hidden} earlier items done and not listed)\n"));
        }
        for item in done.iter().skip(hidden) {
            out.push_str(&format!("- [x] {} — {}\n", item.id, item.text));
        }
        out
    }

    fn persist(&self) {
        let Some(path) = self.path.as_ref() else { return };
        let Ok(json) = serde_json::to_vec_pretty(&self.items) else { return };
        if let Err(failure) = crate::run::write_atomically(path, &json) {
            eprintln!("todo: {failure}");
        }
    }
}

/// What [`TodoList::render`] opens with, and therefore how the worker finds the copy already in the
/// history in order to replace it. ⚠️ It has to be unique among the things a `user` message can
/// start with — [`crate::llm::compaction::SUMMARY_HEADING`] is the other one.
pub const PLAN_HEADING: &str = "## Your plan";

/// Truncate on a character boundary, because [`MAX_TEXT`] lands in the middle of a multi-byte
/// character the first time a model writes about a Pokémon with an accent in its name.
fn truncated(text: &str, limit: usize) -> String {
    match text.len() <= limit {
        true => text.to_string(),
        false => text
            .chars()
            .scan(0, |used, c| {
                *used += c.len_utf8();
                (*used <= limit).then_some(c)
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::Scratch;

    /// The whole of W6b's point: what the model writes is in the *next* turn, and still there after
    /// the process has gone away and come back.
    #[test]
    fn the_plan_survives_the_process_that_wrote_it() {
        let scratch = Scratch::new("todo");
        {
            let mut todo = TodoList::open(Some(&scratch.0));
            todo.apply(TodoCall::Add { text: "beat Brock".into() });
            todo.apply(TodoCall::Add { text: "buy potions".into() });
            assert!(todo.apply(TodoCall::Complete { id: 1 }).contains("beat Brock"));
        }

        let mut todo = TodoList::open(Some(&scratch.0));
        let rendered = todo.render();
        assert!(rendered.starts_with(PLAN_HEADING), "the worker finds the old copy by this: {rendered}");
        assert!(rendered.contains("- [x] 1 — beat Brock"), "{rendered}");
        assert!(rendered.contains("- [ ] 2 — buy potions"), "{rendered}");

        // A new id after a restart, rather than one that collides with what is already there.
        assert!(todo.apply(TodoCall::Add { text: "reach Cerulean".into() }).contains("TODO 3"));
    }

    /// The caps are what stop the plan becoming the context problem it exists to solve — and the
    /// tail of finished work is the part that grows without bound in a long run.
    #[test]
    fn the_caps_hold_and_say_why() {
        let mut todo = TodoList::open(None);
        for n in 0..MAX_ITEMS {
            todo.apply(TodoCall::Add { text: format!("thing {n}") });
        }
        let full = todo.apply(TodoCall::Add { text: "one more".into() });
        assert!(full.contains("full"), "{full}");

        // Completing one makes room, and it is the completed one that is dropped.
        todo.apply(TodoCall::Complete { id: 1 });
        assert!(todo.apply(TodoCall::Add { text: "one more".into() }).starts_with("Added"));
        assert!(!todo.render().contains("thing 0"), "the finished item made way");

        assert!(todo.apply(TodoCall::Add { text: "  ".into() }).contains("not a TODO"));
        assert!(todo.apply(TodoCall::Complete { id: 9999 }).contains("no TODO 9999"));

        // Two bytes a character, so the cap bites at half the characters — and lands *on* a
        // boundary rather than splitting one, which is the thing worth asserting.
        let mut todo = TodoList::open(None);
        todo.apply(TodoCall::Add { text: "é".repeat(MAX_TEXT) });
        assert_eq!(todo.items()[0].text.chars().count(), MAX_TEXT / 2);
    }

    /// ⚠️ The model's copy is not the UI's. A run that finishes fifty things must not carry fifty
    /// answered lines in every request for the rest of its life.
    #[test]
    fn finished_work_leaves_the_prompt_but_not_the_list() {
        let mut todo = TodoList::open(None);
        for n in 0..10 {
            todo.apply(TodoCall::Add { text: format!("thing {n}") });
            todo.apply(TodoCall::Complete { id: n + 1 });
        }
        todo.apply(TodoCall::Add { text: "the one thing left".into() });

        let rendered = todo.render();
        assert_eq!(rendered.matches("- [x]").count(), SHOW_DONE, "{rendered}");
        assert!(rendered.contains("(7 earlier items done and not listed)"), "{rendered}");
        assert!(rendered.contains("- [ ] 11 — the one thing left"), "{rendered}");
        assert!(rendered.contains("thing 9"), "the most recent finished work is still shown");
        assert!(!rendered.contains("thing 0"), "the oldest is not: {rendered}");

        assert_eq!(todo.items().len(), 11, "the UI still gets every one of them");
    }

    /// With no run directory the tools still work — they simply keep nothing. That is what the
    /// worker's tests run against, and it must not be a special case in the calling code.
    #[test]
    fn a_list_without_a_directory_still_answers() {
        let mut todo = TodoList::open(None);
        assert!(todo.render().contains("(empty"));
        assert!(todo.apply(TodoCall::Add { text: "go north".into() }).starts_with("Added"));
        assert!(todo.render().contains("- [ ] 1 — go north"));
    }
}
