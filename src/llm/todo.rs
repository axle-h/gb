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

/// How many items the list holds, **finished ones included**.
///
/// ⚠️ **Five, and it was 32.** At 32 the cap never bound in practice and nothing else pushed back:
/// the deployed run of 2026-08-26 reached **13 items of which 11 were done** without ever deleting
/// one, so most of the plan in every request was work finished an hour earlier and the two live
/// items were at the bottom. Counting done items against the cap is the whole point — a cap on open
/// items only moves the growth into the tail, which is exactly where it went before.
///
/// Small enough to read at a glance, which is what a plan is for, and small enough that
/// [`TodoList::render`] can show every item rather than hiding a tail of them from the model.
pub const MAX_ITEMS: usize = 5;
/// Long enough for the intent *and* the reason it exists, because there is no longer a note beside
/// it to hold the second half. See the module's second ⚠️.
pub const MAX_TEXT: usize = 200;

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
    /// The three edits: no `id` appends a new item, an `id` with text rewrites that item (and
    /// reopens it, because new text is a new intent), an `id` with no text deletes it. One shape
    /// instead of add-then-delete pairs, so revising the plan is never more expensive than writing
    /// it was.
    ///
    /// ⚠️ **Two tool names reach the delete arm and only one of them is advertised.** `todo_delete`
    /// is the word the catalogue offers; `todo_set` with an `id` and no `text` is the overload it
    /// replaced, kept because a resumed run imitating its own history writes that shape. See
    /// `tools::TODO_TOOL_NAMES` for the turn the missing word cost.
    Set { id: Option<u32>, text: Option<String> },
    Complete { id: u32 },
}

/// What one [`TodoCall`] did: the sentence the model is shown, and whether the list acted on it.
pub struct TodoAnswer {
    pub text: String,
    /// True when nothing changed — a number that is not on the list, an item with no text, a plan
    /// with no room. See [`TodoList::apply_reporting`].
    pub refused: bool,
}

impl TodoAnswer {
    fn done(text: impl Into<String>) -> Self {
        Self { text: text.into(), refused: false }
    }

