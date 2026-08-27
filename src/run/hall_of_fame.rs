//! **The end of the game**: a finished run's permanent record, and the leaderboard built from them.
//!
//! `gb` could always play Pokémon Red to the Hall of Fame and never knew that it had. This module is
//! what happens when [`crate::pokemon::agent::AgentEvent::HallOfFame`] fires: the winning run is
//! copied out whole, one line describing it is appended to a ledger, and [`top`] reads that ledger
//! back for `/api/leaderboard`.
//!
//! ```text
//! $GB_RUN_DIR/
//!     run-20260810-093011/                    a live run — one writer, unchanged
//!     hall-of-fame/
//!         ledger.jsonl                        one JSON object per completion, append-only
//!         20260812-142530-run-20260810-093011/
//!             meta.json                       the run's own meta, plus the completion facts
//!             state.gbst                      the save state at the moment of victory
//!             sram.bin
//!             transcript.jsonl.gz             the whole story, up to and including the win
//!             transcript.jsonl.1.gz           the rotated half, when there is one
//!             memories/  todo.json            the model's own notes
//!             battle-script.json                   how it chose to fight
//! ```
//!
//! ⚠️ **The nesting is load-bearing, not tidiness.** [`crate::run::resumable`] lists the *direct*
//! children of `$GB_RUN_DIR` that hold a `state.gbst` and continues the newest one. Every archive is
//! a complete run directory, `state.gbst` included, written *after* the run it copied — so an
//! archive placed beside the runs would be the newest resumable thing on the volume the instant it
//! landed, and the next `gb serve` would resume into a copy of a game that has already been won and
//! filed rather than into the run it had just started. One level of nesting is the whole fix:
//! `hall-of-fame/` has no state file of its own, so the one-level-deep scan skips it.
//!
//! ⚠️ **A JSONL ledger rather than SQLite, deliberately.** Ten rows read and sorted in memory is not
//! a query workload, and the alternative — `rusqlite` with `bundled` — compiles SQLite's C
//! amalgamation into every build including a container whose only non-Rust dependency is `ring`'s.
//! This is the same trade `super::civil` makes against `chrono`.
//!
//! ⚠️ **Ranking is on the *cartridge's* clock** (`wPlayTime`, as `playtime_seconds`), not on ours.
//! It lives in the save state, so it survives every resume with no bookkeeping at all; it is gated
//! on `BIT_GAME_TIMER_COUNTING`, so it counts while a game is in progress and stops on a title
//! screen; and it is the number a player would actually quote. Our own `emulated_ms` and `wall_ms`
//! are recorded beside it because they are the only figures that separate "the emulator ran for six
//! hours" from "the game clock advanced six hours" — but they are not what a leaderboard is for.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::{ArchivedCompletion, RunMeta, files, unique_dir};

/// How long [`archive`] will wait for the transcript writer to catch up to the completion event.
///
/// ⚠️ **Not a fudge — the transcript is written by a different thread.** `transcript.rs` is a
/// `blocking_recv` loop on the broadcast channel, so at the moment the emulator thread decides to
/// archive, the event announcing the victory has been *published* but very likely not yet written.
/// Copying the file now would produce an archive of a win with no win in it. The follow below reads
/// whole lines until it sees the event's own sequence number, and gives up here rather than block
/// the emulator for ever if the writer has died.
const TRANSCRIPT_FOLLOW: Duration = Duration::from_secs(5);

/// How often the follow re-checks a file it has read to the end of.
const FOLLOW_POLL: Duration = Duration::from_millis(50);

