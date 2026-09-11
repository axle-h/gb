//! `transcript.jsonl`: every `UiEvent` worth keeping, one JSON object per line.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::published::{Published, UiEvent, UiEventBody};

pub(crate) const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// The most events `/api/history` will return, however far back `since` reaches. The SPA keeps
/// 500 entries, so this is already more than it can show; the cap exists so a month-old run
/// cannot make a page load allocate a hundred megabytes.
pub const MAX_BACKLOG: usize = 2_000;

/// Write every event to `path` until `stop` is set or the process ends.
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
                // Follow the run, do not capture it.
                let live = current.get().transcript_path();
                if live != path {
                    let _ = writer.flush();
                    match open_append(&live) {
                        Ok(file) => {
                            written = file.metadata().map(|m| m.len()).unwrap_or(0);
                            writer = BufWriter::new(file);
                            path = live;
                        }
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

/// Whether an event belongs in the file.
fn keep(event: &UiEvent) -> bool {
    !matches!(event.body, UiEventBody::Status(_))
}

pub(crate) fn open_append(path: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

pub(crate) fn rotate(path: &Path) -> Result<std::fs::File, String> {
    let previous = path.with_extension("jsonl.1");
    std::fs::rename(path, &previous)
        .map_err(|e| format!("could not rotate {} to {}: {e}", path.display(), previous.display()))?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not reopen {} after rotating it: {e}", path.display()))
}

/// The backlog `/api/history?since=` serves: every event with `seq >= since`, oldest first,
/// capped at the most recent [`MAX_BACKLOG`].
pub fn read_since(path: &Path, since: u64) -> Vec<serde_json::Value> {
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    let mut events = Vec::new();
    for line in RevLines::new(file) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        match event["seq"].as_u64() {
            Some(seq) if seq >= since => events.push(event),
            Some(_) => break,
            None => continue,
        }
        if events.len() >= MAX_BACKLOG {
            break;
        }
    }
    events.reverse();
    events
}

/// The last sequence number in the file, if there is one.
pub fn last_seq(path: &Path) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    RevLines::new(file)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
        .find_map(|event| event["seq"].as_u64())
}

/// How much of the file a backwards read pulls in at a time.
const CHUNK: u64 = 64 * 1024;

/// The lines of a file, last first, read in [`CHUNK`]s from the end so that a file of any size
/// costs only what is consumed.
struct RevLines {
    file: std::fs::File,
    /// Everything below this offset is still unread.
    pos: u64,
    /// Bytes read but not yet yielded: the (possibly partial) first line of the chunks seen so
    /// far, without its newline, which completes once the chunk before it arrives.
    pending: Vec<u8>,
    /// Whole lines ready to yield, in file order, so `pop` is the next line backwards.
    ready: Vec<String>,
}

impl RevLines {
    fn new(file: std::fs::File) -> Self {
        let pos = file.metadata().map(|m| m.len()).unwrap_or(0);
        Self { file, pos, pending: Vec::new(), ready: Vec::new() }
    }

    /// Pull the next chunk off the end and split it into lines.
    fn fill(&mut self) -> bool {
        use std::io::{Read, Seek, SeekFrom};
        if self.pos == 0 {
            if self.pending.is_empty() {
                return false;
            }
            let first = std::mem::take(&mut self.pending);
            self.ready.push(String::from_utf8_lossy(&first).into_owned());
            return true;
        }
        let len = self.pos.min(CHUNK);
        self.pos -= len;
        let mut buf = vec![0u8; len as usize];
        if self.file.seek(SeekFrom::Start(self.pos)).is_err() || self.file.read_exact(&mut buf).is_err() {
            self.pos = 0;
            self.pending.clear();
            return false;
        }
        buf.append(&mut self.pending);
        // Everything after the first newline is whole lines that end in this chunk; everything
        // before it belongs to a line that starts in an earlier chunk and stays pending.
        let Some(first_nl) = buf.iter().position(|&b| b == b'\n') else {
            self.pending = buf;
            return true;
        };
        let rest = buf.split_off(first_nl + 1);
        buf.pop(); // the newline that ended the pending line; it must not be split on again
        self.pending = buf;
        // `ready` is popped from the back, so pushing in file order yields the lines last-first.
        for line in rest.split(|&b| b == b'\n') {
            self.ready.push(String::from_utf8_lossy(line).into_owned());
        }
        true
    }
}

impl Iterator for RevLines {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        loop {
            if let Some(line) = self.ready.pop() {
                return Some(line);
            }
            if !self.fill() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Scratch;
    use crate::published::{RunStatus, StatusSnapshot};
    use std::time::{Duration, Instant};

    /// A `CurrentRun` over a fresh run directory under `root`.
    fn current_run(root: &Path) -> Arc<crate::run::CurrentRun> {
        let (run, _, _) = crate::run::RunDir::open(root, true, "test", &|_| false).expect("a fresh run");
        Arc::new(crate::run::CurrentRun::new(root.to_path_buf(), "test".to_string(), run))
    }

    fn heartbeat() -> UiEventBody {
        UiEventBody::Status(Box::new(StatusSnapshot {
            wall_ms: 0,
            emulated_ms: 0,
            run_emulated_ms: 0,
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

        // …and where a second process must pick the numbering up from.
        assert_eq!(last_seq(&path), Some(3));
        assert_eq!(last_seq(&scratch.0.join("nothing-here.jsonl")), None);

        stop.store(true, Ordering::Relaxed);
        published.publish_event(UiEventBody::Notice { level: "info", message: "after".into() });
        let _ = writer.join();
    }

    /// A restarted process appends to the file it left rather than truncating it — the transcript
    /// is the one thing in the run directory that is not a snapshot.
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

    /// `RevLines` against lines that straddle chunk boundaries, a file with no trailing newline,
    /// and one smaller than a chunk — every line comes back, last first, byte for byte.
    #[test]
    fn lines_are_read_back_from_the_end_across_chunk_boundaries() {
        let scratch = Scratch::new("transcript-rev");
        let path = scratch.0.join("rev.txt");
        // Lines of awkward, varying lengths so that many of them straddle a 64 KiB boundary.
        let lines: Vec<String> = (0..5000).map(|i| format!("{i}:{}", "x".repeat(i % 97 + 1))).collect();
        for trailing_newline in [true, false] {
            let mut text = lines.join("\n");
            if trailing_newline {
                text.push('\n');
            }
            std::fs::write(&path, &text).expect("write");
            let got: Vec<String> = RevLines::new(std::fs::File::open(&path).unwrap()).collect();
            let mut want: Vec<String> = lines.clone();
            if trailing_newline {
                want.push(String::new());
            }
            want.reverse();
            assert_eq!(got, want, "trailing newline: {trailing_newline}");
        }
        std::fs::write(&path, "only\n").expect("write");
        let got: Vec<String> = RevLines::new(std::fs::File::open(&path).unwrap()).collect();
        assert_eq!(got, vec!["".to_string(), "only".to_string()]);
    }

    /// The deployed failure: a transcript far larger than anything the backlog returns is served
    /// without being read whole.
    #[test]
    fn a_huge_transcript_is_not_read_whole() {
        let scratch = Scratch::new("transcript-huge");
        let path = scratch.0.join("transcript.jsonl");
        let total: u64 = 200_000;
        let padding = "p".repeat(150);
        {
            let mut w = BufWriter::new(std::fs::File::create(&path).unwrap());
            for seq in 0..total {
                writeln!(w, "{{\"seq\":{seq},\"type\":\"notice\",\"level\":\"info\",\"message\":\"{padding}\"}}")
                    .unwrap();
            }
        }
        // The cap is honoured, and the values prove the loop *stopped* rather than read on and
        // threw the rest away: `read_since` pushes every line it parses and has no discard path,
        // so a result holding exactly the last `MAX_BACKLOG` seqs is a result that broke at the
        // cap.
        let events = read_since(&path, 0);
        assert_eq!(events.len(), MAX_BACKLOG);
        assert_eq!(events[0]["seq"], total - MAX_BACKLOG as u64);
        assert_eq!(events.last().unwrap()["seq"], total - 1);

        // The same for the other exit: the first `seq` below `since` ends the walk.
        let tail = read_since(&path, total - 10);
        assert_eq!(tail.len(), 10);
        assert_eq!(tail[0]["seq"], total - 10);
        assert_eq!(last_seq(&path), Some(total - 1));

        // The other half of the name — *not read whole* — is about the reader underneath, and it
        // is measured rather than timed.
        let length = std::fs::metadata(&path).unwrap().len();
        let mut lines = RevLines::new(std::fs::File::open(&path).unwrap());
        for _ in 0..MAX_BACKLOG {
            lines.next().expect("the file holds far more lines than the cap");
        }
        let read = length - lines.pos;
        assert!(read < length / 10,
                "a backlog's worth of lines is {read} bytes of a {length}-byte file; a reader that \
                 slurped it whole would have read all of it");
    }

}
