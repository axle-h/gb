//! The conversation on disk: what a restarted process resumes on, and what a compaction destroyed.
//!
//! Two files in the run directory, two audiences, and the split is the whole design:
//!
//! | file | written | read by | job |
//! |---|---|---|---|
//! | [`files::HISTORY`] | rewritten whole, once a turn | the *next* process | the live history |
//! | [`files::CONVERSATION`] | appended, once a turn | people, `jq`, the archive | every message ever |
//!
//! Before this existed the history was a private `Vec<Message>` that never touched disk, so a
//! rollout, an OOM or a node loss left the game, the plan and the transcript intact and the model's
//! actual memory empty; and a compaction ([`compaction::apply_summary`]) rebuilt the history as
//! `[system, summary, tail]` with no copy of the middle anywhere. `transcript.jsonl` is not that
//! copy and cannot be made into one: it holds `UiEvent`s, carries no rendered situation, plan
//! message or summary, and truncates every tool result at `MAX_TOOL_RESULT`.
//!
//! ⚠️ **One append-only file replayed on load was the other design, and it is worse.** Rebuilding
//! the live history from the log means writing a reducer for `pop`, [`compaction::evict_images`]'
//! in-place rewrite, `apply_summary`'s rebuild and `trim_history`'s `drain`, and keeping it
//! bit-exact with `compaction.rs` for ever. A divergence corrupts a resumed history *silently* and
//! the symptom is a 400 on every request for the rest of the run. It also reads the whole file at
//! every start, which is the unbounded read that OOM-killed the deployed pod before
//! [`crate::run::transcript::read_since`] learned to work from the tail. Two files instead: the
//! restore path is one `serde_json::from_slice`, and the log is never load-bearing.
//!
//! ⚠️ **Both copies are image-evicted, and that is not only about size.** A `read_map` picture is
//! hundreds of kilobytes of base64, but the reason it *must* not round-trip is
//! [`ImageUrl::tokens`](crate::llm::protocol::ImageUrl): it is `#[serde(skip)]` with a default of
//! `IMAGE_TOKENS` (85), while a real map costs 765 to 3825. A restored picture would therefore be
//! priced at a twentieth of its weight, [`Accounting::occupancy`](crate::llm::accounting::Accounting::occupancy) would read a full context as
//! nearly empty, and compaction would never fire again. With no image parts stored there is nothing
//! to mis-default. The eviction goes through [`compaction::evict_images`] rather than being
//! open-coded, because compaction's own `is_evicted_image` sniffs the [`compaction::EVICTED`] wording
//! and only one place may own it. Same call, same argument, as `incident::recent_turns`.
//!
//! ⚠️ **The run directory is captured here, and re-read per write by `transcript.rs` — the
//! inversion is deliberate.** The transcript thread re-reads because it is driven by an unrelated
//! event stream and would never otherwise learn that `POST /api/new-run` swapped the directory
//! underneath it. The worker *does* learn, through the `Restart` cell at the top of
//! `Worker::run_one`. So here re-reading would be the bug: a turn already in flight when a new run
//! starts belongs to the **old** game, and filing its conversation in the new run's directory would
//! leave the new run resuming into a conversation about a game that no longer exists.

use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use crate::llm::compaction;
use crate::llm::prompt;
use crate::llm::protocol::{Message, Role};
use crate::run::files;

/// The envelope's version. Bumped when the *meaning* of a field changes; a mismatch is a fresh
/// conversation rather than a migration, because the cost of getting a resumed history subtly wrong
/// is far higher than the cost of one run starting its conversation over.
const VERSION: u32 = 1;

/// What [`History::open`] recovered besides the messages. `None` on a conversation that started here.
#[derive(Debug, Clone)]
pub struct Restored {
    /// How many messages came back, not counting the system prompt or the note appended after them.
    pub messages: usize,
    /// The turn the last process was on when it wrote this. Carried only so the save taken at
    /// construction does not report the run as being back at turn 0.
    pub turn: u64,
    /// The endpoint-versus-us token ratio the last process had measured. See [`Accounting::resumed`](crate::llm::accounting::Accounting::resumed).
    pub calibration: f64,
    /// How many overworld turns had passed since the plan was last repositioned.
    pub turns_since_plan: u32,
    /// Whether [`prompt::SYSTEM_PROMPT`] differs from the one this conversation was being held
    /// under. The new one is in force either way; this is only so it can be said out loud.
    pub system_prompt_changed: bool,
}

/// What one compaction did: the marker line, and the `Compacted` event, are both built from this so
/// the file and the page cannot disagree about it.
#[derive(Debug, Clone)]
pub struct CompactionNote {
    pub before: u64,
    pub after: u64,
    pub images_evicted: usize,
    /// How many messages the summary replaced. Zero when stage 1 was the whole compaction.
    pub dropped: usize,
    /// The prose stage 2 wrote, when it ran.
    pub summary: Option<String>,
}

