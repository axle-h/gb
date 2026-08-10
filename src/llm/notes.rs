//! **W6b / §10** — the model's own notes: a memory directory and a TODO list.
//!
//! ```text
//! $GB_RUN_DIR/<run-id>/
//!     memories/<slug>.md      one note per file: a name in frontmatter, then freeform prose
//!     todo.json               [{ id, text, done }]
//! ```
//!
//! This is how a run keeps long-horizon intent. Everything else the model knows is in the message
//! history, and the message history is **destroyed on purpose** every time the context fills up
//! (§9) — so "beat Brock, then take the ferry" has to live somewhere a compaction cannot reach.
//!
//! Two design points do most of the work:
//!
//! 1. **The index and the whole TODO list are re-rendered into the system prompt every turn**
//!    ([`Notes::render`]). The model never has to spend a tool call finding out what it knows, and
//!    because the system prompt is index 0 of the history, a compaction cannot drop it.
//! 2. **Bodies live in memory as well as on disk.** 64 notes of 8 KB is half a megabyte; keeping
//!    them means `memory_read` never touches the filesystem from the worker thread, and it is what
//!    lets the whole feature work with no run directory at all — which is what the tests use.
//!
//! ⚠️ **A name from a model is not a filename.** `memory_write { name: "../../etc/passwd" }` is one
//! tool call away at all times; [`slugify`] is the only thing between it and the filesystem, so it
//! reduces to `[a-z0-9-]` and nothing else, ever.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::run::files;

/// Caps, all of them arbitrary and all of them necessary — a model that writes a memory every turn
/// would otherwise turn the system prompt into the context problem it exists to solve.
pub const MAX_MEMORIES: usize = 64;
pub const MAX_BODY: usize = 8 * 1024;
pub const MAX_TODO: usize = 64;
pub const MAX_TODO_TEXT: usize = 200;
/// Long enough to be a sentence fragment, short enough to be a filename.
pub const MAX_SLUG: usize = 48;
/// How much of a note is shown in the index. One line, because the index is in *every* request.
const INDEX_SUMMARY: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TodoItem {
    pub id: u32,
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

/// One tool call against the notes, parsed. Answered on the **worker thread** — none of this needs
/// the emulator, so unlike a read it costs no round trip through `service_tools`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteCall {
    Write { name: String, body: String },
    Read { name: String },
    TodoAdd { text: String },
    TodoComplete { id: u32 },
}

pub struct Notes {
    /// `None` for a run with no directory — the tests, and any future caller that wants the tools
    /// without the persistence. Everything still works; nothing survives the process.
    dir: Option<PathBuf>,
    todo_path: Option<PathBuf>,
    bodies: BTreeMap<String, String>,
    todo: Vec<TodoItem>,
    next_id: u32,
}

impl Notes {
    /// Open the notes in a run directory, creating `memories/` if it is not there.
    ///
    /// Never fails: an unreadable note is skipped, an unreadable `todo.json` starts empty. Losing a
    /// note is bad; refusing to play because of one is worse.
    pub fn open(run_dir: Option<&Path>) -> Self {
        let Some(run_dir) = run_dir else {
            return Self { dir: None, todo_path: None, bodies: BTreeMap::new(), todo: Vec::new(), next_id: 1 };
        };
        let dir = run_dir.join(files::MEMORIES);
        if let Err(failure) = std::fs::create_dir_all(&dir) {
            eprintln!("notes: {} is not usable ({failure}) — this run will not keep any", dir.display());
            return Self { dir: None, todo_path: None, bodies: BTreeMap::new(), todo: Vec::new(), next_id: 1 };
        }

        let mut bodies = BTreeMap::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for path in entries.flatten().map(|entry| entry.path()) {
                let Some(slug) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else {
                    continue;
                };
                if path.extension().is_some_and(|extension| extension == "md")
                    && let Ok(contents) = std::fs::read_to_string(&path)
                {
                    bodies.insert(slug, strip_frontmatter(&contents));
                }
            }
        }

        let todo_path = run_dir.join(files::TODO);
        let todo: Vec<TodoItem> = std::fs::read(&todo_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let next_id = todo.iter().map(|item| item.id).max().unwrap_or(0) + 1;

        Self { dir: Some(dir), todo_path: Some(todo_path), bodies, todo, next_id }
    }

