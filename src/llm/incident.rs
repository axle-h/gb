//! What the model says, and does, when it believes the agent is wrong.
//!
//! Two things are filed here and they are the same shape on disk:
//!
//! ```text
//! $GB_RUN_DIR/<run-id>/issues/turn-<id>/          a `report_issue` call
//! $GB_RUN_DIR/<run-id>/press-buttons/turn-<id>/   a press at the watchdog's turn
//!   ├── incident.json   the report, the decision, the run's whole state, and the conversation
//!   ├── screen.png      what was actually on the screen
//!   └── state.gbst      the machine as the turn found it, to replay from — see the ⚠️ below
//! ```
//!
//! **`report_issue`** is the interesting one. It does not end the turn: the model files the
//! complaint and then still has to choose an action, so filing one and playing on are no longer
//! mutually exclusive. That is the whole reason it replaced the escape hatch on every turn that has
//! a menu — see [`crate::llm::tools::report_issue_spec`].
//!
//! **A press** now only ever happens at [`DecisionKind::Stuck`], where the agent has reached no
//! decision point at all and there is no menu to prefer, so it is the model doing exactly as it was
//! asked rather than a fault. The `kind` field still records which turn asked, because the two were
//! worth telling apart when the hatch was offered everywhere and a record is worth reading either
//! way.
//!
//! ⚠️ **`state.gbst` is the machine as it was when *this turn* was put to the model**, not the last
//! periodic checkpoint. It is captured by `EmulatorHost::tick` on the edge into `AwaitingLlm` and
//! left in `Published`, because `GameBoy` exists on the emulator thread and this one has no way to
//! ask for a state when it wants one. The first draft copied `state.gbst` out of the run directory
//! instead and was up to a checkpoint interval — a minute — behind, which is a minute of walking,
//! several battles, or the very transition being complained about. A state costs 24 µs and 6.4 KB
//! (measured on Pokémon Red, 2026-08-25), so taking one per turn is cheaper than the copy it
//! replaced. `state_captured_at` carries the moment it was taken, and the gap between that and `at`
//! is how long the model spent on the turn.
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

/// What is being filed. The directory differs and so do two fields of the JSON; everything else — the
/// screen, the save state, the status heartbeat, the conversation slice — is identical, because the
/// question a person opens either one to answer is the same question.
#[derive(Debug, Clone, Copy)]
pub enum Report<'a> {
    /// A `report_issue` call: the model's own account of what the agent will not let it do.
    Issue { message: &'a str },
    /// A press at the watchdog's turn. `why` is required and enforced by
    /// [`crate::llm::tools::classify`], so unlike the old escape hatch's it is never absent.
    Press { buttons: &'a [JoypadButton], why: &'a str },
}

impl Report<'_> {
    /// Which directory under the run it lands in.
    fn directory(self) -> &'static str {
        match self {
            Self::Issue { .. } => files::ISSUES,
            Self::Press { .. } => files::PRESS_BUTTONS,
        }
    }

    /// The one-word discriminant in the JSON, so a directory of both can be counted apart without
    /// inferring it from which fields are null.
    fn label(self) -> &'static str {
        match self {
            Self::Issue { .. } => "issue",
            Self::Press { .. } => "press",
        }
    }
}