/// The live conversation, and the two files behind it.
///
/// ⚠️ **Deliberately not [`Clone`].** The one site that wants the messages is the `ChatRequest`,
/// which takes them by value; a `Clone` impl would make `self.history.clone()` there compile into a
/// second `History` — carrying a duplicate of the run directory's write path — rather than the
/// `Vec<Message>` the caller meant. `to_vec()` through the `Deref` says which one it wants.
#[derive(Debug)]
pub struct History {
    /// `None` in tests and under `--policy random`: a history with nowhere to live still works, it
    /// just forgets everything, which is what every process did before this module existed.
    dir: Option<PathBuf>,
    messages: Vec<Message>,
    /// How much of `messages` is already in the log.
    ///
    /// ⚠️ **An index into a vector that compaction rewrites**, so every path that shortens
    /// `messages` has to put this back — which is what [`Self::note_compaction`] is for. It is only
    /// ever sound because the log is flushed *before* `compact_if_needed` runs, so everything a
    /// compaction removes has already been written down.
    logged: usize,
    restored: Option<Restored>,
}

/// The on-disk envelope.
///
/// No `deny_unknown_fields` and `#[serde(default)]` throughout, so a field added by a later build is
/// readable by an older one and vice versa — the same forgiving shape as `RunMeta`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    version: u32,
    #[serde(default)]
    run_id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    saved_at: String,
    #[serde(default)]
    turn: u64,
    #[serde(default = "one")]
    calibration: f64,
    #[serde(default)]
    turns_since_plan: u32,
    /// The system prompt this conversation was last being held under.
    ///
    /// ⚠️ **Stored to be *compared*, never to be restored**, and the two are a hair apart. Index 0
    /// is always re-minted from [`prompt::system_message`] — that is the whole reason it is not in
    /// `messages` below — so putting this back into the history would undo it and pin a deployment's
    /// edit to whatever the last process happened to be running.
    /// `the_stored_system_prompt_is_compared_and_never_restored` is the guard.
    ///
    /// The full text rather than a hash, for two reasons: an exact comparison means "changed in any
    /// way" means exactly that, and `DefaultHasher` is explicitly not stable across Rust releases,
    /// so a toolchain bump would report a change that never happened. It is ~10 KB against a file
    /// already bounded by the context window.
    #[serde(default)]
    system_prompt: String,
    /// ⚠️ **Messages `1..n`: index 0 is deliberately absent.** The system prompt is a compile-time
    /// constant, so storing it would pin a deployment's edit to whatever the last process happened
    /// to be running — the one message a rollout most needs to change is the one a naive round trip
    /// would freeze. It is re-minted by [`prompt::system_message`] on the way back in. Storing it
    /// would also open the door to a history holding two `Role::System` messages.
    #[serde(default)]
    messages: Vec<Message>,
}

fn one() -> f64 {
    1.0
}

impl History {
    /// Open the conversation in a run directory, restoring it when there is one to restore.
    ///
    /// Never fails. An unreadable, truncated or version-skewed file is a fresh conversation with a
    /// line on stderr, exactly as `TodoList::open` treats a broken `todo.json`: losing the
    /// conversation is bad, and refusing to play because of it is worse.
    pub fn open(run_dir: Option<&Path>) -> Self {
        Self::start(run_dir, true)
    }

    /// The conversation for a run that is *starting*, which is never the one on disk.
    ///
    /// ⚠️ **This clears and must never reload.** It is what `Worker::apply_restart` calls when
    /// `POST /api/new-run` swaps the game out, and a reload there would hand the new run the dead
    /// game's memory. Today `RunDir::open`'s fresh path always mints an empty directory, so the two
    /// would behave the same by luck; this is the constructor that makes it true by construction.
    /// It writes its empty state out immediately, so there is no window in which a `history.json`
    /// in a run directory describes some other run.
    pub fn fresh(run_dir: Option<&Path>) -> Self {
        Self::start(run_dir, false)
    }

    fn start(run_dir: Option<&Path>, restore: bool) -> Self {
        let dir = run_dir.map(Path::to_path_buf);
        let restored = match restore && crate::llm::config::restore_history() {
            true => dir.as_deref().and_then(read_saved),
            false => None,
        };

        let mut history = Self {
            messages: vec![prompt::system_message()],
            logged: 1,
            dir,
            restored: None,
        };
        history.write_header();
        // ⚠️ **The system prompt is logged here, explicitly, and every process logs its own.** The
        // watermark cannot do it: on a restore the messages behind it were written by the *previous*
        // process, and index 0 was not. Doing it per process is what makes each process's stretch of
        // the log self-describing, and it is what makes a changed prompt visible in the record
        // rather than only in a warning that scrolls away.
        history.log_message(0, &prompt::system_message());

        if let Some((saved, messages)) = restored {
            let count = messages.len();
            history.messages.extend(messages);
            // Everything restored is already in the log this process is about to append to — it was
            // written there by the process that produced it.
            history.logged = history.messages.len();
            // ⚠️ **An empty stored prompt is "not recorded", not "changed".** The field is
            // `#[serde(default)]` so that a file written before it existed reads rather than being
            // thrown away, and reporting those as a change would be a warning about nothing.
            let changed = !saved.system_prompt.is_empty() && saved.system_prompt != prompt::SYSTEM_PROMPT;
            if changed {
                eprintln!(
                    "the system prompt has changed since this conversation was saved ({} bytes -> {}); \
                     the new one is in force and the conversation was kept",
                    saved.system_prompt.len(),
                    prompt::SYSTEM_PROMPT.len(),
                );
                history.append_line(&serde_json::json!({
                    "kind": "system_prompt_changed",
                    "at": crate::web::published::now_ms(),
                    "was_bytes": saved.system_prompt.len(),
                    "now_bytes": prompt::SYSTEM_PROMPT.len(),
                }));
            }
            history.restored = Some(Restored {
                messages: count,
                turn: saved.turn,
                calibration: saved.calibration,
                turns_since_plan: saved.turns_since_plan,
                system_prompt_changed: changed,
            });
            // ⚠️ **Appended after the watermark, so it is logged like anything else.** The note is
            // a real message the model reads and it belongs in the record of what the model was
            // sent. It also has to sit at the tail rather than anywhere earlier: it is written
            // fresh by every process, and a message that changes near the front of the history
            // would throw away the cached prefill of everything after it.
            history.messages.push(Message::user(prompt::RESUMED_NOTE));
        }

        // Puts the resume note in the log now rather than at the end of the first turn, so a process
        // that dies before finishing one still leaves a complete record of what it sent.
        history.flush_log(0);
        history.save();
        history
    }

