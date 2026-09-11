//! What the model says, and does, when it believes the agent is wrong.
//! ```text
//! $GB_RUN_DIR/<run-id>/issues/turn-<id>/          a `report_issue` call
//! $GB_RUN_DIR/<run-id>/press-buttons/turn-<id>/   a press at the watchdog's turn
//!   ├── incident.json   the report, the decision, the run's whole state, and the conversation
//!   ├── screen.png      what was actually on the screen
//!   └── state.gbst      the machine as the turn found it, to replay from
//! ```

use std::path::PathBuf;

use serde::Serialize;

use gb::joypad::JoypadButton;
use crate::llm::compaction;
use crate::llm::protocol::Message;
use crate::llm::screenshot;
use crate::llm::tools::DecisionKind;
use crate::run::{self, CurrentRun, files};
use crate::published::{Published, UiEvent, now_ms};

const TURNS_KEPT: usize = 3;

/// What is being filed: the directory and two JSON fields differ, and the rest is the same.
#[derive(Debug, Clone, Copy)]
pub enum Report<'a> {
    /// A `report_issue` call: the model's own account of what the agent will not let it do.
    Issue { message: &'a str },
    /// A press at the watchdog's turn.
    Press { buttons: &'a [JoypadButton], why: &'a str },
}

impl Report<'_> {
    fn directory(self) -> &'static str {
        match self {
            Self::Issue { .. } => files::ISSUES,
            Self::Press { .. } => files::PRESS_BUTTONS,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Issue { .. } => "issue",
            Self::Press { .. } => "press",
        }
    }
}

/// `incident.json`.
#[derive(Debug, Serialize)]
struct Incident<'a> {
    /// Unix milliseconds, as [`UiEvent::at`]: the one stamp a resume does not restart.
    at: u64,
    run_id: String,
    turn: u64,
    /// `"issue"` or `"press"`, said outright rather than inferred from which fields are null.
    report: &'static str,
    /// The decision kind that asked.
    kind: &'static str,
    /// A `report_issue` call's message: what the model tried, expected and got.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    /// What was pressed, and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    buttons: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    why: Option<&'a str>,
    /// When the `state.gbst` beside this was taken, as Unix milliseconds: the start of this turn.
    state_captured_at: Option<u64>,
    summary: Option<&'a str>,
    /// The last status heartbeat, whole.
    status: Option<UiEvent>,
    /// The last [`TURNS_KEPT`] turns, with every image replaced by its caption.
    conversation: Vec<Message>,
}

/// Write the record, and answer its directory.
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
    // A turn id restarts with the process, so one run can see the same id twice.
    let dir = parent.join(run::unique_dir(&parent, &format!("turn-{turn}")));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {dir:?}: {e}"))?;

    // Written first so its own timestamp can go in the JSON beside it.
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
        // Lower-cased, as the schema's enum spells them, so a grep for what was sent finds it.
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
    // `write_atomically` replaces the extension, so these stage as `incident.tmp` and `screen.tmp`.
    run::write_atomically(&dir.join("incident.json"), &json)?;

    let frame = published.latest_frame();
    run::write_atomically(&dir.join("screen.png"), &screenshot::encode(&frame.pixels))?;
    Ok(dir)
}

/// The tail of the history, cut at a turn boundary and stripped of pictures.
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
    use crate::run::Scratch;

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

    #[test]
    fn a_short_history_is_carried_whole() {
        assert!(recent_turns(&[]).is_empty());
        let messages = vec![Message::system("system"), Message::user("### Turn 0")];
        assert_eq!(recent_turns(&messages).len(), 2, "nothing is dropped from below the window");
    }

    #[test]
    fn the_slice_starts_at_a_user_message() {
        let slice = recent_turns(&history());
        assert_eq!(slice[0].role, Role::User);
    }
}