    fn refused(text: impl Into<String>) -> Self {
        Self { text: text.into(), refused: true }
    }
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
        let mut list = Self { path: Some(path), items, next_id };
        // ⚠️ **Trimmed on the way in, not only on the way up.** `MAX_ITEMS` was 32 and is 5, so a
        // run resumed across that change carries a list the cap would never otherwise touch: `add`
        // only makes room when it needs some, and a model that stops adding keeps the long list for
        // ever. Trimming here makes the cap a property of the list rather than of one code path.
        if list.trim_to_cap() {
            list.persist();
        }
        list
    }

    /// **`POST /api/clear`** — an empty list, and `todo.json` deleted with it.
    ///
    /// ⚠️ **Deleted rather than emptied, and it is the one call that removes the file.** The plan is
    /// the only thing the model writes that outlives a conversation (see this module's header), so
    /// "start again from the system prompt" means nothing at all if the list is still sitting there
    /// waiting to be rendered into the first turn. An empty `todo.json` would do the same job today;
    /// deleting it says what happened to anybody reading the directory afterwards, and leaves the
    /// run in the state a brand new one is in.
    ///
    /// ⚠️ **On the worker thread, like every other write to these files.** A run directory has one
    /// writer, and the emulator thread unlinking a file the worker rewrites at the end of every turn
    /// is exactly the race that rule exists to prevent. A failure is reported and otherwise ignored:
    /// the list in memory is empty either way, so the worst case is a stale file that the next
    /// `todo_set` overwrites, and refusing to play over it would be far worse.
    pub fn cleared(run_dir: Option<&Path>) -> Self {
        let path = run_dir.map(|dir| dir.join(files::TODO));
        if let Some(path) = path.as_deref() {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => eprintln!("could not delete {}: {e}", path.display()),
            }
        }
        Self { path, items: Vec::new(), next_id: 1 }
    }

    /// Service one call, returning the sentence the model is shown as the tool result.
    pub fn apply(&mut self, call: TodoCall) -> String {
        self.apply_reporting(call).text
    }

    /// [`Self::apply`], plus whether the list did anything about it.
    ///
    /// ⚠️ **`refused` exists so the *caller* can stop a repeat, which no answer can do by itself.**
    /// Every refusal here is cheap, deterministic and identical on the second attempt, so a model
    /// that misreads one has no reason not to send it again — and one turn of the deployed run of
    /// 2026-09-03 sent thirty-five. Better wording narrows that (see [`Self::where_to_put_it`]) but
    /// cannot close it; the worker's per-turn guard does. See `Worker::apply_todo`.
    pub fn apply_reporting(&mut self, call: TodoCall) -> TodoAnswer {
        match call {
            TodoCall::Set { id, text } => self.set(id, text.as_deref()),
            TodoCall::Complete { id } => self.complete(id),
        }
    }

    /// Every item, for the UI. The model's copy is [`Self::render`], which is shorter.
    pub fn items(&self) -> &[TodoItem] {
        &self.items
    }

    fn set(&mut self, id: Option<u32>, text: Option<&str>) -> TodoAnswer {
        let text = text.map(|text| truncated(text.trim(), MAX_TEXT)).filter(|text| !text.is_empty());
        match (id, text) {
            (None, Some(text)) => self.add(&text),
            (None, None) => TodoAnswer::refused(
                "An empty TODO is not a TODO. Give `todo_set` some `text`, or call `todo_delete` \
                 with an `id`.",
            ),
            (Some(id), Some(text)) => match self.items.iter_mut().find(|item| item.id == id) {
                Some(item) => {
                    item.text = text.clone();
                    item.done = false;
                    self.persist();
                    TodoAnswer::done(format!("TODO {id} is now: {text}"))
                }
                // ⚠️ **This used to append, "forgiving on purpose", and the forgiveness is what
                // wrecked a live plan.** The argument was that a stale id still carries a real
                // intent, so keeping the text beats spending a round trip. What it could not tell
                // apart is a model using `text` as a *command word*: the deployed run of
                // 2026-09-03 sent `{"id": 5, "text": "Delete"}` and was answered "There was no TODO
                // 5, so this went on the end. Added TODO 12: Delete". Two of the five items in that
                // run's plan — the cap is five — existed only because of this branch, one of them
                // the literal word `Delete` and one a byte-for-byte duplicate of another. With none
                // of them done, `add` then refused everything as full, and the plan is the only
                // thing that survives a compaction.
                //
                // Refusing costs the round trip the old branch was avoiding, and buys back the two
                // things it could not give: the model is told which ids actually exist, so the next
                // call lands, and a plan cannot be filled with items nobody asked for.
                None => TodoAnswer::refused(format!(
                    "There is no TODO {id}, so nothing was changed. {}", self.where_to_put_it())),
            },
            (Some(id), None) => match self.items.iter().position(|item| item.id == id) {
                Some(position) => {
                    let item = self.items.remove(position);
                    self.persist();
                    TodoAnswer::done(format!("Removed TODO {id}: {}", item.text))
                }
                None => TodoAnswer::refused(
                    format!("There is no TODO {id}. {}", self.where_to_put_it())),
            },
        }
    }

    /// The half of every "there is no TODO n" that tells the model what to do about it.
    ///
    /// ⚠️ **It names the ids, and the sentence it replaces did not.** That one read "The list is in
    /// the turn you were just sent", which points a model that has just mis-read the list back at
    /// the list, with nothing new in its hands — so the cheapest next move is to try again, and
    /// that is what one turn of the deployed run did thirty-five times. The same shape `press_buttons`
    /// had before it was withdrawn: **a call that fails cheaply and identically will be repeated
    /// until something else stops it.** An answer has to move the model on within itself.
    fn where_to_put_it(&self) -> String {
        match self.numbers() {
            None => "Your plan is empty — `todo_set` with no `id` starts it.".to_string(),
            Some(ids) => format!(
                "Your plan holds {ids}. Use one of those numbers, or call `todo_set` with no `id` \
                 to put this on the end as a new item."
            ),
        }
    }

    /// The ids the list actually holds, for an answer that has to name them. `None` when there are
    /// none, because "your plan holds " with nothing after it reads as a bug in the tool.
    fn numbers(&self) -> Option<String> {
        match self.items.is_empty() {
            true => None,
            false => Some(
                self.items.iter().map(|item| item.id.to_string()).collect::<Vec<_>>().join(", "),
            ),
        }
    }

    /// Bring the list down to [`MAX_ITEMS`], returning whether anything was dropped.
    ///
    /// **Finished items go first, oldest first**: they are the ones the run has already had the
    /// value of, and dropping live work to keep a tick would be the wrong way round. Open items are
    /// only reached by a list that arrived over the cap from disk — [`Self::add`] refuses rather
    /// than deleting a plan the model still means to carry out.
    fn trim_to_cap(&mut self) -> bool {
        let mut dropped = false;
        while self.items.len() > MAX_ITEMS {
            let position = self.items.iter().position(|item| item.done).unwrap_or(0);
            self.items.remove(position);
            dropped = true;
        }
        dropped
    }

    fn add(&mut self, text: &str) -> TodoAnswer {
        let text = truncated(text.trim(), MAX_TEXT);
        if text.is_empty() {
            return TodoAnswer::refused("An empty TODO is not a TODO.");
        }
        // Dropping the oldest *done* item is what makes room, since the cap counts them: a plan
        // whose finished half is squeezing out the live half is the failure this exists to prevent.
        //
        // ⚠️ **What went is named in the answer, because an id that vanishes silently is an id the
        // model goes on calling.** TODO 5 of the deployed run of 2026-09-03 was real, completed, and
        // evicted here several hundred turns before the model spent a turn trying to delete it. The
        // model is reading this string at the moment the eviction happens, which is the only place
        // the news is both true and cheap: `render` would have to carry it forward, and the plan
        // message is only re-emitted when it changes.
        let mut evicted = None;
        if self.items.len() >= MAX_ITEMS {
            if let Some(position) = self.items.iter().position(|item| item.done) {
                let item = self.items.remove(position);
                evicted = Some(format!(
                    " TODO {} was finished and has been dropped to make room: {}",
                    item.id, item.text,
                ));
            } else {
                // ⚠️ **The ids, but not `where_to_put_it`'s escape clause.** That one ends "or call
                // `todo_set` with no `id` to put this on the end as a new item", which is the call
                // that has just been refused for having nowhere to go. An answer that suggests
                // retrying the failure is the loop this whole change is about.
                return TodoAnswer::refused(format!(
                    "Your plan is full: {MAX_ITEMS} items, and none of them are done. It is meant \
                     to be short. Finish one with `todo_complete`, or drop the one you no longer \
                     mean to do with `todo_delete`, then add this. It holds {}.",
                    self.numbers().unwrap_or_else(|| "nothing".to_string()),
                ));
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(TodoItem { id, text: text.clone(), done: false });
        self.persist();
        TodoAnswer::done(format!("Added TODO {id}: {text}{}", evicted.unwrap_or_default()))
    }

    fn complete(&mut self, id: u32) -> TodoAnswer {
        if !self.items.iter().any(|item| item.id == id) {
            return TodoAnswer::refused(format!("There is no TODO {id}. {}", self.where_to_put_it()));
        }
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            unreachable!("just checked")
        };
        if item.done {
            return TodoAnswer::done(format!("TODO {id} was already done."));
        }
        item.done = true;
        let text = item.text.clone();
        self.persist();
        TodoAnswer::done(format!("Done: {text}"))
    }

    /// The block that goes into a turn as a message of its own (§10).
    ///
    /// ⚠️ **In the list's own order, finished items included and in place.** It used to partition —
    /// open items first, then the last few done ones — which meant the order the model wrote was not
    /// the order it read back, and an item ticked off in the middle of a plan jumped to the bottom.
    /// A plan is a sequence; re-sorting it silently is a way to make the model's own numbering stop
    /// meaning anything. [`MAX_ITEMS`] is what keeps the whole thing short now, so nothing has to be
    /// hidden to make it fit.
    ///
    /// ⚠️ **The rules moved out of here and into the system prompt, which is the cache-cheap half of
    /// the same split.** Four of this preamble's five paragraphs restated `SYSTEM_PROMPT`'s "Your
    /// plan, and keeping it" bullets almost verbatim — draft-not-commitment, the cap counting
    /// finished items, the order being kept — and the two copies pull opposite ways at the prompt
    /// cache: message 0 is prefilled once and cached for the whole run, while this message is
    /// **fresh tokens** on every edit and every `PLAN_REFRESH_TURNS` refresh. Restating the rules in
    /// the expensive copy was the wrong way round.
    ///
    /// ⚠️ **What stays is the part that is not a rule.** "Look at it now and ask whether it still
    /// describes what you are doing" is the only line here whose value *is* its recency — it is a
    /// prompt to act, addressed to the plan directly below it, and it is the entire reason this
    /// message is re-appended to a model that demonstrably stopped reading its own list. The cap is
    /// kept as a clause rather than a paragraph because it is the one rule the very next call can
    /// break. Everything else is one line back in message 0.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str(PLAN_HEADING);
        out.push_str(
            &format!(
                "\n\nLook at it now and ask whether it still describes what you are doing. If \
                 something on it is done, `todo_complete` it. If you have been told something you \
                 will need later — an errand, a place, a name, what is blocking you — add it, with \
                 who told you. If the list has not changed while you have been going round in \
                 circles, it is the list that is wrong. It holds {MAX_ITEMS}, finished ones \
                 included, so tick off or delete before you add.\n\n\
                 ⚠️ This replaces any earlier `## Your plan` message in this conversation. Older \
                 copies are left where they were so nothing above them has to be rewritten; the one \
                 nearest the end is always the current one.\n\n"
            ),
        );

        if self.items.is_empty() {
            out.push_str(
                "(empty — `todo_set` is how a plan outlives this conversation. Start one now, even \
                 a rough one.)\n",
            );
            return out;
        }
        for item in &self.items {
            let tick = if item.done { 'x' } else { ' ' };
            out.push_str(&format!("- [{tick}] {} — {}\n", item.id, item.text));
        }
        if self.items.iter().all(|item| item.done) {
            out.push_str("\n(nothing outstanding — decide what comes next.)\n");
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

    /// The shape every plain append takes on the wire: `todo_set` with no `id`.
    fn add(text: impl Into<String>) -> TodoCall {
        TodoCall::Set { id: None, text: Some(text.into()) }
    }

    /// The whole of W6b's point: what the model writes is in the *next* turn, and still there after
    /// the process has gone away and come back.
    #[test]
    fn the_plan_survives_the_process_that_wrote_it() {
        let scratch = Scratch::new("todo");
        {
            let mut todo = TodoList::open(Some(&scratch.0));
            todo.apply(add("beat Brock"));
            todo.apply(add("buy potions"));
            assert!(todo.apply(TodoCall::Complete { id: 1 }).contains("beat Brock"));
        }

        let mut todo = TodoList::open(Some(&scratch.0));
        let rendered = todo.render();
        assert!(rendered.starts_with(PLAN_HEADING), "the worker finds the old copy by this: {rendered}");
        assert!(rendered.contains("- [x] 1 — beat Brock"), "{rendered}");
        assert!(rendered.contains("- [ ] 2 — buy potions"), "{rendered}");

        // A new id after a restart, rather than one that collides with what is already there.
        assert!(todo.apply(add("reach Cerulean")).contains("TODO 3"));
    }

    /// The caps are what stop the plan becoming the context problem it exists to solve — and the
    /// tail of finished work is the part that grows without bound in a long run.
    #[test]
    fn the_caps_hold_and_say_why() {
        let mut todo = TodoList::open(None);
        for n in 0..MAX_ITEMS {
            todo.apply(add(format!("thing {n}")));
        }
        let full = todo.apply(add("one more"));
        assert!(full.contains("full"), "{full}");
        assert!(full.contains("It holds 1, 2, 3, 4, 5"), "a refusal names the ids: {full}");
        // ⚠️ And it must not end by suggesting the very call it just refused — see `add`.
        assert!(!full.contains("no `id` to put this on the end"), "{full}");

        // Completing one makes room, and it is the completed one that is dropped.
        todo.apply(TodoCall::Complete { id: 1 });
        assert!(todo.apply(add("one more")).starts_with("Added"));
        assert!(!todo.render().contains("thing 0"), "the finished item made way");

        assert!(todo.apply(add("  ")).contains("not a TODO"));
        assert!(todo.apply(TodoCall::Complete { id: 9999 }).contains("no TODO 9999"));

        // Two bytes a character, so the cap bites at half the characters — and lands *on* a
        // boundary rather than splitting one, which is the thing worth asserting.
        let mut todo = TodoList::open(None);
        todo.apply(add("é".repeat(MAX_TEXT)));
        assert_eq!(todo.items()[0].text.chars().count(), MAX_TEXT / 2);
    }

    /// `todo_set` is one tool doing two jobs — append without an `id`, rewrite with one — and the
    /// delete arm behind it is what `todo_delete` reaches. The plan is meant to be *rewritten* — an
    /// item that turned out wrong is replaced, not completed — so none of the three may cost more
    /// than one call.
    #[test]
    fn set_rewrites_and_deletes_as_well_as_adding() {
        let mut todo = TodoList::open(None);
        todo.apply(add("beat Brock"));
        todo.apply(add("go to Mt Moon via Route 4"));

        // A rewrite keeps the number and reopens a finished item: new text is a new intent.
        todo.apply(TodoCall::Complete { id: 1 });
        let answer = todo.apply(TodoCall::Set { id: Some(1), text: Some("rematch Brock with Mankey".into()) });
        assert!(answer.contains("TODO 1 is now"), "{answer}");
        assert!(todo.render().contains("- [ ] 1 — rematch Brock with Mankey"), "{}", todo.render());

        // An `id` with no text deletes — the arm `todo_delete` parses to.
        let answer = todo.apply(TodoCall::Set { id: Some(2), text: None });
        assert!(answer.contains("Removed TODO 2"), "{answer}");
        assert!(!todo.render().contains("Route 4"));

        // Deleting nothing and writing nothing both answer rather than fail.
        assert!(todo.apply(TodoCall::Set { id: Some(99), text: None }).contains("no TODO 99"));
        assert!(todo.apply(TodoCall::Set { id: None, text: Some("  ".into()) }).contains("not a TODO"));
        assert!(todo.apply(TodoCall::Set { id: None, text: None }).contains("not a TODO"));
    }

    /// ⚠️ **A number that is not on the list changes nothing, and the answer says which numbers
    /// are.** Both halves are the deployed run of 2026-09-03, and they are one bug seen twice.
    ///
    /// This branch used to append, on the argument that a stale id still carries a real intent. It
    /// cannot tell that apart from a model using `text` as a command word: `{"id": 5, "text":
    /// "Delete"}` was answered "There was no TODO 5, so this went on the end. Added TODO 12:
    /// Delete", and that run's five-item plan ended up holding the literal word `Delete`, a
    /// byte-for-byte duplicate of another item, and no free room.
    ///
    /// And the refusal it did give — "The list is in the turn you were just sent" — hands back
    /// nothing, so the cheapest next move is to send the same call again. That turn sent it
    /// thirty-five times. Naming the ids is what makes the *next* call land; `Worker::apply_todo`
    /// is what makes the same call twice not free.
    #[test]
    fn an_id_that_is_not_on_the_list_changes_nothing_and_the_answer_names_the_ones_that_are() {
        let mut todo = TodoList::open(None);
        todo.apply(add("beat Brock"));
        todo.apply(add("buy potions"));

        for call in [
            TodoCall::Set { id: Some(5), text: Some("Delete".into()) },
            TodoCall::Set { id: Some(5), text: None },
            TodoCall::Complete { id: 5 },
        ] {
            let answer = todo.apply_reporting(call.clone());
            assert!(answer.refused, "{call:?} should be refused: {}", answer.text);
            assert!(answer.text.contains("no TODO 5"), "{}", answer.text);
            assert!(answer.text.contains("holds 1, 2"), "it has to name the ids: {}", answer.text);
        }
        assert_eq!(todo.items().len(), 2, "a stale id must not add anything: {:?}", todo.items());
        assert!(!todo.render().contains("Delete"), "{}", todo.render());

        // With nothing to name it says so rather than printing an empty list.
        let mut empty = TodoList::open(None);
        assert!(empty.apply(TodoCall::Complete { id: 1 }).contains("plan is empty"));
    }

    /// ⚠️ **An id that vanishes is an id the model goes on calling**, so the eviction is named in
    /// the answer of the call that caused it. TODO 5 of the deployed run of 2026-09-03 was real,
    /// completed, and squeezed out here several hundred turns before the model spent a whole turn
    /// trying to delete it.
    #[test]
    fn the_item_squeezed_out_to_make_room_is_named() {
        let mut todo = TodoList::open(None);
        for n in 0..MAX_ITEMS {
            todo.apply(add(format!("thing {n}")));
        }
        todo.apply(TodoCall::Complete { id: 1 });

        let answer = todo.apply(add("the new thing"));
        assert!(answer.starts_with("Added TODO"), "{answer}");
        assert!(answer.contains("TODO 1 was finished and has been dropped"), "{answer}");
        assert!(answer.contains("thing 0"), "it names what went, not only its number: {answer}");
    }

    /// ⚠️ The model's copy is not the UI's. A run that finishes fifty things must not carry fifty
    /// answered lines in every request for the rest of its life. The cap is what does that now:
    /// finished items count against it, so a run that keeps working keeps evicting its own history
    /// rather than accumulating it.
    #[test]
    fn finished_work_is_squeezed_out_by_the_cap_rather_than_hidden() {
        let mut todo = TodoList::open(None);
        for n in 0..10 {
            todo.apply(add(format!("thing {n}")));
            todo.apply(TodoCall::Complete { id: n + 1 });
        }
        todo.apply(add("the one thing left"));

        assert_eq!(todo.items().len(), MAX_ITEMS, "the cap counts finished items too");
        let rendered = todo.render();
        // Nothing is hidden from the model any more: what it holds is what it is shown.
        assert_eq!(
            rendered.matches("- [").count(),
            MAX_ITEMS,
            "every item the list holds should be rendered: {rendered}"
        );
        assert!(!rendered.contains("not listed"), "nothing is hidden now: {rendered}");
        assert!(rendered.contains("- [ ] 11 — the one thing left"), "{rendered}");
        assert!(rendered.contains("thing 9"), "the most recent finished work is still there");
        assert!(!rendered.contains("thing 0"), "the oldest was evicted: {rendered}");
    }

    /// ⚠️ **The order the model writes is the order it reads back**, done items in place. This used
    /// to partition, so ticking off an item in the middle of a plan moved it to the bottom and the
    /// numbering stopped matching what the model had written.
    #[test]
    fn the_list_is_rendered_in_the_order_the_model_maintains() {
        let mut todo = TodoList::open(None);
        for text in ["first", "second", "third", "fourth"] {
            todo.apply(add(text));
        }
        // Tick off the one in the middle: it must not move.
        todo.apply(TodoCall::Complete { id: 2 });

        let rendered = todo.render();
        let lines: Vec<&str> = rendered.lines().filter(|line| line.starts_with("- [")).collect();
        assert_eq!(
            lines,
            vec![
                "- [ ] 1 — first",
                "- [x] 2 — second",
                "- [ ] 3 — third",
                "- [ ] 4 — fourth",
            ],
            "{rendered}"
        );

        // And a rewrite stays where it was rather than going to the end.
        todo.apply(TodoCall::Set { id: Some(1), text: Some("first, revised".to_string()) });
        let first = todo.render();
        let lines: Vec<&str> = first.lines().filter(|line| line.starts_with("- [")).collect();
        assert_eq!(lines[0], "- [ ] 1 — first, revised", "{first}");
    }

    /// A list written when the cap was 32 has to come under it on the way in, not merely stop
    /// growing — a model that never adds again would otherwise keep the long list for ever.
    #[test]
    fn a_list_from_before_the_cap_is_trimmed_when_it_is_opened() {
        let scratch = crate::run::tests::Scratch::new("todo-legacy-cap");
        let long: Vec<TodoItem> = (1..=13)
            .map(|id| TodoItem { id, text: format!("thing {id}"), done: id <= 11 })
            .collect();
        std::fs::write(
            scratch.0.join(crate::run::files::TODO),
            serde_json::to_vec(&long).expect("serialises"),
        )
        .expect("write");

        let todo = TodoList::open(Some(&scratch.0));
        assert_eq!(todo.items().len(), MAX_ITEMS, "the long list was not trimmed");
        // The two live items survived; the finished ones were what went.
        let open: Vec<&str> = todo.items().iter().filter(|i| !i.done).map(|i| i.text.as_str()).collect();
        assert_eq!(open, vec!["thing 12", "thing 13"], "live work was dropped to keep ticks");

        // And it was written back, so the next process does not have to trim it again.
        let reopened = TodoList::open(Some(&scratch.0));
        assert_eq!(reopened.items().len(), MAX_ITEMS);
    }

    /// ⚠️ **`POST /api/clear` deletes the file, and an empty list in memory is not enough.** The
    /// plan is the one thing a model writes that outlives its conversation, so a clear that only
    /// forgot it in memory would hand the whole thing back at the next process start.
    #[test]
    fn a_cleared_list_takes_the_file_with_it() {
        let scratch = crate::run::tests::Scratch::new("todo-cleared");
        let mut todo = TodoList::open(Some(&scratch.0));
        todo.apply(add("deliver the parcel to Oak"));
        let path = scratch.0.join(crate::run::files::TODO);
        assert!(path.is_file(), "the precondition: there is a plan on disk to delete");

        let cleared = TodoList::cleared(Some(&scratch.0));
        assert!(cleared.items().is_empty(), "the list in memory kept the old plan");
        assert!(!path.is_file(), "{} outlived the clear", path.display());
        // Ids start again, so the first item of the next plan is 1 rather than 2.
        assert!(TodoList::open(Some(&scratch.0)).render().contains("(empty"), "and the next process sees none");

        // Clearing a run that never wrote a plan is not an error, and there is no file left behind.
        let empty = crate::run::tests::Scratch::new("todo-cleared-empty");
        assert!(TodoList::cleared(Some(&empty.0)).items().is_empty());
        assert!(TodoList::cleared(None).items().is_empty(), "…nor is one without a directory at all");
    }

    /// With no run directory the tools still work — they simply keep nothing. That is what the
    /// worker's tests run against, and it must not be a special case in the calling code.
    #[test]
    fn a_list_without_a_directory_still_answers() {
        let mut todo = TodoList::open(None);
        assert!(todo.render().contains("(empty"));
        assert!(todo.apply(add("go north")).starts_with("Added"));
        assert!(todo.render().contains("- [ ] 1 — go north"));
    }
}
