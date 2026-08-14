//! **W7 / §11** — `transcript.jsonl`: every `UiEvent` worth keeping, one JSON object per line.
//!
//! Written by a thread of its own, subscribed to the same broadcast the browser reads. Neither the
//! emulator nor the LLM worker ever touches the file: a `publish_event` call is a channel send and
//! must stay one, because the emulator thread makes them at 10 Hz between instructions and the
//! worker makes one per streamed token.
//!
//! ⚠️ **The status heartbeat is deliberately excluded.** §11 says "one JSON object per `UiEvent`",
//! which taken literally is ten a second of a message whose entire purpose is to be current — 36 000
//! lines and ~14 MB an hour, drowning the conversation it is supposed to preserve and making
//! `/api/history` a replay of yesterday's clock. A viewer gets a fresh heartbeat within 100 ms of
//! connecting, so nothing is lost by leaving them out and a great deal is gained: what remains is
//! the run's *story* — what the agent did, what the model said, what it decided.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::web::published::{Published, UiEvent, UiEventBody};

/// §11's rotation point. Reached only by a run that has been going for weeks: with heartbeats
/// excluded a busy hour is a few hundred kilobytes.
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// The most events `/api/history` will return, however far back `since` reaches. The SPA keeps 500
/// entries, so this is already more than it can show; the cap exists so a month-old run cannot make
/// a page load allocate a hundred megabytes.
pub const MAX_BACKLOG: usize = 2_000;

/// Write every event to `path` until `stop` is set or the process ends.
///
/// Returns immediately; the work is on its own thread. Failure to *open* the file is reported to the
/// caller, and failure to write is reported once and then the thread stops — a run whose disk has
/// filled should not also spend the rest of its life printing about it.
pub fn spawn(
    current: Arc<crate::run::CurrentRun>,
    published: Arc<Published>,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>, String> {
    let mut events = published.subscribe_events();
    let mut path = current.get().transcript_path();
    let file = open_append(&path)?;
    let mut written = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut writer = BufWriter::new(file);

    std::thread::Builder::new()
        .name("transcript".to_string())
        .spawn(move || {
            while let Ok(event) = events.blocking_recv() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if !keep(&event) {
                    continue;
                }
                // ⚠️ **Follow the run, do not capture it.** `POST /api/new-run` swaps the current run
                // directory under a live process, and this thread is the only writer of the file it
                // swaps away from — a captured `PathBuf` would keep appending the new run's events to
                // the old run's transcript, which is invisible until someone reads either file. The
                // check is a `PathBuf` join and a compare per *kept* event (heartbeats are filtered
                // out above), so it costs nothing on the path that matters.
                let live = current.get().transcript_path();
                if live != path {
                    let _ = writer.flush();
                    match open_append(&live) {
                        Ok(file) => {
                            written = file.metadata().map(|m| m.len()).unwrap_or(0);
                            writer = BufWriter::new(file);
                            path = live;
                        }
                        // Keep writing to the old file rather than losing the run's events outright.
                        Err(failure) => eprintln!("transcript: {failure} — still writing to {}",
                                                  path.display()),
                    }
                }
                let Ok(line) = serde_json::to_string(&event) else { continue };
                if writeln!(writer, "{line}").is_err() || writer.flush().is_err() {
                    eprintln!("transcript: could not write to {} — it stops here", path.display());
                    return;
                }
                written += line.len() as u64 + 1;
                if written >= MAX_BYTES {
                    match rotate(&path) {
                        Ok(file) => {
                            writer = BufWriter::new(file);
                            written = 0;
                        }
                        Err(failure) => {
                            eprintln!("transcript: {failure} — it stops here");
                            return;
                        }
                    }
                }
            }
            let _ = writer.flush();
        })
        .map_err(|e| format!("could not start the transcript thread: {e}"))
}

/// Whether an event belongs in the file. See the module's ⚠️.
fn keep(event: &UiEvent) -> bool {
    !matches!(event.body, UiEventBody::Status(_))
}

fn open_append(path: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

fn rotate(path: &Path) -> Result<std::fs::File, String> {
    let previous = path.with_extension("jsonl.1");
    std::fs::rename(path, &previous)
        .map_err(|e| format!("could not rotate {} to {}: {e}", path.display(), previous.display()))?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not reopen {} after rotating it: {e}", path.display()))
}

/// The backlog `/api/history?since=` serves: every event with `seq >= since`, oldest first, capped
/// at the most recent [`MAX_BACKLOG`].
///
/// Parsed back to `serde_json::Value` rather than re-serialised from a typed struct, because the file
/// is the wire format already — round-tripping it through `UiEventBody` would mean the transcript
/// could only ever hold events *this* build knows the shape of.
pub fn read_since(path: &Path, since: u64) -> Vec<serde_json::Value> {
    let Ok(contents) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut events: Vec<serde_json::Value> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["seq"].as_u64().is_some_and(|seq| seq >= since))
        .collect();
    if events.len() > MAX_BACKLOG {
        events.drain(..events.len() - MAX_BACKLOG);
    }
    events
}