/// One finished playthrough, as it appears in the ledger and on `/api/leaderboard`.
///
/// `Deserialize` as well as `Serialize` because this *is* the wire format: the row is read back by
/// [`top`], not projected out of some richer store.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Completion {
    /// The archive directory's name, relative to `<root>/hall-of-fame/`.
    pub archive: String,
    pub run_id: String,
    /// `wNumHoFTeams` after the increment: `1` is a first championship, `2` a second in the same save.
    pub teams: u8,
    pub completed_at: String,
    pub started_at: String,
    /// [`crate::cli::VERSION`] at the time — which build played this.
    pub app_version: String,
    /// [`crate::pokemon::policy::Policy::name`].
    pub policy: String,
    /// `GB_MODEL`. `None` under any policy that is not an LLM.
    pub model: Option<String>,

    // ── How long it took ─────────────────────────────────────────────────────────────────────────
    /// The cartridge's own play clock, in seconds. **The ranking key** — see the module note.
    pub playtime_seconds: u32,
    /// `HH:MM:SS`, for a person reading the ledger. ⚠️ Never sort on this: the hours field runs to
    /// 255, so it is two digits below 100 hours and three above, and a lexical comparison puts
    /// `255:59:59` before `06:12:44`.
    pub playtime: String,
    /// `wPlayTimeMaxed` — the clock stopped at 255:59:59 and the real figure is unknown. Such a run
    /// ranks last rather than first.
    pub playtime_maxed: bool,
    /// Emulated milliseconds over the run's whole life, across every process that played it.
    pub emulated_ms: u64,
    /// Wall clock spent playing, ditto.
    pub wall_ms: u64,

    // ── What it cost ─────────────────────────────────────────────────────────────────────────────
    pub turns: u64,
    pub completions: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The endpoint reported no `usage` and these are our own estimate. A guess presented as a
    /// measurement is worse than no number.
    pub tokens_estimated: bool,
    /// Times the stuck-run watchdog fired. Zero in a healthy run.
    pub watchdog_firings: u64,
    /// How many times the run was resumed by a new process.
    pub resumes: usize,
    pub checkpoints: u64,

    // ── What it finished with ────────────────────────────────────────────────────────────────────
    pub badges: u32,
    pub pokedex_owned: usize,
    pub pokedex_seen: usize,
    pub money: u32,
    pub party: Vec<PartyMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PartyMember {
    pub nickname: String,
    pub species: String,
    pub level: u8,
}

impl Completion {
    /// Where this run sorts. Lower is better; a maxed clock is pushed behind every honest one,
    /// because 255:59:59 is not a time, it is the counter giving up. Ties break on when the run
    /// finished, so the board is stable rather than dependent on the order of the file.
    fn rank(&self) -> (bool, u32, &str) {
        (self.playtime_maxed, self.playtime_seconds, &self.completed_at)
    }
}

/// Everything [`archive`] needs, captured on the emulator thread at the instant of victory.
pub struct ArchiveJob {
    /// `$GB_RUN_DIR`.
    pub root: PathBuf,
    /// The run directory being filed. Still the current run — see [`archive`]'s ⚠️ about ordering.
    pub run_dir: PathBuf,
    pub meta: RunMeta,
    /// `gb.save_state()` at the moment the counter moved. Held in memory rather than re-read from
    /// the checkpoint that has just been written, because the two must not be able to differ.
    pub state: Vec<u8>,
    pub sram: Vec<u8>,
    /// The sequence number [`crate::web::published::Published::publish_event`] returned for the
    /// completion event — where the transcript follow stops.
    pub until_seq: u64,
    /// The row to append, bar the fields only this module can fill in (`archive`, `completed_at`).
    pub completion: Completion,
}