    /// What was recovered, if anything. Drives `Accounting::resumed` and the notice on the page.
    pub fn restored(&self) -> Option<&Restored> {
        self.restored.as_ref()
    }

    /// Write the turn down: append whatever is new to the log, then rewrite the live file.
    ///
    /// ⚠️ **Called between `decide` returning and the outcome being sent, never at the end of the
    /// turn.** The moment the worker sends its `TurnOutcome` the emulator thread may act on it, and
    /// if that action wins the game then `hall_of_fame::archive` copies the run directory — so
    /// anything written after the send races the archive and usually loses. Publishing the
    /// `Decision`, and `compact_if_needed` with its whole summarising completion, are both after it.
    /// Checkpointing first makes durability precede visibility and closes the race by construction.
    /// It is the same argument that made the archiver *follow* the transcript rather than copy it.
    pub fn checkpoint(&mut self, turn: u64, calibration: f64, turns_since_plan: u32) {
        self.flush_log(turn);
        self.save_with(turn, calibration, turns_since_plan);
    }

    /// Record what a compaction did, and put the log's watermark back.
    ///
    /// ⚠️ **The watermark reset is the whole reason this is a method rather than a log line.**
    /// `logged` indexes `messages`, and a compaction is the one thing that makes the vector shorter.
    /// It is safe because [`Self::checkpoint`] has already flushed everything the compaction is
    /// about to remove: `evict_images` rewrites in place (and the log holds the evicted text, which
    /// is what the live history will hold afterwards too), while `apply_summary` and `trim_history`
    /// only ever drop messages that are already written down.
    pub fn note_compaction(&mut self, turn: u64, note: &CompactionNote) {
        self.append_line(&serde_json::json!({
            "kind": "compaction",
            "turn": turn,
            "at": crate::web::published::now_ms(),
            "before": note.before,
            "after": note.after,
            "images_evicted": note.images_evicted,
            "dropped": note.dropped,
            "summarised": note.summary.is_some(),
        }));
        // The summary goes in as an ordinary message line, so a reader filtering on `kind ==
        // "message"` still sees every message that ever entered the history — including the one
        // written to replace the messages it just watched disappear.
        if let Some(summary) = &note.summary {
            self.log_message(turn, &compaction::summary_message(summary));
        }
        self.logged = self.messages.len();
    }

    /// One line naming the run and this process, so a restart is legible in the log rather than
    /// showing up as a conversation that inexplicably repeats itself.
    fn write_header(&mut self) {
        let restored = self.restored.as_ref().map_or(0, |r| r.messages);
        self.append_line(&serde_json::json!({
            "kind": "run",
            "at": crate::web::published::now_ms(),
            "version": VERSION,
            "restored": restored,
        }));
    }

    fn flush_log(&mut self, turn: u64) {
        // ⚠️ `DerefMut` hands the worker the vector itself, and one path in `decide` pops from it.
        // A watermark past the end would panic the slice below, so it is clamped rather than
        // trusted — and asserted in debug, since a *silent* clamp would hide a pop that dropped a
        // message the log never received.
        debug_assert!(self.logged <= self.messages.len(), "the log watermark ran past the history");
        self.logged = self.logged.min(self.messages.len());
        let mut fresh = self.messages[self.logged..].to_vec();
        compaction::evict_images(&mut fresh, 0);
        for message in &fresh {
            self.log_message(turn, message);
        }
        self.logged = self.messages.len();
    }

    fn log_message(&mut self, turn: u64, message: &Message) {
        self.append_line(&serde_json::json!({
            "kind": "message",
            "turn": turn,
            "at": crate::web::published::now_ms(),
            "message": message,
        }));
    }

    /// The save taken at construction, before any turn has run.
    ///
    /// ⚠️ **It writes the *restored* figures back, not defaults.** `save_with(0, 1.0, 0)` looks
    /// harmless here because the first turn's checkpoint overwrites it a few seconds later, but a
    /// process that dies before finishing a turn would then have walked the calibration back to 1.0
    /// — and a run that keeps restarting under a crash loop would lose it for good, which is the
    /// silent half of a context that stops compacting.
    fn save(&mut self) {
        let (turn, calibration, turns_since_plan) = match &self.restored {
            Some(restored) => (restored.turn, restored.calibration, restored.turns_since_plan),
            None => (0, 1.0, 0),
        };
        self.save_with(turn, calibration, turns_since_plan);
    }

