//! What the model did when it reached past the action menu and pressed buttons itself.
//!
//! `press_buttons` is the escape hatch — raw joypad presses delivered ahead of the whole state
//! machine, for the case where the action menu, which is the *agent's model of the game* rather than
//! the game, is incomplete. It resets the agent to idle afterwards, so a model that reaches for it
//! instead of choosing an action walks the player into a wall. On the deployed run it was being
//! reached for on ordinary turns that had a perfectly good menu.
//!
//! Prose alone could not fix that, because prose cannot be checked afterwards. This writes one
//! directory per press:
//!
//! ```text
//! $GB_RUN_DIR/<run-id>/press-buttons/turn-<id>/
//!   ├── incident.json   the reason, the decision, the run's whole state, and the conversation
//!   └── screen.png      what was actually on the screen at the moment of the press
//! ```
//!
//! ⚠️ **A `Stuck` record is not a fault.** The watchdog's turn offers `press_buttons` and `wait` and
//! nothing else — there is no menu to prefer — so a press there is the model doing exactly as it was
//! asked. Everything is recorded and the `kind` field is what tells the two apart; filtering here
//! would mean the one number worth knowing (how often the hatch is used, and how often it was
//! needed) could not be counted.
//!
//! Three things this deliberately does **not** do.
//!
//! ⚠️ **It carries no picture on an event and publishes nothing new.** A screenshot is a couple of
//! kilobytes base64'd and every published event is also a line of `transcript.jsonl` broadcast to
//! every open page; that is the same arithmetic that keeps tool pictures in a ring in
//! [`Published`](crate::web::published::Published) instead of on the event. The page already shows
//! the press, as `Pressed a, b`, off the `Decision` event it was always going to publish.
//!
//! ⚠️ **It evicts images from the conversation slice, and that is not an optimisation.** A history
//! holding a map render is hundreds of kilobytes of base64 *per message*, and a model that has
//! decided to press buttons is quite likely to do it again on the next turn.
//!
//! ⚠️ **It re-reads the run directory from [`CurrentRun`] every time**, never keeping a path. This
//! is the trap `transcript.rs` documents: a captured `PathBuf` keeps writing into the run that
//! `POST /api/new-run` has already checkpointed and set aside.
//!
//! This is the first PNG the program writes to disk. Everything else it draws — badges, party
//! sprites, map renders, tool pictures — is served from memory over HTTP and never lands anywhere.

use std::path::PathBuf;

use serde::Serialize;

use crate::joypad::JoypadButton;
use crate::llm::compaction;
use crate::llm::protocol::Message;
use crate::llm::screenshot;
use crate::llm::tools::DecisionKind;
use crate::run::{self, CurrentRun, files};
use crate::web::published::{Published, UiEvent, now_ms};

/// How many turns of conversation a record carries.
///
/// Enough to see the decision, what was read to reach it, and the turn before that — which is
/// usually where the thing the model was actually trying to do is stated. The whole history is the
/// obvious alternative and is the wrong one: near `GB_COMPACT_ABOVE` it is a hundred thousand tokens
/// of JSON, written again on every press.
const TURNS_KEPT: usize = 3;

/// `incident.json`. Everything a person needs to answer "should this have been an action?" without
/// opening anything else.
#[derive(Debug, Serialize)]
struct Incident<'a> {
    /// Unix milliseconds, the same clock and for the same reason as [`UiEvent::at`]: a run resumed
    /// nightly restarts every elapsed counter it has, so this is the only stamp that can date a
    /// record against the transcript beside it.
    at: u64,
    run_id: String,
    turn: u64,
    /// ⚠️ `"stuck"` here means the watchdog asked, which is the one turn where a press is correct.
    kind: &'static str,
    buttons: Vec<String>,
    /// The model's answer to "which action could not do this". `None` means it did not say, which
    /// the schema asks for and the parser does not enforce — see `tools::call_reason`.
    why: Option<&'a str>,
    /// The model's `summary`: what it thought it was doing, in its own words.
    summary: Option<&'a str>,
    /// The last status heartbeat, whole. Carries the agent's own state string alongside the map,
    /// position, mode, party, badges, money and play time, so nothing here is re-derived.
    status: Option<UiEvent>,
    /// The last [`TURNS_KEPT`] turns, with every image replaced by its caption.
    conversation: Vec<Message>,
}

/// Write the record, and answer where it went.
///
/// ⚠️ **Every failure is the caller's to shrug at.** This runs on the turn loop's thread on the way
/// to handing a decision back to the game; a full disk must cost a log line, never a turn.
#[allow(clippy::too_many_arguments)]
pub fn record(
    run: &CurrentRun,
    published: &Published,
    turn: u64,
    kind: DecisionKind,
    buttons: &[JoypadButton],
    why: Option<&str>,
    summary: Option<&str>,
    messages: &[Message],
) -> Result<PathBuf, String> {
    let run = run.get();
    let parent = run.path().join(files::PRESS_BUTTONS);
    std::fs::create_dir_all(&parent).map_err(|e| format!("could not create {parent:?}: {e}"))?;
    // ⚠️ A turn id is the worker's cancellation generation and restarts with the process, so a run
    // resumed twice in a day would otherwise overwrite the first record with the second.
    let dir = parent.join(run::unique_dir(&parent, &format!("turn-{turn}")));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;

    let incident = Incident {
        at: now_ms(),
        run_id: run.run_id(),
        turn,
        kind: kind.label(),
        // Lower-cased so the record spells a button the way the model's own call did — the schema's
        // enum is `"start"`, and a record grepped for what was sent should find it.
        buttons: buttons.iter().map(|button| button.to_string().to_lowercase()).collect(),
        why,
        summary,
        status: published.latest_status(),
        conversation: recent_turns(messages),
    };
    let json = serde_json::to_vec_pretty(&incident)
        .map_err(|e| format!("could not serialise the record: {e}"))?;
    // ⚠️ `write_atomically` stages at `with_extension("tmp")`, which *replaces* the extension — so
    // these two stage as `incident.tmp` and `screen.tmp`. Distinct, and inside a directory of their
    // own, so two records cannot collide however close together they land.
    run::write_atomically(&dir.join("incident.json"), &json)?;

    let frame = published.latest_frame();
    run::write_atomically(&dir.join("screen.png"), &screenshot::encode(&frame.pixels))?;
    Ok(dir)
}