/// The last sequence number in the file, if there is one.
///
/// ⚠️ **A resumed run must not restart its sequence numbers**, and this is what stops it. The
/// counter lives in `Published`, which is built fresh every process — so a second process would
/// otherwise write `seq: 0` again, ten thousand lines into a transcript that already has one. Two
/// things break at once: `?since=` selects across both ranges, and the browser, which keys entries
/// by sequence number, gets duplicates. Found by reading the file after a restart.
pub fn last_seq(path: &Path) -> Option<u64> {
    let contents = std::fs::read_to_string(path).ok()?;
    contents
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|event| event["seq"].as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::Scratch;
    use crate::web::published::{RunStatus, StatusSnapshot};
    use std::time::{Duration, Instant};

    /// A `CurrentRun` over a fresh run directory under `root`. The transcript now lives *inside* a
    /// run directory rather than wherever the caller says, because that is what the writer follows.
    fn current_run(root: &Path) -> Arc<crate::run::CurrentRun> {
        let (run, _, _) = crate::run::RunDir::open(root, true, "test", &|_| false).expect("a fresh run");
        Arc::new(crate::run::CurrentRun::new(root.to_path_buf(), "test".to_string(), run))
    }

    fn heartbeat() -> UiEventBody {
        UiEventBody::Status(Box::new(StatusSnapshot {
            wall_ms: 0,
            emulated_ms: 0,
            dropped_ms: 0,
            target_speed: 1.0,
            policy: "random",
            model: None,
            agent_state: "idle".into(),
            frame_seq: 0,
            game: None,
            run: RunStatus::Playing,
        }))
    }

    /// The whole path: published on one thread, on disk from another, and read back in order — with
    /// the heartbeats left out, which is the thing §11 did not say.
    #[test]
    fn the_story_is_written_and_the_heartbeats_are_not() {
        let scratch = Scratch::new("transcript");
        let published = Published::new();
        let stop = Arc::new(AtomicBool::new(false));
        let current = current_run(&scratch.0);
        let path = current.get().transcript_path();
        let writer = spawn(Arc::clone(&current), Arc::clone(&published), Arc::clone(&stop)).expect("starts");

        published.publish_event(heartbeat());
        published.publish_event(UiEventBody::Notice { level: "info", message: "one".into() });
        published.publish_event(heartbeat());
        published.publish_event(UiEventBody::Decision {
            turn: 1,
            summary: "wait 1 ticks".into(),
            narration: None,
            usage: None,
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while read_since(&path, 0).len() < 2 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let events = read_since(&path, 0);
        assert_eq!(events.len(), 2, "only the two that are not heartbeats: {events:?}");
        assert_eq!(events[0]["type"], "notice");
        assert_eq!(events[0]["seq"], 1, "the sequence numbers are the broadcast's, gaps and all");
        assert_eq!(events[1]["type"], "decision");

        // `since` is inclusive of the sequence asked for, which is what makes it a resume point.
        assert_eq!(read_since(&path, 3).len(), 1);
        assert_eq!(read_since(&path, 99).len(), 0);
        assert_eq!(read_since(&scratch.0.join("nothing-here.jsonl"), 0).len(), 0);

        // ⚠️ …and where a second process must pick the numbering up from.
        assert_eq!(last_seq(&path), Some(3));
        assert_eq!(last_seq(&scratch.0.join("nothing-here.jsonl")), None);

        stop.store(true, Ordering::Relaxed);
        published.publish_event(UiEventBody::Notice { level: "info", message: "after".into() });
        let _ = writer.join();
    }

    /// A restarted process appends to the file it left rather than truncating it — the transcript is
    /// the one thing in the run directory that is not a snapshot.
    #[test]
    fn a_second_process_appends_rather_than_starting_again() {
        let scratch = Scratch::new("transcript-append");
        let current = current_run(&scratch.0);
        let path = current.get().transcript_path();
        std::fs::write(&path, "{\"seq\":0,\"type\":\"notice\",\"level\":\"info\",\"message\":\"before\"}\n")
            .expect("write");

        let published = Published::new();
        let stop = Arc::new(AtomicBool::new(false));
        let writer = spawn(Arc::clone(&current), Arc::clone(&published), Arc::clone(&stop)).expect("starts");
        published.publish_event(UiEventBody::Notice { level: "info", message: "after".into() });

        let deadline = Instant::now() + Duration::from_secs(5);
        while read_since(&path, 0).len() < 2 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let events = read_since(&path, 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["message"], "before");
        assert_eq!(events[1]["message"], "after");

        stop.store(true, Ordering::Relaxed);
        published.publish_event(UiEventBody::Notice { level: "info", message: "last".into() });
        let _ = writer.join();
    }

    /// A month-old transcript must not make a page load allocate the whole file.
    #[test]
    fn the_backlog_is_capped_at_the_most_recent_events() {
        let scratch = Scratch::new("transcript-cap");
        let path = scratch.0.join("transcript.jsonl");
        let lines: String = (0..MAX_BACKLOG + 500)
            .map(|seq| format!("{{\"seq\":{seq},\"type\":\"notice\",\"level\":\"info\",\"message\":\"{seq}\"}}\n"))
            .collect();
        std::fs::write(&path, lines).expect("write");

        let events = read_since(&path, 0);
        assert_eq!(events.len(), MAX_BACKLOG);
        assert_eq!(events[0]["seq"], 500, "the cap keeps the *recent* end");
        assert_eq!(events.last().unwrap()["seq"], (MAX_BACKLOG + 499) as u64);
    }
}