/// `incident.json`. Everything a person needs to answer "is the agent actually wrong here?" without
/// opening anything else.
#[derive(Debug, Serialize)]
struct Incident<'a> {
    /// Unix milliseconds, the same clock and for the same reason as [`UiEvent::at`]: a run resumed
    /// nightly restarts every elapsed counter it has, so this is the only stamp that can date a
    /// record against the transcript beside it.
    at: u64,
    run_id: String,
    turn: u64,
    /// `"issue"` or `"press"` — which of the two this is, said outright rather than inferred from
    /// which of the fields below are null.
    report: &'static str,
    /// The decision kind that asked. ⚠️ `"stuck"` means the watchdog did, which is now the only turn
    /// a press can happen on at all.
    kind: &'static str,
    /// A `report_issue` call's message: what the model tried, expected and got.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    /// What was pressed, and why. Both present only for a press.
    #[serde(skip_serializing_if = "Option::is_none")]
    buttons: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    why: Option<&'a str>,
    /// When the `state.gbst` beside this was taken, as Unix milliseconds: the start of this turn.
    /// `None` means there was none to write — a policy that never asks the model anything, or a
    /// test with no host behind it.
    state_captured_at: Option<u64>,
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
pub fn record(
    run: &CurrentRun,
    published: &Published,
    turn: u64,
    kind: DecisionKind,
    report: Report<'_>,
    summary: Option<&str>,
    messages: &[Message],
) -> Result<PathBuf, String> {
    let run = run.get();
    let parent = run.path().join(report.directory());
    std::fs::create_dir_all(&parent).map_err(|e| format!("could not create {parent:?}: {e}"))?;
    // ⚠️ A turn id is the worker's cancellation generation and restarts with the process, so a run
    // resumed twice in a day would otherwise overwrite the first record with the second.
    let dir = parent.join(run::unique_dir(&parent, &format!("turn-{turn}")));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;

    // Written first so its own timestamp can go in the JSON beside it. ⚠️ A failure here is a
    // `None` and never an error: a report that could not carry its save state still carries the
    // screen, the status and the conversation, and the caller is on its way back to the game.
    let state_captured_at = match published.latest_save_state() {
        Some((state, at)) => run::write_atomically(&dir.join(files::STATE), &state).ok().map(|()| at),
        None => None,
    };

    let incident = Incident {
        at: now_ms(),
        run_id: run.run_id(),
        turn,
        report: report.label(),
        kind: kind.label(),
        message: match report {
            Report::Issue { message } => Some(message),
            Report::Press { .. } => None,
        },
        // Lower-cased so the record spells a button the way the model's own call did — the schema's
        // enum is `"start"`, and a record grepped for what was sent should find it.
        buttons: match report {
            Report::Press { buttons, .. } => {
                Some(buttons.iter().map(|button| button.to_string().to_lowercase()).collect())
            }
            Report::Issue { .. } => None,
        },
        why: match report {
            Report::Press { why, .. } => Some(why),
            Report::Issue { .. } => None,
        },
        state_captured_at,
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

    /// The whole of it: every file lands, the conversation is cut to the last three turns, and no
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
            DecisionKind::Stuck,
            Report::Press {
                buttons: &[JoypadButton::Start, JoypadButton::A],
                why: "the screen has a menu on it and nothing has moved for a minute",
            },
            Some("checking the bag"),
            &history(),
        )
        .expect("the record writes");

        let json = std::fs::read_to_string(dir.join("incident.json")).expect("incident.json");
        assert!(dir.join("screen.png").exists(), "the screen is half of what a record is for");
        assert!(!json.contains("data:image"), "a picture must never reach the record");
        assert!(json.contains("[image removed to save context]"), "the caption stays");
        assert!(json.contains("nothing has moved for a minute"), "the reason is the headline");
        assert!(json.contains("\"report\": \"press\""), "which of the two shapes this is");
        assert!(json.contains("\"kind\": \"stuck\""), "which turn asked");
        // Spelled the model's way, not `strum`'s `Display`.
        assert!(json.contains("\"start\"") && json.contains("\"a\""), "the buttons that were pressed");
        assert!(!json.contains("\"message\""), "an issue's field has no business on a press");

        let incident: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let conversation = incident["conversation"].as_array().expect("a conversation");
        // Three turn starts, their assistant replies, and the trailing picture message.
        assert_eq!(conversation.len(), 7, "{conversation:#?}");
        assert_eq!(conversation[0]["content"], "### Turn 1", "cut at a turn boundary");
    }

    /// An issue lands in a directory of its own, carries its message and none of a press's fields,
    /// and gets the same screen and conversation treatment.
    ///
    /// ⚠️ **The separate directory is the point.** A press is now only ever the watchdog doing as it
    /// was asked; an issue is the model saying the agent is wrong. Counting the second is the whole
    /// reason the tool exists, and a shared directory would mean counting them apart by a field.
    #[test]
    fn an_issue_is_filed_apart_from_a_press() {
        let scratch = Scratch::new("incident-issue");
        let run = current(&scratch);
        let published = Published::new();

        let dir = record(
            &run,
            &published,
            9,
            DecisionKind::Overworld,
            Report::Issue { message: "the menu will not offer the ladder I am standing on" },
            Some("trying the other ladder instead"),
            &history(),
        )
        .expect("the record writes");

        assert!(dir.ends_with("turn-9"));
        assert_eq!(dir.parent().and_then(|p| p.file_name()), Some(files::ISSUES.as_ref()));
        let json = std::fs::read_to_string(dir.join("incident.json")).expect("incident.json");
        assert!(json.contains("\"report\": \"issue\""));
        assert!(json.contains("will not offer the ladder"), "the message is the record");
        assert!(json.contains("trying the other ladder instead"), "and what it did anyway");
        assert!(!json.contains("\"buttons\""), "nothing was pressed");
        assert!(!json.contains("\"why\""), "a press's field has no business on an issue");
        assert!(dir.join("screen.png").exists());
    }

    /// A second press on the same turn id — a run resumed twice in a day is the real case — must not
    /// overwrite the first record.
    #[test]
    fn two_records_of_one_turn_id_do_not_overwrite_each_other() {
        let scratch = Scratch::new("incident-collide");
        let run = current(&scratch);
        let published = Published::new();
        let write = || {
            let report = Report::Press { buttons: &[JoypadButton::A], why: "nothing has moved" };
            record(&run, &published, 1, DecisionKind::Stuck, report, None, &[])
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