    fn save_with(&mut self, turn: u64, calibration: f64, turns_since_plan: u32) {
        let Some(dir) = self.dir.clone() else { return };
        let mut messages = self.messages[1.min(self.messages.len())..].to_vec();
        compaction::evict_images(&mut messages, 0);
        let saved = Saved {
            version: VERSION,
            // ⚠️ **The prompt in force *now*, not the one that was restored.** This is what makes
            // the warning fire once per change rather than once per restart after one.
            system_prompt: prompt::SYSTEM_PROMPT.to_string(),
            run_id: crate::run::directory_name(&dir),
            model: std::env::var("GB_MODEL").unwrap_or_default(),
            saved_at: crate::run::iso8601(std::time::SystemTime::now()),
            turn,
            calibration,
            turns_since_plan,
            messages,
        };
        let bytes = match serde_json::to_vec(&saved) {
            Ok(bytes) => bytes,
            Err(e) => return eprintln!("could not serialise the conversation: {e}"),
        };
        if let Err(e) = crate::run::write_atomically(&dir.join(files::HISTORY), &bytes) {
            eprintln!("could not save the conversation: {e}");
        }
    }

    fn append_line(&mut self, value: &serde_json::Value) {
        let Some(dir) = self.dir.as_deref() else { return };
        let path = dir.join(files::CONVERSATION);
        let Ok(line) = serde_json::to_string(value) else { return };
        if let Err(e) = append(&path, &line) {
            eprintln!("could not write to the conversation log: {e}");
        }
    }
}

/// Append one line, rotating first if the file has grown past the transcript's own limit.
///
/// The log is flushed per line for the reason the transcript is: `hall_of_fame::archive` reads it
/// from another thread while this one is still writing, and it reads whole lines.
fn append(path: &Path, line: &str) -> Result<(), String> {
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) >= crate::run::transcript::MAX_BYTES {
        crate::run::transcript::rotate(path)?;
    }
    let mut file = crate::run::transcript::open_append(path)?;
    writeln!(file, "{line}").map_err(|e| format!("could not append to {}: {e}", path.display()))?;
    file.flush().map_err(|e| format!("could not flush {}: {e}", path.display()))
}

/// Read the saved conversation back, or `None` if there is not a usable one there.
fn read_saved(dir: &Path) -> Option<(Saved, Vec<Message>)> {
    let path = dir.join(files::HISTORY);
    let bytes = std::fs::read(&path).ok()?;
    let saved: Saved = match serde_json::from_slice(&bytes) {
        Ok(saved) => saved,
        Err(e) => {
            eprintln!("{} could not be read ({e}); starting a fresh conversation", path.display());
            return None;
        }
    };
    if saved.version != VERSION {
        eprintln!(
            "{} was written by version {} and this build reads {VERSION}; starting a fresh conversation",
            path.display(),
            saved.version
        );
        return None;
    }
    let messages = sanitise(saved.messages.clone());
    match messages.is_empty() {
        true => None,
        false => Some((saved, messages)),
    }
}