/// Copy the run out and append its row. Returns the archive directory's name.
///
/// ⚠️ **Called synchronously, on the emulator thread, and *before* the run is swapped.**
/// `transcript.rs` re-reads `CurrentRun::get().transcript_path()` per event, so once a new run is
/// current, an event published before the swap but written after it lands in the *new* run's
/// transcript. Doing the follow first is what makes "which file is the victory in?" a question with
/// an answer. It costs the stream a stall bounded by [`TRANSCRIPT_FOLLOW`] plus the copy — once per
/// run, during a cutscene.
pub fn archive(job: &ArchiveJob) -> Result<String, String> {
    let home = job.root.join(files::HALL_OF_FAME);
    std::fs::create_dir_all(&home)
        .map_err(|e| format!("could not create {}: {e}", home.display()))?;

    let name = unique_dir(
        &home,
        &format!("{}-{}", super::compact_timestamp(SystemTime::now()), job.meta.run_id),
    );
    let into = home.join(&name);
    std::fs::create_dir_all(&into)
        .map_err(|e| format!("could not create {}: {e}", into.display()))?;

    std::fs::write(into.join(files::STATE), &job.state)
        .map_err(|e| format!("could not write the archived save state: {e}"))?;
    std::fs::write(into.join(files::SRAM), &job.sram)
        .map_err(|e| format!("could not write the archived sram: {e}"))?;

    // The whole story, and the whole reason this is not `fs::copy` — see `TRANSCRIPT_FOLLOW`.
    follow_lines(
        &job.run_dir.join(files::TRANSCRIPT),
        &into.join(format!("{}.gz", files::TRANSCRIPT)),
        job.until_seq,
    )?;
    // The rotated half, if this run went on long enough to have one. Already complete, so no follow.
    let rotated = job.run_dir.join(files::TRANSCRIPT).with_extension("jsonl.1");
    if rotated.exists() {
        follow_lines(&rotated, &into.join("transcript.jsonl.1.gz"), u64::MAX)?;
    }

    // `memories/` is legacy — nothing has written one since W6b's two note mechanisms became one —
    // but a run old enough to have one is exactly the kind whose archive should be complete.
    // `copy_tree` is a no-op when the directory is not there.
    copy_tree(&job.run_dir.join(files::MEMORIES), &into.join(files::MEMORIES))?;
    let todo = job.run_dir.join(files::TODO);
    if todo.exists() {
        std::fs::copy(&todo, into.join(files::TODO))
            .map_err(|e| format!("could not copy {}: {e}", todo.display()))?;
    }
    // The battle script, if the run wrote one. It is a decision the model made about how to play
    // rather than a cache, so a finished run's archive is incomplete without it.
    let script = job.run_dir.join(files::BATTLE_SCRIPT);
    if script.exists() {
        std::fs::copy(&script, into.join(files::BATTLE_SCRIPT))
            .map_err(|e| format!("could not copy {}: {e}", script.display()))?;
    }

    // The conversation, both halves. `history.json` is written by rename, so a plain copy of it is
    // whole or absent and never torn.
    let history = job.run_dir.join(files::HISTORY);
    if history.exists() {
        std::fs::copy(&history, into.join(files::HISTORY))
            .map_err(|e| format!("could not copy {}: {e}", history.display()))?;
    }
    // ⚠️ **Whole lines, not `fs::copy`, for the transcript's reason**: the LLM worker is appending
    // to this on its own thread while we read, so a byte copy can catch it between the `writeln!`
    // and the flush. There is no seq to follow to — nothing keys on one — so it is read to EOF.
    follow_lines(
        &job.run_dir.join(files::CONVERSATION),
        &into.join(format!("{}.gz", files::CONVERSATION)),
        u64::MAX,
    )?;
    let rotated_log = job.run_dir.join(format!("{}.1", files::CONVERSATION));
    if rotated_log.exists() {
        follow_lines(&rotated_log, &into.join(format!("{}.1.gz", files::CONVERSATION)), u64::MAX)?;
    }

    let mut completion = job.completion.clone();
    completion.archive = name.clone();

    // The run's own meta beside the row, so the directory is self-describing without the ledger.
    let meta = serde_json::json!({ "run": job.meta, "completion": completion });
    std::fs::write(
        into.join(files::META),
        serde_json::to_vec_pretty(&meta).map_err(|e| format!("could not encode the archived meta: {e}"))?,
    )
    .map_err(|e| format!("could not write the archived meta: {e}"))?;

    // ⚠️ **Last.** A row always points at a directory that is already complete; the reverse order
    // would leave a leaderboard entry for an archive that a crash truncated.
    append(&home.join(files::LEDGER), &completion)?;
    Ok(name)
}