    /// Service one call, returning the sentence the model is shown as the tool result.
    pub fn apply(&mut self, call: NoteCall) -> String {
        match call {
            NoteCall::Write { name, body } => self.write(&name, &body),
            NoteCall::Read { name } => self.read(&name),
            NoteCall::TodoAdd { text } => self.todo_add(&text),
            NoteCall::TodoComplete { id } => self.todo_complete(id),
        }
    }

    fn write(&mut self, name: &str, body: &str) -> String {
        let Some(slug) = slugify(name) else {
            return format!("`{name}` has no letters or digits in it, so it cannot be a note name.");
        };
        if self.bodies.len() >= MAX_MEMORIES && !self.bodies.contains_key(&slug) {
            return format!(
                "You already have the maximum of {MAX_MEMORIES} notes. Overwrite one instead — the \
                 index is in your system prompt."
            );
        }
        let body = truncated(body.trim(), MAX_BODY);
        let replaced = self.bodies.insert(slug.clone(), body.clone()).is_some();
        self.persist_memory(&slug, &body);
        format!(
            "{} note `{slug}` ({} characters). Its first line is now in your system prompt; \
             `memory_read` gets the whole thing back.",
            if replaced { "Replaced" } else { "Wrote" },
            body.chars().count(),
        )
    }

    fn read(&self, name: &str) -> String {
        let Some(slug) = slugify(name) else {
            return format!("`{name}` is not a note name.");
        };
        match self.bodies.get(&slug) {
            Some(body) => body.clone(),
            None => format!(
                "There is no note called `{slug}`. You have: {}.",
                match self.bodies.is_empty() {
                    true => "none".to_string(),
                    false => self.bodies.keys().cloned().collect::<Vec<_>>().join(", "),
                }
            ),
        }
    }

