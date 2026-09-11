//! The end of the game: a finished run's permanent record under `hall-of-fame/`, and the
//! leaderboard built from its `ledger.jsonl`. [`archive`] copies each run file by name, so a new
//! one is dropped until the archive test asserts it.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::{ArchivedCompletion, RunMeta, files, unique_dir};

/// How long [`archive`] will wait for the transcript writer to catch up to the completion event.
const TRANSCRIPT_FOLLOW: Duration = Duration::from_secs(5);

/// How often the follow re-checks a file it has read to the end of.
const FOLLOW_POLL: Duration = Duration::from_millis(50);

/// One finished playthrough, as it appears in the ledger and on `/api/leaderboard`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Completion {
    /// The archive directory's name, relative to `<root>/hall-of-fame/`.
    pub archive: String,
    pub run_id: String,
    /// `wNumHoFTeams` after the increment: `2` is a second championship in the same save.
    pub teams: u8,
    pub completed_at: String,
    pub started_at: String,
    /// `crate::cli::VERSION` at the time — which build played this.
    pub app_version: String,
    /// [`crate::pokemon::policy::Policy::name`].
    pub policy: String,
    /// `GB_MODEL`. `None` under any policy that is not an LLM.
    pub model: Option<String>,

    /// The cartridge's own play clock, in seconds.
    pub playtime_seconds: u32,
    /// `HH:MM:SS`, for a person. Never sort on it: the hours field runs to 255, so it is not
    /// fixed-width.
    pub playtime: String,
    /// `wPlayTimeMaxed`: the clock stopped at 255:59:59, so the run ranks last.
    pub playtime_maxed: bool,
    /// Emulated milliseconds over the run's whole life, across every process that played it.
    pub emulated_ms: u64,
    /// Wall clock spent playing, ditto.
    pub wall_ms: u64,

    pub turns: u64,
    pub completions: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The endpoint reported no `usage`, so these are our own estimate.
    pub tokens_estimated: bool,
    /// Times the stuck-run watchdog fired. Zero in a healthy run.
    pub watchdog_firings: u64,
    /// How many times the run was resumed by a new process.
    pub resumes: usize,
    pub checkpoints: u64,

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
    /// Where this run sorts.
    fn rank(&self) -> (bool, u32, &str) {
        (self.playtime_maxed, self.playtime_seconds, &self.completed_at)
    }
}

/// Everything [`archive`] needs, captured on the emulator thread at the instant of victory.
pub struct ArchiveJob {
    /// `$GB_RUN_DIR`.
    pub root: PathBuf,
    /// The run directory being filed, still the current run.
    pub run_dir: PathBuf,
    pub meta: RunMeta,
    /// `gb.save_state()` at the moment the counter moved, held rather than re-read from the
    /// checkpoint so the two cannot differ.
    pub state: Vec<u8>,
    pub sram: Vec<u8>,
    /// The seq `publish_event` returned for the completion event: where the transcript follow stops.
    pub until_seq: u64,
    /// The row to append; [`archive`] fills in its `archive` field.
    pub completion: Completion,
}

/// Copy the run out and append its row. Returns the archive directory's name.
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

    // Followed, not copied: another thread writes the completion event after this is triggered.
    follow_lines(
        &job.run_dir.join(files::TRANSCRIPT),
        &into.join(format!("{}.gz", files::TRANSCRIPT)),
        job.until_seq,
    )?;
    // The rotated half, if this run went on long enough to have one.
    let rotated = job.run_dir.join(files::TRANSCRIPT).with_extension("jsonl.1");
    if rotated.exists() {
        follow_lines(&rotated, &into.join("transcript.jsonl.1.gz"), u64::MAX)?;
    }

    copy_tree(&job.run_dir.join(files::MEMORIES), &into.join(files::MEMORIES))?;
    let todo = job.run_dir.join(files::TODO);
    if todo.exists() {
        std::fs::copy(&todo, into.join(files::TODO))
            .map_err(|e| format!("could not copy {}: {e}", todo.display()))?;
    }
    let script = job.run_dir.join(files::BATTLE_SCRIPT);
    if script.exists() {
        std::fs::copy(&script, into.join(files::BATTLE_SCRIPT))
            .map_err(|e| format!("could not copy {}: {e}", script.display()))?;
    }

    // The conversation, both halves.
    let history = job.run_dir.join(files::HISTORY);
    if history.exists() {
        std::fs::copy(&history, into.join(files::HISTORY))
            .map_err(|e| format!("could not copy {}: {e}", history.display()))?;
    }
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

    // Last, so a ledger row never points at a partial archive.
    append(&home.join(files::LEDGER), &completion)?;
    Ok(name)
}

/// The best `limit` completions, fastest first.
pub fn top(root: &Path, limit: usize) -> Vec<Completion> {
    let path = root.join(files::HALL_OF_FAME).join(files::LEDGER);
    let Ok(file) = std::fs::File::open(&path) else { return Vec::new() };
    let mut rows: Vec<Completion> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect();
    rows.sort_by(|a, b| a.rank().cmp(&b.rank()));
    rows.truncate(limit);
    rows
}

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
fn follow_lines(from: &Path, to: &Path, until_seq: u64) -> Result<(), String> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let Ok(file) = std::fs::File::open(from) else {
        // A missing transcript is not a reason to refuse to file the run.
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
                // Caught up.
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
    use crate::run::Scratch;
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

    /// [`archive`] with no emulator: it carries every file, and what it writes is not resumable.
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

        // The conversation log is gzipped whole rather than followed to a seq.
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

    /// A torn final append must not take `/api/leaderboard` down with it.
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

    /// `"seq":4` is a substring of `"seq":41`, so the follow parses rather than matches.
    #[test]
    fn the_follow_stops_at_the_sequence_number_it_was_given() {
        assert!(carries_seq("{\"seq\":41,\"type\":\"agent\"}", 41));
        assert!(!carries_seq("{\"seq\":4,\"type\":\"agent\"}", 41));
        assert!(carries_seq("{\"seq\":42,\"type\":\"agent\"}", 41), "a gap must not run past the end");
        assert!(!carries_seq("not json at all", 41), "an unreadable line is not a stop signal");
    }
}