/// The best `limit` completions, fastest first.
///
/// ⚠️ **An unreadable line is skipped, not fatal, and a missing file is an empty leaderboard.** A
/// server nobody has finished a game on is the normal state of a fresh deployment, and a torn final
/// append — the one failure mode an append-only file has — must not take the endpoint down with it.
pub fn top(root: &Path, limit: usize) -> Vec<Completion> {
    let path = root.join(files::HALL_OF_FAME).join(files::LEDGER);
    let Ok(file) = std::fs::File::open(&path) else { return Vec::new() };
    let mut rows: Vec<Completion> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect();
    // `sort_by` rather than `sort_by_key`, because a borrowing key cannot be returned from the
    // latter and cloning `completed_at` once per comparison is a silly price for a total order.
    rows.sort_by(|a, b| a.rank().cmp(&b.rank()));
    rows.truncate(limit);
    rows
}

// ── The pieces ───────────────────────────────────────────────────────────────────────────────────

fn append(path: &Path, completion: &Completion) -> Result<(), String> {
    let line = serde_json::to_string(completion)
        .map_err(|e| format!("could not encode the ledger row: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("could not append to {}: {e}", path.display()))?;
    file.flush().map_err(|e| format!("could not flush {}: {e}", path.display()))
}

/// Copy `from` to `to`, gzipped, reading whole lines until one carries `until_seq`.
///
/// Whole lines only: a byte copy can catch the writer between its `writeln!` and its `flush` and
/// archive half an event. `u64::MAX` means "whatever is there now", for a file nothing is appending
/// to any more.
fn follow_lines(from: &Path, to: &Path, until_seq: u64) -> Result<(), String> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let Ok(file) = std::fs::File::open(from) else {
        // A run that finished the game without ever writing a transcript is not a run, but it is
        // also not a reason to refuse to file it.
        return Ok(());
    };
    let out = std::fs::File::create(to).map_err(|e| format!("could not create {}: {e}", to.display()))?;
    let mut encoder = GzEncoder::new(out, Compression::default());

    let mut reader = BufReader::new(file);
    let deadline = Instant::now() + TRANSCRIPT_FOLLOW;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // Caught up. Either we have what we came for, or the writer has not got here yet.
                if until_seq == u64::MAX || Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(FOLLOW_POLL);
            }
            Ok(_) => {
                encoder
                    .write_all(line.as_bytes())
                    .map_err(|e| format!("could not write {}: {e}", to.display()))?;
                if carries_seq(&line, until_seq) {
                    break;
                }
            }
            Err(e) => return Err(format!("could not read {}: {e}", from.display())),
        }
    }
    encoder.finish().map_err(|e| format!("could not finish {}: {e}", to.display()))?;
    Ok(())
}

/// Whether this transcript line is the event we are waiting for.
///
/// Parsed rather than substring-matched: `"seq":41` is also a substring of `"seq":410`, and the
/// difference between stopping at the right line and stopping ten events early is invisible in the
/// archive.
fn carries_seq(line: &str, seq: u64) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("seq")?.as_u64())
        .is_some_and(|found| found >= seq)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
    std::fs::create_dir_all(to).map_err(|e| format!("could not create {}: {e}", to.display()))?;
    for entry in entries.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            std::fs::copy(&source, &target)
                .map_err(|e| format!("could not copy {}: {e}", source.display()))?;
        }
    }
    Ok(())
}