/// The tail of the history, cut at a turn boundary and stripped of pictures.
///
/// ⚠️ **The boundary is `compaction::is_turn_start`**, the same one the compactor cuts on and the
/// same one the plan message is deliberately excluded from — so a slice never begins halfway through
/// a turn, with a tool result whose call is not in the record.
fn recent_turns(messages: &[Message]) -> Vec<Message> {
    let starts = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| compaction::is_turn_start(message))
        .map(|(index, _)| index);
    let from = starts.rev().nth(TURNS_KEPT - 1).unwrap_or(0);
    let mut slice = messages[from..].to_vec();
    // `keep: 0` — every picture, not the oldest few.
    compaction::evict_images(&mut slice, 0);
    slice
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::protocol::Role;
    use crate::run::RunDir;
    use crate::run::tests::Scratch;

    fn current(scratch: &Scratch) -> CurrentRun {
        let (run, _, _) =
            RunDir::open(&scratch.0, true, "test-model", &|_| false).expect("a fresh run directory");
        CurrentRun::new(scratch.0.clone(), "test-model".to_string(), run)
    }

    /// Four turns in, with a picture in the middle of the last one.
    fn history() -> Vec<Message> {
        let mut messages = vec![Message::system("system")];
        for turn in 0..4 {
            messages.push(Message::user(format!("### Turn {turn}")));
            messages.push(Message::assistant(String::new(), vec![]));
        }
        messages.push(Message::user_with_image(
            "a screenshot",
            "data:image/png;base64,AAAA".to_string(),
        ));
        messages
    }

    /// The whole of it: both files land, the conversation is cut to the last three turns, and no
    /// picture survives into the JSON.
    ///
    /// ⚠️ The `data:` assertion is the load-bearing one. Without the eviction a record is a base64
    /// map render per message, written again on every press — and a test that only counted messages
    /// would pass all the way through that regression.
    #[test]
    fn a_press_is_recorded_with_its_screen_and_a_picture_free_conversation() {
        let scratch = Scratch::new("incident");
        let run = current(&scratch);
        let published = Published::new();

        let dir = record(
            &run,
            &published,
            7,
            DecisionKind::Overworld,
            &[JoypadButton::Start, JoypadButton::A],
            Some("no action opens the START menu"),
            Some("checking the bag"),
            &history(),
        )
        .expect("the record writes");

        let json = std::fs::read_to_string(dir.join("incident.json")).expect("incident.json");
        assert!(dir.join("screen.png").exists(), "the screen is half of what a record is for");
        assert!(!json.contains("data:image"), "a picture must never reach the record");
        assert!(json.contains("[image removed to save context]"), "the caption stays");
        assert!(json.contains("no action opens the START menu"), "the reason is the headline");
        assert!(json.contains("\"kind\": \"overworld\""), "a Stuck press has to be tellable apart");
        // Spelled the model's way, not `strum`'s `Display`.
        assert!(json.contains("\"start\"") && json.contains("\"a\""), "the buttons that were pressed");

        let incident: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let conversation = incident["conversation"].as_array().expect("a conversation");
        // Three turn starts, their assistant replies, and the trailing picture message.
        assert_eq!(conversation.len(), 7, "{conversation:#?}");
        assert_eq!(conversation[0]["content"], "### Turn 1", "cut at a turn boundary");
    }

    /// A second press on the same turn id — a run resumed twice in a day is the real case — must not
    /// overwrite the first record.
    #[test]
    fn two_records_of_one_turn_id_do_not_overwrite_each_other() {
        let scratch = Scratch::new("incident-collide");
        let run = current(&scratch);
        let published = Published::new();
        let write = || {
            record(&run, &published, 1, DecisionKind::Stuck, &[JoypadButton::A], None, None, &[])
                .expect("the record writes")
        };

        let first = write();
        let second = write();
        assert_ne!(first, second);
        assert!(first.exists() && second.exists());
    }

    /// A history shorter than [`TURNS_KEPT`] is the first minutes of every run, and an empty one is
    /// what the second test above passes. Neither may panic on the slice arithmetic.
    #[test]
    fn a_short_history_is_carried_whole() {
        assert!(recent_turns(&[]).is_empty());
        let messages = vec![Message::system("system"), Message::user("### Turn 0")];
        assert_eq!(recent_turns(&messages).len(), 2, "nothing is dropped from below the window");
    }

    /// ⚠️ **A slice may never open on an assistant message or a tool result.** A record is read to
    /// work out why a press happened, and one that begins with a reply to a question it does not
    /// contain — or with the answer to a call that is not in it — is a record of nothing. That is
    /// what cutting on `compaction::is_turn_start` buys, and it is invisible if the only assertion
    /// is a length.
    #[test]
    fn the_slice_starts_at_a_user_message() {
        let slice = recent_turns(&history());
        assert_eq!(slice[0].role, Role::User);
    }
}