/// Everything that has to be true of a history before it is put back in front of an endpoint.
///
/// Three passes, each guarding a different way a stored conversation can be rejected for the rest of
/// the run rather than for one request.
fn sanitise(messages: Vec<Message>) -> Vec<Message> {
    // A stored system message would end up second, behind the one `open` prepends. Nothing we write
    // produces one; a hand-edited or older file might.
    let mut messages: Vec<Message> = messages.into_iter().skip_while(|m| m.role == Role::System).collect();

    // ⚠️ The `serde` route into a `Message` walks straight past `Message::assistant`, which is where
    // `history_safe` lives. See its doc comment: one stored call whose `arguments` are not a JSON
    // object 400s *every* request from then on, because it is re-sent with the whole history.
    for message in &mut messages {
        if !message.tool_calls.is_empty() {
            message.tool_calls = crate::llm::protocol::history_safe(std::mem::take(&mut message.tool_calls));
        }
    }

    // ⚠️ A history whose last assistant message asks for tools nobody answered is rejected outright
    // by a strict endpoint. Our own writer cannot produce one — every checkpoint is taken with the
    // turn's messages complete — so this is guarding the file rather than the code, and it is worth
    // its twenty lines because the failure is permanent rather than transient.
    let mut end = messages.len();
    for (index, message) in messages.iter().enumerate() {
        if message.tool_calls.is_empty() {
            continue;
        }
        let answered = messages[index + 1..]
            .iter()
            .take_while(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect::<Vec<_>>();
        if message.tool_calls.iter().any(|call| !answered.contains(&call.id.as_str())) {
            end = index;
            break;
        }
    }
    messages.truncate(end);

    // A conversation may not open on an answer to something that is no longer there.
    let start = messages
        .iter()
        .position(|m| m.role != Role::Tool && m.role != Role::Assistant)
        .unwrap_or(messages.len());
    messages.drain(..start);
    messages
}

/// The worker holds a `History` where it used to hold a `Vec<Message>`, and reads it as one.
///
/// Sound *here* specifically because persistence is checkpoint-based rather than write-through:
/// nothing has to intercept a mutation, only observe the vector once a turn. A write-through design
/// could not expose `DerefMut` at all.
impl Deref for History {
    type Target = Vec<Message>;

    fn deref(&self) -> &Self::Target {
        &self.messages
    }
}

impl DerefMut for History {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::protocol::{FunctionCall, ImageDetail, ToolCall};
    use crate::run::tests::Scratch;

    fn call(id: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall { name: "read_map".into(), arguments: arguments.into() },
        }
    }

    /// Two turns' worth of a perfectly ordinary conversation.
    fn conversation(history: &mut History) {
        history.push(Message::user("### Turn 1\nYou are in Pallet Town."));
        history.push(Message::assistant(String::new(), vec![call("c1", r#"{"summary":"look"}"#)]));
        history.push(Message::tool_result("c1", "the map".to_string()));
        history.push(Message::user("### Turn 2\nYou are on Route 1."));
    }

    fn log_lines(dir: &Path) -> Vec<serde_json::Value> {
        let text = std::fs::read_to_string(dir.join(files::CONVERSATION)).expect("a log");
        text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).expect("json")).collect()
    }

    /// Half (A). A conversation written by one process is what the next one opens on.
    #[test]
    fn a_conversation_reopened_from_disk_starts_where_it_left_off() {
        let scratch = Scratch::new("history-roundtrip");
        let mut first = History::open(Some(&scratch.0));
        assert!(first.restored().is_none(), "an empty directory restores nothing");
        conversation(&mut first);
        first.checkpoint(1, 1.0, 0);
        let before = first.len();
        drop(first);

        // The precondition: the file holds everything *except* the system prompt, or the assertion
        // below would pass on a file that stored index 0 as well.
        let saved: Saved =
            serde_json::from_slice(&std::fs::read(scratch.0.join(files::HISTORY)).expect("a file")).expect("json");
        assert_eq!(saved.messages.len(), before - 1, "the system prompt is not on disk");

        let second = History::open(Some(&scratch.0));
        let restored = second.restored().expect("the conversation came back");
        assert_eq!(restored.messages, before - 1);
        assert_eq!(second[0].role, Role::System, "index 0 is still the system prompt");
        assert_eq!(second[1].text(), Some("### Turn 1\nYou are in Pallet Town."));
        assert_eq!(second[3].tool_call_id.as_deref(), Some("c1"), "the tool result came back with its call");
    }

    /// ⚠️ The deployment case. Message 0 is re-minted rather than restored, so an edit to the system
    /// prompt reaches a run that is resumed across it instead of being pinned by the old file.
    #[test]
    fn the_system_prompt_is_rebuilt_rather_than_restored_so_an_edit_reaches_the_model() {
        let scratch = Scratch::new("history-prompt");
        let stale = "You are playing something else entirely.";
        let saved = serde_json::json!({
            "version": VERSION,
            "messages": [
                { "role": "system", "content": stale },
                { "role": "user", "content": "### Turn 1" },
            ],
        });
        std::fs::write(scratch.0.join(files::HISTORY), saved.to_string()).expect("write");
        // The precondition: the file really does carry an old system prompt.
        assert!(std::fs::read_to_string(scratch.0.join(files::HISTORY)).unwrap().contains(stale));

        let history = History::open(Some(&scratch.0));
        assert_eq!(history[0], prompt::system_message(), "index 0 is this build's prompt");
        assert!(
            !history.iter().any(|m| m.text().is_some_and(|t| t.contains(stale))),
            "the stored prompt is gone rather than sitting second"
        );
        assert_eq!(history.iter().filter(|m| m.role == Role::System).count(), 1);
    }

    /// ⚠️ The `ImageUrl::tokens` trap. `tokens` is `#[serde(skip)]` and defaults to 85, so a picture
    /// that round-tripped would be priced at a twentieth of its weight and a full context would read
    /// as nearly empty for the rest of the run. Nothing is stored, so nothing can be mis-defaulted.
    #[test]
    fn a_restored_history_carries_no_pictures_and_no_mispriced_image_tokens() {
        let scratch = Scratch::new("history-images");
        let mut history = History::open(Some(&scratch.0));
        history.push(Message::user("### Turn 1"));
        history.push(Message::user_with_image_detail(
            "the map of Route 2",
            "data:image/png;base64,AAAA".to_string(),
            ImageDetail::High,
            3825,
        ));
        // The precondition: this really is an expensive picture before it goes anywhere near disk.
        assert!(history.last().unwrap().has_image());
        assert!(history.last().unwrap().approximate_tokens() >= 3825, "a map is not an 85-token thumbnail");
        history.checkpoint(1, 1.0, 0);
        drop(history);

        let text = std::fs::read_to_string(scratch.0.join(files::HISTORY)).expect("a file");
        assert!(!text.contains("data:image"), "no base64 reaches the disk");

        let restored = History::open(Some(&scratch.0));
        assert!(!restored.iter().any(Message::has_image), "nothing comes back as an image");
        // ⚠️ Not `last()`: a restored conversation ends in `RESUMED_NOTE`. The picture is the
        // message before it.
        let caption = restored[restored.len() - 2].text().expect("text");
        assert!(caption.starts_with("the map of Route 2"), "the caption survives: {caption}");
        assert!(caption.ends_with(compaction::EVICTED), "and says the picture went: {caption}");
    }

    /// ⚠️ `serde` walks straight past `Message::assistant`, where `history_safe` lives. One stored
    /// call whose arguments are not a JSON object 400s *every* request for the rest of the run.
    #[test]
    fn a_tool_call_the_model_wrote_badly_is_still_repaired_when_it_comes_back_off_disk() {
        let scratch = Scratch::new("history-badcall");
        let saved = serde_json::json!({
            "version": VERSION,
            "messages": [
                { "role": "user", "content": "### Turn 1" },
                { "role": "assistant", "tool_calls": [
                    { "id": "c1", "type": "function",
                      "function": { "name": "read_map", "arguments": "{\"broken\": " } }
                ] },
                { "role": "tool", "tool_call_id": "c1", "content": "answered" },
            ],
        });
        std::fs::write(scratch.0.join(files::HISTORY), saved.to_string()).expect("write");
        // The precondition: the fragment really is on disk, so the repair below is doing something.
        assert!(std::fs::read_to_string(scratch.0.join(files::HISTORY)).unwrap().contains(r#"{\"broken\": "#));

        let history = History::open(Some(&scratch.0));
        let assistant = history.iter().find(|m| !m.tool_calls.is_empty()).expect("the call survives");
        assert_eq!(assistant.tool_calls.len(), 1, "repaired in place, never dropped");
        assert_eq!(assistant.tool_calls[0].function.arguments, "{}");
        // ⚠️ Dropping the call instead would orphan this result, which is the *other* way to 400.
        assert_eq!(assistant.tool_calls[0].id, "c1");
        assert!(history.iter().any(|m| m.tool_call_id.as_deref() == Some("c1")), "its answer is still paired");
    }

    /// A history ending in tool calls nobody answered is rejected outright by a strict endpoint. Our
    /// own writer cannot produce one, so this guards the *file* rather than the code.
    #[test]
    fn a_history_whose_last_assistant_was_never_answered_is_rolled_back_on_open() {
        let scratch = Scratch::new("history-unpaired");
        let saved = serde_json::json!({
            "version": VERSION,
            "messages": [
                { "role": "user", "content": "### Turn 1" },
                { "role": "assistant", "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "read_map", "arguments": "{}" } }
                ] },
            ],
        });
        std::fs::write(scratch.0.join(files::HISTORY), saved.to_string()).expect("write");
        // The precondition: the stored history really does end on an unanswered call.
        let stored: Saved =
            serde_json::from_slice(&std::fs::read(scratch.0.join(files::HISTORY)).unwrap()).unwrap();
        assert!(!stored.messages.last().unwrap().tool_calls.is_empty());

        let history = History::open(Some(&scratch.0));
        for message in history.iter() {
            for call in &message.tool_calls {
                assert!(
                    history.iter().any(|m| m.tool_call_id.as_deref() == Some(call.id.as_str())),
                    "every surviving call has its answer"
                );
            }
        }
        assert_eq!(history.last().unwrap().text(), Some(prompt::RESUMED_NOTE), "what is left still restores");
    }

    /// Losing the conversation is bad; refusing to play because of it is worse.
    #[test]
    fn an_unreadable_history_is_a_fresh_conversation_rather_than_a_dead_run() {
        let scratch = Scratch::new("history-garbage");
        std::fs::write(scratch.0.join(files::HISTORY), b"\x00not json at all").expect("write");
        let mut history = History::open(Some(&scratch.0));
        assert!(history.restored().is_none());
        assert_eq!(history.len(), 1, "just the system prompt");
        // And the run carries on writing.
        history.push(Message::user("### Turn 1"));
        history.checkpoint(1, 1.0, 0);
        assert!(History::open(Some(&scratch.0)).restored().is_some(), "the next process resumes normally");
    }

    /// Half (B), the headline: what a compaction destroys is still on disk afterwards.
    #[test]
    fn every_message_ever_appended_is_in_the_log_even_after_a_compaction_drops_it() {
        let scratch = Scratch::new("history-compact");
        let mut history = History::open(Some(&scratch.0));
        let doomed = "a sentence only the dropped middle contains";
        history.push(Message::user(format!("### Turn 1\n{doomed}")));
        history.push(Message::assistant("thinking".into(), vec![]));
        for turn in 2..8 {
            history.push(Message::user(format!("### Turn {turn}\nordinary")));
        }
        history.checkpoint(1, 1.0, 0);

        let before = history.len();
        compaction::apply_summary(&mut history, "the story so far", 2);
        history.note_compaction(8, &CompactionNote {
            before: 900,
            after: 100,
            images_evicted: 0,
            dropped: before - history.len(),
            summary: Some("the story so far".into()),
        });
        history.checkpoint(8, 1.0, 0);

        // ⚠️ **The precondition is the whole test.** Without it this passes on a compaction that
        // never dropped anything, which is exactly the shape that proves nothing.
        assert!(
            !history.iter().any(|m| m.text().is_some_and(|t| t.contains(doomed))),
            "the live history really has lost it"
        );
        assert!(
            !std::fs::read_to_string(scratch.0.join(files::HISTORY)).unwrap().contains(doomed),
            "and so has what the next process would resume on"
        );

        let logged = std::fs::read_to_string(scratch.0.join(files::CONVERSATION)).expect("a log");
        assert!(logged.contains(doomed), "but the log still has it");
    }

    /// The marker line says a compaction happened, and the summary that replaced the middle goes in
    /// as an ordinary message so a reader filtering on `kind == "message"` still sees everything.
    #[test]
    fn the_log_records_a_compaction_and_the_summary_it_kept() {
        let scratch = Scratch::new("history-marker");
        let mut history = History::open(Some(&scratch.0));
        conversation(&mut history);
        history.checkpoint(1, 1.0, 0);
        history.note_compaction(2, &CompactionNote {
            before: 900,
            after: 100,
            images_evicted: 3,
            dropped: 4,
            summary: Some("the story so far".into()),
        });

        let lines = log_lines(&scratch.0);
        let marker = lines.iter().find(|l| l["kind"] == "compaction").expect("a marker line");
        assert_eq!(marker["before"], 900);
        assert_eq!(marker["after"], 100);
        assert!(marker["after"].as_u64() < marker["before"].as_u64(), "a compaction that shrank it");
        assert_eq!(marker["images_evicted"], 3);
        assert_eq!(marker["dropped"], 4);
        assert_eq!(marker["summarised"], true);

        let summary = lines
            .iter()
            .filter(|l| l["kind"] == "message")
            .find(|l| l["message"]["content"].as_str().is_some_and(|t| t.contains(compaction::SUMMARY_HEADING)))
            .expect("the summary is logged as a message");
        assert_eq!(summary["turn"], 2);
    }

    /// ⚠️ `fresh` clears and `open` restores, and `apply_restart` must call the first one: a new run
    /// reading the conversation back would inherit the dead game's memory.
    #[test]
    fn a_new_run_clears_the_conversation_instead_of_reloading_it() {
        let scratch = Scratch::new("history-newrun");
        let mut first = History::open(Some(&scratch.0));
        first.push(Message::user("### Turn 1\nthe old game"));
        first.checkpoint(1, 1.0, 0);
        drop(first);
        // The precondition: there really is a conversation there to be wrongly reloaded.
        assert!(std::fs::read_to_string(scratch.0.join(files::HISTORY)).unwrap().contains("the old game"));

        let fresh = History::fresh(Some(&scratch.0));
        assert!(fresh.restored().is_none());
        assert_eq!(fresh.len(), 1, "the system prompt and nothing else");
        assert!(
            !std::fs::read_to_string(scratch.0.join(files::HISTORY)).unwrap().contains("the old game"),
            "and it wrote its empty state out, so no window exists where the file describes another run"
        );
    }

    /// ⚠️ The skew half (A) creates: `state.gbst` is the last checkpoint and `history.json` is the
    /// last turn, so a restored conversation can be ahead of the game. Saying so once is what stops
    /// the model concluding the game is broken.
    #[test]
    fn a_resumed_run_tells_the_model_the_game_may_be_behind_the_conversation() {
        let scratch = Scratch::new("history-note");
        let mut first = History::open(Some(&scratch.0));
        // The precondition: a conversation that started here is told nothing.
        assert!(!first.iter().any(|m| m.text() == Some(prompt::RESUMED_NOTE)), "nothing to explain yet");
        conversation(&mut first);
        first.checkpoint(1, 1.0, 0);
        drop(first);

        let second = History::open(Some(&scratch.0));
        assert_eq!(second.last().expect("a tail").text(), Some(prompt::RESUMED_NOTE), "and it is at the tail");
        assert!(compaction::is_turn_start(second.last().unwrap()), "a legal cut point, so it can be compacted away");
    }

    /// The calibration is the one number that has to survive, and it comes back with the messages.
    #[test]
    fn the_endpoints_measure_of_the_context_survives_the_restart_with_it() {
        let scratch = Scratch::new("history-calibration");
        let mut first = History::open(Some(&scratch.0));
        conversation(&mut first);
        first.checkpoint(7, 2.75, 4);
        drop(first);

        let restored = History::open(Some(&scratch.0)).restored().expect("restored").clone();
        assert_eq!(restored.calibration, 2.75);
        assert_eq!(restored.turns_since_plan, 4);
    }

    /// ⚠️ **A restore that never reaches a turn must not walk the calibration back to 1.0.** The
    /// save taken at construction is the one nobody thinks about, because the first turn's
    /// checkpoint overwrites it seconds later — but a process that dies before finishing a turn
    /// leaves it as the file, and a run restarting under a crash loop would lose the endpoint's
    /// measure of its own context for good.
    #[test]
    fn a_restart_that_finishes_no_turn_still_leaves_the_calibration_where_it_found_it() {
        let scratch = Scratch::new("history-crashloop");
        let mut first = History::open(Some(&scratch.0));
        conversation(&mut first);
        first.checkpoint(9, 2.75, 3);
        drop(first);

        // A process that opens the run and then dies: no checkpoint, only the construction save.
        drop(History::open(Some(&scratch.0)));

        let saved: Saved =
            serde_json::from_slice(&std::fs::read(scratch.0.join(files::HISTORY)).expect("a file")).expect("json");
        assert_eq!(saved.calibration, 2.75, "the calibration survived a process that did nothing");
        assert_eq!(saved.turns_since_plan, 3);
        assert_eq!(saved.turn, 9);
        // And it is still there for the process after that.
        assert_eq!(History::open(Some(&scratch.0)).restored().expect("restored").calibration, 2.75);
    }

    /// ⚠️ **A deployment that edits the system prompt must have the edit take effect on the next
    /// restart, out loud.** The conversation is kept and index 0 is replaced, which is the whole
    /// point: a run that is resumed for a week would otherwise be held under the prompt it started
    /// with for ever.
    #[test]
    fn a_system_prompt_that_changed_under_a_restart_is_replaced_and_said_out_loud() {
        let scratch = Scratch::new("history-promptchange");
        // A conversation saved under a *different* prompt, which is what a deployment looks like
        // from the next process's side.
        let saved = serde_json::json!({
            "version": VERSION,
            "system_prompt": "You are playing something else entirely.",
            "messages": [{ "role": "user", "content": "### Turn 1" }],
        });
        std::fs::write(scratch.0.join(files::HISTORY), saved.to_string()).expect("write");

        let history = History::open(Some(&scratch.0));
        let restored = history.restored().expect("the conversation is kept");
        assert!(restored.system_prompt_changed, "and the change is reported");
        assert_eq!(restored.messages, 1, "⚠️ kept, not discarded — a new prompt is not a new run");
        assert_eq!(history[0], prompt::system_message(), "the new prompt is in force");

        // It is in the log too, so a reader of the archive can see where it changed.
        let lines = log_lines(&scratch.0);
        assert!(lines.iter().any(|l| l["kind"] == "system_prompt_changed"), "{lines:#?}");

        // ⚠️ **And it fires once per change, not once per restart after one.** The save taken at
        // construction records the prompt now in force, so the next process finds them equal.
        drop(history);
        assert!(
            !History::open(Some(&scratch.0)).restored().expect("restored").system_prompt_changed,
            "the second restart under the same prompt is silent",
        );
    }

    /// The other half, and the one that would make the warning useless: an unchanged prompt must not
    /// report a change, or every restart cries wolf.
    #[test]
    fn an_unchanged_system_prompt_is_not_reported_as_a_change() {
        let scratch = Scratch::new("history-promptsame");
        let mut first = History::open(Some(&scratch.0));
        conversation(&mut first);
        first.checkpoint(1, 1.0, 0);
        drop(first);

        let restored = History::open(Some(&scratch.0)).restored().expect("restored").clone();
        assert!(!restored.system_prompt_changed);

        // ⚠️ A file predating the field reads as "not recorded" rather than "changed": the field is
        // `#[serde(default)]` so an older file is still readable, and warning about one would be a
        // warning about nothing.
        let text = std::fs::read_to_string(scratch.0.join(files::HISTORY)).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("system_prompt");
        std::fs::write(scratch.0.join(files::HISTORY), value.to_string()).unwrap();
        assert!(
            !History::open(Some(&scratch.0)).restored().expect("restored").system_prompt_changed,
            "a file that never recorded a prompt has not changed one",
        );
    }

    /// ⚠️ **Stored to be compared, never to be restored**, and the distance between the two is one
    /// line of code. Restoring it would undo the re-minting that makes a deployment's edit land.
    #[test]
    fn the_stored_system_prompt_is_compared_and_never_restored() {
        let scratch = Scratch::new("history-promptonce");
        let stale = "You are playing something else entirely.";
        let saved = serde_json::json!({
            "version": VERSION,
            "system_prompt": stale,
            "messages": [{ "role": "user", "content": "### Turn 1" }],
        });
        std::fs::write(scratch.0.join(files::HISTORY), saved.to_string()).expect("write");
        // The precondition: the stale prompt really is in the file being read.
        assert!(std::fs::read_to_string(scratch.0.join(files::HISTORY)).unwrap().contains(stale));

        let history = History::open(Some(&scratch.0));
        assert_eq!(history.iter().filter(|m| m.role == Role::System).count(), 1, "exactly one");
        assert!(
            !history.iter().any(|m| m.text().is_some_and(|t| t.contains(stale))),
            "the stored prompt reached the comparison and not the conversation",
        );
    }

    /// The log is a record of what the model was *sent*, so it carries the system prompt — and a
    /// process that changed it logs the new one under its own header. ⚠️ Reading the file back as
    /// one conversation therefore shows the prompt changing partway through, which is the honest
    /// picture rather than a glitch.
    #[test]
    fn the_log_carries_the_system_prompt_each_process_actually_used() {
        let scratch = Scratch::new("history-logprompt");
        let mut first = History::open(Some(&scratch.0));
        conversation(&mut first);
        first.checkpoint(1, 1.0, 0);
        drop(first);
        drop(History::open(Some(&scratch.0)));

        let lines = log_lines(&scratch.0);
        let prompts: Vec<_> = lines
            .iter()
            .filter(|l| l["kind"] == "message" && l["message"]["role"] == "system")
            .collect();
        assert_eq!(prompts.len(), 2, "one per process, not one per run: {}", lines.len());
        for logged in prompts {
            assert_eq!(logged["message"]["content"], prompt::SYSTEM_PROMPT);
        }
        // ⚠️ And the conversation itself is not re-logged on the restart, or a run restarted nightly
        // would multiply its own log.
        assert_eq!(
            lines
                .iter()
                .filter(|l| {
                    l["kind"] == "message"
                        && l["message"]["content"].as_str().is_some_and(|t| t.starts_with("### Turn 1"))
                })
                .count(),
            1,
            "the restored middle was written once, by the process that produced it",
        );
    }

    /// A file from another build is a fresh conversation, not a guess at what it meant.
    #[test]
    fn a_history_written_by_another_version_is_not_guessed_at() {
        let scratch = Scratch::new("history-version");
        let saved = serde_json::json!({
            "version": VERSION + 1,
            "messages": [{ "role": "user", "content": "### Turn 1\nfrom the future" }],
        });
        std::fs::write(scratch.0.join(files::HISTORY), saved.to_string()).expect("write");
        let history = History::open(Some(&scratch.0));
        assert!(history.restored().is_none());
        assert!(!history.iter().any(|m| m.text().is_some_and(|t| t.contains("from the future"))));
    }

    /// A history with nowhere to live still works — `--policy random` and every test rig.
    #[test]
    fn a_conversation_with_no_run_directory_still_plays_it_just_forgets() {
        let mut history = History::open(None);
        conversation(&mut history);
        history.checkpoint(1, 1.0, 0);
        assert!(history.restored().is_none());
        assert_eq!(history.len(), 5);
    }
}