/// The `meta.json` entry a run keeps for a championship that has been filed.
pub fn recorded(teams: u8, archive: String) -> ArchivedCompletion {
    ArchivedCompletion { at: super::iso8601(SystemTime::now()), teams, archive }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::Scratch;
    use crate::run::{RunDir, RunProgress, resumable};

    /// A row with the fields the ranking looks at, and plausible junk everywhere else.
    fn row(run_id: &str, playtime_seconds: u32, maxed: bool, completed_at: &str) -> Completion {
        Completion {
            archive: String::new(),
            run_id: run_id.into(),
            teams: 1,
            completed_at: completed_at.into(),
            started_at: "2026-08-10T09:30:11Z".into(),
            app_version: "1.0.0".into(),
            policy: "llm".into(),
            model: Some("gpt-test".into()),
            playtime_seconds,
            playtime: format!("{:02}:00:00", playtime_seconds / 3600),
            playtime_maxed: maxed,
            emulated_ms: 1,
            wall_ms: 2,
            turns: 3,
            completions: 4,
            prompt_tokens: 5,
            completion_tokens: 6,
            tokens_estimated: false,
            watchdog_firings: 0,
            resumes: 0,
            checkpoints: 1,
            badges: 8,
            pokedex_owned: 40,
            pokedex_seen: 60,
            money: 12345,
            party: vec![PartyMember { nickname: "VAPOREON".into(), species: "Vaporeon".into(), level: 62 }],
        }
    }

    fn ledger(root: &Path) -> PathBuf {
        root.join(files::HALL_OF_FAME).join(files::LEDGER)
    }

    /// The whole of [`archive`] with no emulator anywhere near it — and, load-bearing, the proof
    /// that what it writes cannot be mistaken for a run to resume.
    ///
    /// ⚠️ **The `resumable` assertion is the point of this test, not a flourish.** An archive is a
    /// complete run directory with a `state.gbst` in it, written *after* the run it copied, so if it
    /// landed beside the runs it would be the newest resumable thing on the volume and the next
    /// `gb serve` would continue a game that has already been won and filed.
    #[test]
    fn an_archive_carries_the_whole_run_including_the_conversation_and_is_not_resumable() {
        let scratch = Scratch::new("hof-archive");
        let (run, _, _) = RunDir::open(&scratch.0, false, "gpt-test", &|_| true).expect("a run");
        run.checkpoint(b"GBSTlive", b"sram", RunProgress::default()).expect("checkpoint");

        std::fs::write(
            run.path().join(files::TRANSCRIPT),
            "{\"seq\":0,\"type\":\"notice\"}\n{\"seq\":41,\"type\":\"agent\",\"kind\":\"hall_of_fame\"}\n",
        )
        .expect("a transcript");
        // A run old enough to have a `memories/` directory, which the archive must still carry.
        std::fs::create_dir_all(run.path().join(files::MEMORIES)).expect("memories");
        std::fs::write(run.path().join(files::MEMORIES).join("plan.md"), "beat brock").expect("a memory");
        std::fs::write(run.path().join(files::TODO), "[]").expect("a todo list");
        std::fs::write(run.path().join(files::BATTLE_SCRIPT), "{}").expect("a battle script");
        std::fs::write(run.path().join(files::HISTORY), r#"{"version":1,"messages":[]}"#)
            .expect("a saved conversation");
        std::fs::write(
            run.path().join(files::CONVERSATION),
            "{\"kind\":\"run\"}\n{\"kind\":\"message\",\"turn\":1}\n",
        )
        .expect("a conversation log");

        let job = ArchiveJob {
            root: scratch.0.clone(),
            run_dir: run.path().to_path_buf(),
            meta: run.meta(),
            state: b"GBSTwinning".to_vec(),
            sram: b"sram".to_vec(),
            until_seq: 41,
            completion: row(&run.run_id(), 22_364, false, "2026-08-12T14:30:00Z"),
        };
        let name = archive(&job).expect("the run is filed");

        let into = scratch.0.join(files::HALL_OF_FAME).join(&name);
        assert!(name.contains(&run.run_id()), "the archive names the run it holds, got {name}");
        assert_eq!(std::fs::read(into.join(files::STATE)).unwrap(), b"GBSTwinning");
        assert_eq!(std::fs::read(into.join(files::SRAM)).unwrap(), b"sram");
        assert_eq!(std::fs::read_to_string(into.join(files::MEMORIES).join("plan.md")).unwrap(), "beat brock");
        assert!(into.join(files::TODO).is_file(), "the model's plan travels with the run");
        assert!(into.join(files::BATTLE_SCRIPT).is_file(), "the battle script travels with the run");
        assert!(into.join(files::META).is_file(), "the archive is self-describing without the ledger");
        assert!(into.join(files::HISTORY).is_file(), "the conversation travels with the run");

        // The transcript is gzipped and stops at the event that announced the win.
        let gz = std::fs::read(into.join("transcript.jsonl.gz")).expect("a gzipped transcript");
        let mut inflated = String::new();
        std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&gz[..]), &mut inflated)
            .expect("it inflates");
        assert_eq!(inflated.lines().count(), 2, "both lines, and nothing invented: {inflated}");
        assert!(inflated.contains("hall_of_fame"), "the victory itself is in the archive");

        // ⚠️ **The conversation log is gzipped whole rather than followed to a seq.** Nothing keys on
        // one, so it is read to EOF — but it is still read as *lines*, because the LLM worker is
        // appending to it on another thread while this runs and a byte copy can catch it between the
        // `writeln!` and the flush.
        let gz = std::fs::read(into.join("conversation.jsonl.gz")).expect("a gzipped conversation");
        let mut log = String::new();
        std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&gz[..]), &mut log)
            .expect("it inflates");
        assert_eq!(log.lines().count(), 2, "both lines and nothing invented: {log}");
        assert!(log.contains("\"kind\":\"message\""), "what the model was actually sent is in the archive");

        let rows = top(&scratch.0, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].archive, name, "the row points at the directory that was written");
        assert_eq!(rows[0].playtime_seconds, 22_364);

        // ⚠️ The one that matters.
        let candidates = resumable(&scratch.0);
        assert_eq!(candidates, vec![run.path().to_path_buf()],
            "hall-of-fame/ must be invisible to the resume scan, or gb serve resumes a finished game");
    }

    /// Fastest first, and a clock that gave up ranks behind every honest one.
    #[test]
    fn the_ledger_ranks_on_the_cartridges_own_clock() {
        let scratch = Scratch::new("hof-rank");
        let home = scratch.0.join(files::HALL_OF_FAME);
        std::fs::create_dir_all(&home).expect("the hall of fame");
        for entry in [
            row("run-slow", 50_000, false, "2026-08-01T00:00:00Z"),
            row("run-maxed", 921_599, true, "2026-08-02T00:00:00Z"),
            row("run-fast", 22_364, false, "2026-08-03T00:00:00Z"),
        ] {
            append(&ledger(&scratch.0), &entry).expect("append");
        }

        let ranked: Vec<String> = top(&scratch.0, 10).into_iter().map(|row| row.run_id).collect();
        assert_eq!(ranked, ["run-fast", "run-slow", "run-maxed"],
            "255:59:59 is the counter giving up, not a fast run");

        assert_eq!(top(&scratch.0, 2).len(), 2, "the limit is honoured");
    }

    /// ⚠️ A torn final append is the one failure mode an append-only file has, and it must not take
    /// `/api/leaderboard` down with it. Nor may a ledger nobody has written yet.
    #[test]
    fn a_broken_line_is_skipped_and_a_missing_ledger_is_empty() {
        let scratch = Scratch::new("hof-broken");
        assert!(top(&scratch.0, 10).is_empty(), "a fresh deployment has an empty leaderboard, not an error");

        let home = scratch.0.join(files::HALL_OF_FAME);
        std::fs::create_dir_all(&home).expect("the hall of fame");
        append(&ledger(&scratch.0), &row("run-good", 100, false, "2026-08-01T00:00:00Z")).expect("append");
        // A process killed mid-write.
        std::fs::OpenOptions::new()
            .append(true)
            .open(ledger(&scratch.0))
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"{\"archive\":\"tru"))
            .expect("a torn line");

        let rows = top(&scratch.0, 10);
        assert_eq!(rows.len(), 1, "the good row survives its neighbour");
        assert_eq!(rows[0].run_id, "run-good");
    }

    /// ⚠️ `"seq":4` is a substring of `"seq":41`, so the follow has to parse rather than match — the
    /// difference between stopping at the right line and stopping thirty-seven events early is
    /// invisible once the archive is written.
    #[test]
    fn the_follow_stops_at_the_sequence_number_it_was_given() {
        assert!(carries_seq("{\"seq\":41,\"type\":\"agent\"}", 41));
        assert!(!carries_seq("{\"seq\":4,\"type\":\"agent\"}", 41));
        assert!(carries_seq("{\"seq\":42,\"type\":\"agent\"}", 41), "a gap must not run past the end");
        assert!(!carries_seq("not json at all", 41), "an unreadable line is not a stop signal");
    }
}
