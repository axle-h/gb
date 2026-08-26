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
pub(crate) const MAX_BYTES: u64 = 256 * 1024 * 1024;

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

/// The backlog `/api/history?since=` serves: every event with `seq >= since`, oldest first, capped
/// at the most recent [`MAX_BACKLOG`].
///
/// Parsed back to `serde_json::Value` rather than re-serialised from a typed struct, because the file
/// is the wire format already — round-tripping it through `UiEventBody` would mean the transcript
/// could only ever hold events *this* build knows the shape of.
///
/// ⚠️ **Read from the end, and never the whole file.** The first version was `read_to_string` and a
/// parse of every line with the cap applied last — fine at the "couple of megabytes" it was written
/// for, and what OOM-killed the deployed pod at its 2 GiB limit once a reasoning model publishing
/// one event per streamed token had grown a four-day run's transcript to 254 MB and 2.9 million
/// lines. A page load is `/api/history`, so the run died *on connect*, eight times. This walks
/// backwards in [`CHUNK`]-sized reads and stops at the cap or at the first event below `since`
/// (sequence numbers are monotonic within a file; that is what [`last_seq`] is for), so the
/// allocation is bounded by what is returned rather than by the age of the run.
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
///
/// ⚠️ **A resumed run must not restart its sequence numbers**, and this is what stops it. The
/// counter lives in `Published`, which is built fresh every process — so a second process would
/// otherwise write `seq: 0` again, ten thousand lines into a transcript that already has one. Two
/// things break at once: `?since=` selects across both ranges, and the browser, which keys entries
/// by sequence number, gets duplicates. Found by reading the file after a restart.
///
/// Reads from the end for the same reason as [`read_since`]: this runs at every start, and a
/// `read_to_string` here was a quarter of a gigabyte of baseline on the deployed run.
pub fn last_seq(path: &Path) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    RevLines::new(file)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
        .find_map(|event| event["seq"].as_u64())
}

/// How much of the file a backwards read pulls in at a time. A transcript line is a few hundred
/// bytes, a `plan` a few kilobytes, so a chunk holds many of them and a line straddling two is the
/// rare case rather than the rule.
const CHUNK: u64 = 64 * 1024;

/// The lines of a file, last first, read in [`CHUNK`]s from the end so that a file of any size
/// costs only what is consumed. Invalid UTF-8 is replaced rather than failing the whole read, since
/// one torn line must not hide the rest of the file.
struct RevLines {
    file: std::fs::File,
    /// Everything below this offset is still unread.
    pos: u64,
    /// Bytes read but not yet yielded: the (possibly partial) first line of the chunks seen so far,
    /// without its newline, which completes once the chunk before it arrives.
    pending: Vec<u8>,
    /// Whole lines ready to yield, in file order, so `pop` is the next line backwards.
    ready: Vec<String>,
}

impl RevLines {
    fn new(file: std::fs::File) -> Self {
        let pos = file.metadata().map(|m| m.len()).unwrap_or(0);
        Self { file, pos, pending: Vec::new(), ready: Vec::new() }
    }

    /// Pull the next chunk off the end and split it into lines. Returns `false` at the start of
    /// the file, once whatever was pending has been flushed as the first line.
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
    /// without being read whole. The file is ~40 MB; the read is bounded by the cap, so it finishes
    /// in a small fraction of what parsing every line would take, and `since` near the end reads
    /// almost nothing at all.
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
        // The cap is honoured, and the values prove the loop *stopped* rather than read on and threw
        // the rest away: `read_since` pushes every line it parses and has no discard path, so a
        // result holding exactly the last `MAX_BACKLOG` seqs is a result that broke at the cap.
        let events = read_since(&path, 0);
        assert_eq!(events.len(), MAX_BACKLOG);
        assert_eq!(events[0]["seq"], total - MAX_BACKLOG as u64);
        assert_eq!(events.last().unwrap()["seq"], total - 1);

        // The same for the other exit: the first `seq` below `since` ends the walk.
        let tail = read_since(&path, total - 10);
        assert_eq!(tail.len(), 10);
        assert_eq!(tail[0]["seq"], total - 10);
        assert_eq!(last_seq(&path), Some(total - 1));

        // ⚠️ **The other half of the name — *not read whole* — is about the reader underneath, and
        // it is measured rather than timed.** This asserted "the capped read took under two seconds"
        // and "a read near the end is faster than one across the cap"; both are races against
        // whatever else the machine is doing, and the pair went red once under a loaded full-suite
        // run while passing alone every time. `RevLines::pos` is the offset below which nothing has
        // been read, so laziness is a number the test can simply look at.
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

    /// Against a real transcript: `GB_TRANSCRIPT=/path/to/transcript.jsonl`. Prints how long the
    /// backlog and the last sequence number take to read, and the peak resident size afterwards.
    #[test]
    #[ignore]
    #[cfg(feature = "diagnostics")]
    fn probe_real_transcript() {
        let Ok(path) = std::env::var("GB_TRANSCRIPT") else { return };
        let path = Path::new(&path);
        let started = Instant::now();
        let events = read_since(path, 0);
        println!("read_since(0): {} events in {:?}", events.len(), started.elapsed());
        let started = Instant::now();
        println!("last_seq: {:?} in {:?}", last_seq(path), started.elapsed());
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines().filter(|l| l.starts_with("VmHWM") || l.starts_with("VmRSS")) {
                println!("{line}");
            }
        }
    }
}