    fn todo_add(&mut self, text: &str) -> String {
        let text = truncated(text.trim(), MAX_TODO_TEXT);
        if text.is_empty() {
            return "An empty TODO is not a TODO.".to_string();
        }
        // Dropping *done* items first is what stops a long run hitting the cap because of things it
        // finished an hour ago.
        if self.todo.len() >= MAX_TODO {
            if let Some(position) = self.todo.iter().position(|item| item.done) {
                self.todo.remove(position);
            } else {
                return format!(
                    "Your TODO list is full ({MAX_TODO} items) and none of them are done. Finish \
                     something with `todo_complete` first."
                );
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.todo.push(TodoItem { id, text: text.clone(), done: false });
        self.persist_todo();
        format!("Added TODO {id}: {text}")
    }

    fn todo_complete(&mut self, id: u32) -> String {
        let Some(item) = self.todo.iter_mut().find(|item| item.id == id) else {
            return format!("There is no TODO {id}. The list is in your system prompt.");
        };
        if item.done {
            return format!("TODO {id} was already done.");
        }
        item.done = true;
        let text = item.text.clone();
        self.persist_todo();
        format!("Done: {text}")
    }

    /// The block appended to the system prompt every turn (§10). Both lists in full — the TODO
    /// because it is short and is the whole point, the memories as an index because a note can be
    /// eight kilobytes and there can be sixty-four of them.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("\n## Your notes\n\nThese survive everything — a compaction, and the process \
                      restarting. They are the only thing that does.\n\n### TODO\n");
        if self.todo.is_empty() {
            out.push_str("(nothing yet — `todo_add` is how you keep a plan that outlives this \
                          conversation.)\n");
        }
        for item in &self.todo {
            out.push_str(&format!("- [{}] {} — {}\n", if item.done { "x" } else { " " }, item.id, item.text));
        }
        out.push_str("\n### Notes you have written\n");
        if self.bodies.is_empty() {
            out.push_str("(none — `memory_write` is for what you would otherwise have to rediscover.)\n");
        }
        for (slug, body) in &self.bodies {
            out.push_str(&format!("- `{slug}` — {}\n", first_line(body)));
        }
        out
    }

    fn persist_memory(&self, slug: &str, body: &str) {
        let Some(dir) = self.dir.as_ref() else { return };
        let contents = format!("---\nname: {slug}\n---\n\n{body}\n");
        if let Err(failure) = crate::run::write_atomically(&dir.join(format!("{slug}.md")), contents.as_bytes()) {
            eprintln!("notes: {failure}");
        }
    }

    fn persist_todo(&self) {
        let Some(path) = self.todo_path.as_ref() else { return };
        let Ok(json) = serde_json::to_vec_pretty(&self.todo) else { return };
        if let Err(failure) = crate::run::write_atomically(path, &json) {
            eprintln!("notes: {failure}");
        }
    }
}

/// A model's idea of a name, reduced to something that is unambiguously a filename.
///
/// ⚠️ This is the security boundary — see the module's ⚠️. Everything that is not a lowercase letter
/// or a digit becomes a hyphen, runs collapse, and the ends are trimmed: `../../etc/passwd` is
/// `etc-passwd`, and there is no input that produces a `/`, a `.` or an empty string.
pub fn slugify(name: &str) -> Option<String> {
    let mut slug = String::with_capacity(name.len());
    for character in name.chars() {
        match character.is_ascii_alphanumeric() {
            true => slug.push(character.to_ascii_lowercase()),
            false if !slug.ends_with('-') => slug.push('-'),
            false => {}
        }
    }
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(MAX_SLUG).collect();
    let slug = slug.trim_end_matches('-').to_string();
    (!slug.is_empty()).then_some(slug)
}

/// Truncate on a character boundary, because `MAX_BODY` lands in the middle of a multi-byte
/// character the first time a model writes about a Pokémon with an accent in its name.
fn truncated(text: &str, limit: usize) -> String {
    match text.len() <= limit {
        true => text.to_string(),
        false => text.chars().scan(0, |used, c| {
            *used += c.len_utf8();
            (*used <= limit).then_some(c)
        }).collect(),
    }
}

fn first_line(body: &str) -> String {
    let line = body.lines().find(|line| !line.trim().is_empty()).unwrap_or("(empty)").trim();
    truncated(line, INDEX_SUMMARY)
}

/// Drop the `---` block [`Notes::persist_memory`] writes, so a note read back is the body it was
/// written with rather than the file it was written to.
fn strip_frontmatter(contents: &str) -> String {
    let Some(rest) = contents.strip_prefix("---\n") else { return contents.trim().to_string() };
    match rest.split_once("\n---\n") {
        Some((_, body)) => body.trim().to_string(),
        None => contents.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::Scratch;

    /// The whole of W6b's point: what the model writes is in the *next* turn's system prompt, and
    /// still there after the process has gone away and come back.
    #[test]
    fn notes_survive_the_process_that_wrote_them() {
        let scratch = Scratch::new("notes");
        {
            let mut notes = Notes::open(Some(&scratch.0));
            notes.apply(NoteCall::Write {
                name: "Route 3".into(),
                body: "Full of bug catchers.\nThe Mt Moon entrance is at the east end.".into(),
            });
            notes.apply(NoteCall::TodoAdd { text: "beat Brock".into() });
            notes.apply(NoteCall::TodoAdd { text: "buy potions".into() });
            assert!(notes.apply(NoteCall::TodoComplete { id: 1 }).contains("beat Brock"));
        }

        let notes = Notes::open(Some(&scratch.0));
        let rendered = notes.render();
        assert!(rendered.contains("- [x] 1 — beat Brock"), "{rendered}");
        assert!(rendered.contains("- [ ] 2 — buy potions"), "{rendered}");
        assert!(rendered.contains("`route-3` — Full of bug catchers."), "{rendered}");
        assert!(!rendered.contains("Mt Moon entrance"), "the index is one line per note, not the note");

        // …and the body comes back whole, without the frontmatter it was stored with.
        let body = notes.read("route 3");
        assert!(body.starts_with("Full of bug catchers."), "{body}");
        assert!(body.contains("Mt Moon entrance is at the east end."), "{body}");
        assert!(!body.contains("---"), "the frontmatter is storage, not content: {body}");

        // A new id after a restart, rather than one that collides with what is already there.
        let mut notes = notes;
        assert!(notes.apply(NoteCall::TodoAdd { text: "reach Cerulean".into() }).contains("TODO 3"));
    }

    /// ⚠️ The security boundary. A name from a model is not a filename, and the only proof that
    /// matters is that nothing lands outside `memories/`.
    #[test]
    fn a_name_from_a_model_can_never_escape_the_directory() {
        let scratch = Scratch::new("notes-escape");
        let mut notes = Notes::open(Some(&scratch.0));
        for hostile in ["../../etc/passwd", "/etc/shadow", "..", "./../x", "a/b/c", "C:\\\\windows\\\\system"] {
            notes.apply(NoteCall::Write { name: hostile.into(), body: "hello".into() });
        }
        notes.apply(NoteCall::Write { name: "нет".into(), body: "hello".into() });

        let written: Vec<String> = std::fs::read_dir(scratch.0.join(files::MEMORIES))
            .expect("the directory exists")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        for name in &written {
            assert!(name.ends_with(".md") && !name.contains('/') && !name.contains(".."), "{name}");
        }
        // Nothing was created beside the run directory either.
        let beside: Vec<String> = std::fs::read_dir(&scratch.0)
            .expect("readable")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(beside, ["memories"], "nothing but the notes directory was touched");

        assert_eq!(slugify("../../etc/passwd").as_deref(), Some("etc-passwd"));
        assert_eq!(slugify("Mt. Moon — B1F!").as_deref(), Some("mt-moon-b1f"));
        assert_eq!(slugify("---"), None, "a name with nothing in it is refused, not turned into a file");
        assert_eq!(slugify("").as_deref(), None);
        assert!(slugify(&"x".repeat(200)).unwrap().len() <= MAX_SLUG);
    }

    /// The caps are what stop the system prompt becoming the context problem it exists to solve.
    #[test]
    fn the_caps_hold_and_say_why() {
        let mut notes = Notes::open(None);
        for n in 0..MAX_MEMORIES {
            notes.apply(NoteCall::Write { name: format!("note {n}"), body: "x".into() });
        }
        let refused = notes.apply(NoteCall::Write { name: "one too many".into(), body: "x".into() });
        assert!(refused.contains("maximum"), "{refused}");
        // …but overwriting one of the sixty-four is still allowed.
        assert!(notes.apply(NoteCall::Write { name: "note 0".into(), body: "y".into() }).starts_with("Replaced"));

        // Two bytes a character, so the cap bites at half the characters — and lands *on* a
        // boundary rather than splitting one, which is the thing worth asserting.
        let long = notes.apply(NoteCall::Write { name: "note 1".into(), body: "é".repeat(MAX_BODY) });
        assert!(long.contains(&format!("{} characters", MAX_BODY / 2)), "{long}");

        let mut notes = Notes::open(None);
        for n in 0..MAX_TODO {
            notes.apply(NoteCall::TodoAdd { text: format!("thing {n}") });
        }
        let full = notes.apply(NoteCall::TodoAdd { text: "one more".into() });
        assert!(full.contains("full"), "{full}");
        // Completing one makes room, and it is the completed one that is dropped.
        notes.apply(NoteCall::TodoComplete { id: 1 });
        assert!(notes.apply(NoteCall::TodoAdd { text: "one more".into() }).starts_with("Added"));
        assert!(!notes.render().contains("thing 0"), "the finished item made way");

        assert!(notes.apply(NoteCall::TodoAdd { text: "  ".into() }).contains("not a TODO"));
        assert!(notes.apply(NoteCall::TodoComplete { id: 9999 }).contains("no TODO 9999"));
    }

    /// With no run directory the tools still work — they simply keep nothing. That is what the
    /// worker's tests run against, and it must not be a special case in the calling code.
    #[test]
    fn notes_without_a_directory_still_answer() {
        let mut notes = Notes::open(None);
        assert!(notes.apply(NoteCall::Write { name: "x".into(), body: "remember this".into() }).starts_with("Wrote"));
        assert_eq!(notes.apply(NoteCall::Read { name: "x".into() }), "remember this");
        assert!(notes.apply(NoteCall::Read { name: "y".into() }).contains("no note called `y`"));
        assert!(notes.render().contains("`x` — remember this"));
    }
}
