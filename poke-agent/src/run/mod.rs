//! The run directory: where a playthrough lives between processes. `files` names what is in it.

pub mod hall_of_fame;
pub mod transcript;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Where runs live when `GB_RUN_DIR` is unset.
pub const DEFAULT_ROOT: &str = "runs";

/// File names inside a run directory.
pub mod files {
    pub const META: &str = "meta.json";
    pub const STATE: &str = "state.gbst";
    pub const SRAM: &str = "sram.bin";
    pub const TRANSCRIPT: &str = "transcript.jsonl";
    /// Legacy: nothing writes it, and only the archiver reads it.
    pub const MEMORIES: &str = "memories";
    /// The model's plan: what outlives a compaction.
    pub const TODO: &str = "todo.json";
    /// The model's battle script and whether it is armed. See [`crate::llm::battle_script`].
    pub const BATTLE_SCRIPT: &str = "battle-script.json";
    /// The live conversation, rewritten each turn: what a restart resumes on. See [`crate::llm::history`].
    pub const HISTORY: &str = "history.json";
    /// Every message ever appended, plus a marker line per compaction. Append-only, and read by
    /// nothing in this program.
    pub const CONVERSATION: &str = "conversation.jsonl";
    /// One subdirectory per `press_buttons` escape hatch; nothing reads it back. See [`crate::llm::incident`].
    pub const PRESS_BUTTONS: &str = "press-buttons";

    /// One directory per `report_issue` call, with the screen and a save state. See [`crate::llm::incident`].
    pub const ISSUES: &str = "issues";
    /// Where finished runs are filed, one level below the root, where `resumable` does not look.
    pub const HALL_OF_FAME: &str = "hall-of-fame";
    /// The leaderboard itself, inside `HALL_OF_FAME`.
    pub const LEDGER: &str = "ledger.jsonl";
}

/// What one process has contributed to a run since it opened it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunProgress {
    pub emulated_ms: u64,
    /// Wall clock spent playing, not `now - started_at`: a run resumed nightly spans days.
    pub wall_ms: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Completions billed; a turn that reads before it decides costs several.
    pub completions: u64,
    /// Decisions that landed, not turn ids: a turn id is `llm::worker`'s cancellation generation.
    pub turns: u64,
    /// Times the stuck-run watchdog fired; zero in a healthy run.
    pub watchdog_firings: u64,
}

/// One Hall of Fame entry this run has had archived — see [`hall_of_fame`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArchivedCompletion {
    pub at: String,
    /// `wNumHoFTeams` after the increment: `2` is a second championship in the same save.
    pub teams: u8,
    /// The archive directory's name, relative to `<root>/hall-of-fame/`.
    pub archive: String,
}

/// What `meta.json` holds, for a person reading the directory; a resume needs only `state.gbst`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunMeta {
    pub run_id: String,
    /// `GB_MODEL`, or `"random"` under `--policy random`.
    pub model: String,
    pub started_at: String,
    pub last_checkpoint_at: Option<String>,
    /// Emulated milliseconds across every process that has played the run.
    #[serde(default)]
    pub emulated_ms: u64,
    /// Wall clock spent playing, ditto. See [`RunProgress::wall_ms`].
    #[serde(default)]
    pub wall_ms: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub completions: u64,
    #[serde(default)]
    pub turns: u64,
    #[serde(default)]
    pub watchdog_firings: u64,
    #[serde(default)]
    pub checkpoints: u64,
    /// When each resume happened, oldest first.
    #[serde(default)]
    pub resumed_from: Vec<String>,
    /// Championships this run has had filed. Empty for every run that has not finished the game.
    #[serde(default)]
    pub completed: Vec<ArchivedCompletion>,
}

impl RunMeta {
    /// A fresh run's meta, with every total at zero.
    fn new(run_id: String, model: String) -> Self {
        Self {
            run_id,
            model,
            started_at: iso8601(SystemTime::now()),
            last_checkpoint_at: None,
            emulated_ms: 0,
            wall_ms: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            completions: 0,
            turns: 0,
            watchdog_firings: 0,
            checkpoints: 0,
            resumed_from: Vec::new(),
            completed: Vec::new(),
        }
    }

    /// The totals as read: the baseline the next checkpoint rebases onto.
    fn progress(&self) -> RunProgress {
        RunProgress {
            emulated_ms: self.emulated_ms,
            wall_ms: self.wall_ms,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            completions: self.completions,
            turns: self.turns,
            watchdog_firings: self.watchdog_firings,
        }
    }

    fn set_progress(&mut self, total: RunProgress) {
        let RunProgress {
            emulated_ms,
            wall_ms,
            prompt_tokens,
            completion_tokens,
            completions,
            turns,
            watchdog_firings,
        } = total;
        self.emulated_ms = emulated_ms;
        self.wall_ms = wall_ms;
        self.prompt_tokens = prompt_tokens;
        self.completion_tokens = completion_tokens;
        self.completions = completions;
        self.turns = turns;
        self.watchdog_firings = watchdog_firings;
    }
}

impl std::ops::Add for RunProgress {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            emulated_ms: self.emulated_ms + other.emulated_ms,
            wall_ms: self.wall_ms + other.wall_ms,
            prompt_tokens: self.prompt_tokens + other.prompt_tokens,
            completion_tokens: self.completion_tokens + other.completion_tokens,
            completions: self.completions + other.completions,
            turns: self.turns + other.turns,
            watchdog_firings: self.watchdog_firings + other.watchdog_firings,
        }
    }
}

/// One run's directory, and the only thing that writes into it.
pub struct RunDir {
    path: PathBuf,
    meta: Mutex<RunMeta>,
    /// The totals `meta.json` held when this directory was opened.
    baseline: RunProgress,
}

/// How the directory was chosen, for the line the server prints at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Nothing to resume, or `--new-run`.
    Fresh,
    /// A previous run's `state.gbst` was loaded.
    Resumed,
}

impl RunDir {
    /// Resolve a run directory and open it.
    pub fn open(
        root: &Path,
        new_run: bool,
        model: &str,
        validate: &dyn Fn(&[u8]) -> bool,
    ) -> Result<(Self, Origin, Option<Vec<u8>>), String> {
        std::fs::create_dir_all(root)
            .map_err(|e| format!("could not create the run directory {}: {e}", root.display()))?;

        if !new_run {
            for candidate in resumable(root) {
                let state = match std::fs::read(candidate.join(files::STATE)) {
                    Ok(bytes) if validate(&bytes) => bytes,
                    Ok(_) => {
                        eprintln!(
                            "run {}: state.gbst does not load — starting a fresh run instead",
                            candidate.display(),
                        );
                        continue;
                    }
                    Err(_) => continue,
                };
                let mut meta = read_meta(&candidate).unwrap_or_else(|| {
                    RunMeta::new(directory_name(&candidate), model.to_string())
                });
                // The model can change between processes; the current one is the one to record.
                meta.model = model.to_string();
                meta.resumed_from.push(iso8601(SystemTime::now()));
                // Read before the first checkpoint overwrites it, so the totals stay the run's.
                let baseline = meta.progress();
                let run = Self { path: candidate, meta: Mutex::new(meta), baseline };
                run.write_meta()?;
                return Ok((run, Origin::Resumed, Some(state)));
            }
        }

        let run_id = unique_run_id(root, SystemTime::now());
        let path = root.join(&run_id);
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("could not create the run directory {}: {e}", path.display()))?;
        let run = Self {
            meta: Mutex::new(RunMeta::new(run_id, model.to_string())),
            path,
            baseline: RunProgress::default(),
        };
        run.write_meta()?;
        Ok((run, Origin::Fresh, None))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn transcript_path(&self) -> PathBuf {
        self.path.join(files::TRANSCRIPT)
    }

    pub fn run_id(&self) -> String {
        self.meta.lock().expect("run meta lock poisoned").run_id.clone()
    }

    /// `meta.json` as it stands.
    pub fn meta(&self) -> RunMeta {
        self.meta.lock().expect("run meta lock poisoned").clone()
    }

    /// What the run had been played for when this process opened it; zero for a fresh one.
    pub fn baseline(&self) -> RunProgress {
        self.baseline
    }

    /// Write a checkpoint: the save state, the SRAM beside it, and the updated `meta.json`.
    pub fn checkpoint(&self, state: &[u8], sram: &[u8], progress: RunProgress) -> Result<(), String> {
        write_atomically(&self.path.join(files::STATE), state)?;
        write_atomically(&self.path.join(files::SRAM), sram)?;
        {
            let mut meta = self.meta.lock().expect("run meta lock poisoned");
            meta.last_checkpoint_at = Some(iso8601(SystemTime::now()));
            meta.set_progress(self.baseline + progress);
            meta.checkpoints += 1;
        }
        self.write_meta()
    }

    /// Note in `meta.json` that this run has been filed in the Hall of Fame.
    pub fn record_completion(&self, entry: ArchivedCompletion) -> Result<(), String> {
        {
            let mut meta = self.meta.lock().expect("run meta lock poisoned");
            if meta.completed.iter().any(|already| already.teams == entry.teams) {
                return Ok(());
            }
            meta.completed.push(entry);
        }
        self.write_meta()
    }

    /// Whether this run has already had `teams` filed.
    pub fn already_archived(&self, teams: u8) -> bool {
        self.meta
            .lock()
            .expect("run meta lock poisoned")
            .completed
            .iter()
            .any(|already| already.teams == teams)
    }

    fn write_meta(&self) -> Result<(), String> {
        let meta = self.meta.lock().expect("run meta lock poisoned");
        let json = serde_json::to_vec_pretty(&*meta).map_err(|e| format!("could not encode meta.json: {e}"))?;
        write_atomically(&self.path.join(files::META), &json)
    }
}

/// Which [`RunDir`] the process is writing to right now, and the only thing allowed to change it.
pub struct CurrentRun {
    root: PathBuf,
    model: String,
    inner: std::sync::RwLock<Arc<RunDir>>,
}

impl CurrentRun {
    pub fn new(root: PathBuf, model: String, run: RunDir) -> Self {
        Self { root, model, inner: std::sync::RwLock::new(Arc::new(run)) }
    }

    /// `$GB_RUN_DIR` — where every run, and the Hall of Fame beside them, lives.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// What is playing: a property of the process, not the run, so it is rewritten into
    /// `meta.json` on every open.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The run directory as of now. Cheap enough for the transcript thread to ask per event.
    pub fn get(&self) -> Arc<RunDir> {
        Arc::clone(&self.inner.read().expect("current run lock poisoned"))
    }

    /// Open a fresh run directory beside the current one and make it current, returning it.
    pub fn start_new(&self) -> Result<Arc<RunDir>, String> {
        let (run, _, _) = RunDir::open(&self.root, true, &self.model, &|_| false)?;
        let run = Arc::new(run);
        *self.inner.write().expect("current run lock poisoned") = Arc::clone(&run);
        Ok(run)
    }
}

/// Every child of `root` that has a `state.gbst`, newest first.
pub(crate) fn resumable(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut candidates: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| path.file_name() != Some(std::ffi::OsStr::new(files::HALL_OF_FAME)))
        .filter_map(|path| {
            let modified = std::fs::metadata(path.join(files::STATE)).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates.into_iter().map(|(_, path)| path).collect()
}

fn read_meta(path: &Path) -> Option<RunMeta> {
    serde_json::from_slice(&std::fs::read(path.join(files::META)).ok()?).ok()
}

pub(crate) fn directory_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "run".into())
}

/// Write, then rename: a half-written `state.gbst` only shows on the next start, when the good
/// copy is gone.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("could not write {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("could not replace {}: {e}", path.display()))
}

/// `run-20260810-143205`, and `-2` appended if that second already has a directory.
fn unique_run_id(root: &Path, now: SystemTime) -> String {
    unique_dir(root, &format!("run-{}", compact_timestamp(now)))
}

/// `base`, or `base-2`, `base-3` … — the first that is not taken inside `root`.
pub(crate) fn unique_dir(root: &Path, base: &str) -> String {
    if !root.join(base).exists() {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !root.join(candidate).exists())
        .expect("the range is unbounded")
}

pub fn iso8601(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The same instant as a directory name, `20260810-143205`, which sorts chronologically.
pub fn compact_timestamp(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Seconds since the epoch to a civil date, by Howard Hinnant's `civil_from_days`.
fn civil(time: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let seconds = time.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 } as u32;
    let year = year + i64::from(month <= 2);

    (year, month, day, (time_of_day / 3600) as u32, (time_of_day / 60 % 60) as u32, (time_of_day % 60) as u32)
}

/// A scratch directory of its own, for a test that needs a run directory. Not `#[cfg(test)]`:
/// `poke-agent-web`'s host tests are in another crate and a `cfg(test)` item does not cross one.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("gb-{name}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temp directory");
        Self(path)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    pub(crate) use super::Scratch;

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn epoch(seconds: u64) -> SystemTime {
        std::time::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// The date arithmetic against known dates, including a leap day and a non-leap century.
    #[test]
    fn the_clock_agrees_with_a_calendar() {
        assert_eq!(iso8601(epoch(0)), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(epoch(1_770_000_000)), "2026-02-02T02:40:00Z");
        assert_eq!(iso8601(epoch(1_709_164_800)), "2024-02-29T00:00:00Z");
        assert_eq!(iso8601(epoch(4_107_542_400)), "2100-03-01T00:00:00Z");
        assert_eq!(compact_timestamp(epoch(1_770_000_000)), "20260202-024000");
    }

    /// A fresh root has nothing to resume, and what it creates can then be resumed.
    #[test]
    fn a_fresh_run_becomes_a_resumable_one() {
        let scratch = Scratch::new("runfresh");
        let (run, origin, state) =
            RunDir::open(&scratch.0, false, "gpt-test", &|_| true).expect("a fresh run");
        assert_eq!(origin, Origin::Fresh);
        assert!(state.is_none(), "nothing to resume from");
        assert!(run.path().join(files::META).is_file(), "meta.json is written immediately");
        let id = run.run_id();

        run.checkpoint(b"a save state", b"sram", RunProgress { emulated_ms: 61_000, ..Default::default() }).expect("checkpoint");
        drop(run);

        let (resumed, origin, state) =
            RunDir::open(&scratch.0, false, "gpt-test", &|_| true).expect("a resumed run");
        assert_eq!(origin, Origin::Resumed);
        assert_eq!(state.as_deref(), Some(&b"a save state"[..]));
        assert_eq!(resumed.run_id(), id, "a resume continues the run in place rather than forking it");

        let meta = read_meta(resumed.path()).expect("meta.json round-trips");
        assert_eq!(meta.checkpoints, 1);
        assert_eq!(meta.emulated_ms, 61_000);
        assert_eq!(meta.resumed_from.len(), 1, "the resume is recorded");
    }

    #[test]
    fn a_resume_continues_the_totals_rather_than_restarting_them() {
        let scratch = Scratch::new("runtotals");
        let (run, _, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("a run");
        run.checkpoint(b"one", b"", RunProgress {
            emulated_ms: 61_000, wall_ms: 90_000, prompt_tokens: 2_000, turns: 7, ..Default::default()
        }).expect("checkpoint");
        drop(run);

        let (resumed, origin, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("a resume");
        assert_eq!(origin, Origin::Resumed);
        // The second process's own figures start from zero, exactly as the host's do.
        resumed.checkpoint(b"two", b"", RunProgress {
            emulated_ms: 30_000, wall_ms: 40_000, prompt_tokens: 1_000, turns: 3, ..Default::default()
        }).expect("checkpoint");

        let meta = resumed.meta();
        assert_eq!(meta.emulated_ms, 91_000, "emulated time is the run's, not this process's");
        assert_eq!(meta.wall_ms, 130_000);
        assert_eq!(meta.prompt_tokens, 3_000);
        assert_eq!(meta.turns, 10);

        // A second checkpoint in the same process rebases rather than adds.
        resumed.checkpoint(b"three", b"", RunProgress {
            emulated_ms: 30_000, wall_ms: 40_000, prompt_tokens: 1_000, turns: 3, ..Default::default()
        }).expect("checkpoint");
        assert_eq!(resumed.meta().emulated_ms, 91_000, "a checkpoint is a rebase, not an addition");
    }

    /// Every total is `#[serde(default)]`, so an older `meta.json` still opens.
    #[test]
    fn a_meta_json_from_before_the_totals_still_parses() {
        let scratch = Scratch::new("runoldmeta");
        let path = scratch.0.join("run-old");
        std::fs::create_dir_all(&path).expect("a run directory");
        std::fs::write(path.join(files::STATE), b"GBSTold").expect("a state");
        std::fs::write(
            path.join(files::META),
            br#"{"run_id":"run-old","model":"m","started_at":"2026-01-01T00:00:00Z",
                "last_checkpoint_at":null,"emulated_ms":5000,"checkpoints":3}"#,
        )
        .expect("an old meta.json");

        let (run, origin, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("it opens");
        assert_eq!(origin, Origin::Resumed);
        let meta = run.meta();
        assert_eq!(meta.emulated_ms, 5_000, "what was there is kept as the baseline");
        assert_eq!(meta.wall_ms, 0, "what was not there defaults");
        assert!(meta.completed.is_empty());

        run.checkpoint(b"GBSTnew", b"", RunProgress { emulated_ms: 1_000, ..Default::default() })
            .expect("checkpoint");
        assert_eq!(run.meta().emulated_ms, 6_000, "and the old figure is continued, not replaced");
    }

    /// A restart from a checkpoint just before `wNumHoFTeams` moved does not file the victory twice.
    #[test]
    fn a_championship_is_only_recorded_once() {
        let scratch = Scratch::new("runhof");
        let (run, _, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("a run");
        assert!(!run.already_archived(1));

        run.record_completion(hall_of_fame::recorded(1, "hof-a".into())).expect("recorded");
        run.record_completion(hall_of_fame::recorded(1, "hof-b".into())).expect("ignored");
        assert!(run.already_archived(1));
        assert_eq!(run.meta().completed.len(), 1, "the same championship is filed once");
        assert_eq!(run.meta().completed[0].archive, "hof-a", "the first filing wins");

        // A second championship in the same save is a genuinely new one.
        run.record_completion(hall_of_fame::recorded(2, "hof-c".into())).expect("recorded");
        assert_eq!(run.meta().completed.len(), 2);
    }

    #[test]
    fn new_run_starts_beside_the_old_one_without_touching_it() {
        let scratch = Scratch::new("runnew");
        let (first, _, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("first");
        first.checkpoint(b"first state", b"", RunProgress::default()).expect("checkpoint");

        let (second, origin, state) = RunDir::open(&scratch.0, true, "m", &|_| true).expect("second");
        assert_eq!(origin, Origin::Fresh);
        assert!(state.is_none());
        assert_ne!(second.run_id(), first.run_id());
        assert_eq!(
            std::fs::read(first.path().join(files::STATE)).expect("still there"),
            b"first state",
        );
    }

    #[test]
    fn a_corrupt_checkpoint_falls_through_to_a_fresh_run() {
        let scratch = Scratch::new("runcorrupt");
        let (broken, _, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("first");
        broken.checkpoint(b"not a save state", b"", RunProgress::default()).expect("checkpoint");
        let broken_id = broken.run_id();

        let (run, origin, state) =
            RunDir::open(&scratch.0, false, "m", &|bytes| bytes.starts_with(b"GBST")).expect("opens");
        assert_eq!(origin, Origin::Fresh, "the only candidate was unloadable");
        assert!(state.is_none());
        assert_ne!(run.run_id(), broken_id);

        // …and a *good* checkpoint beside it wins, however old the broken one is.
        run.checkpoint(b"GBSTgood", b"", RunProgress::default()).expect("checkpoint");
        let (resumed, origin, state) =
            RunDir::open(&scratch.0, false, "m", &|bytes| bytes.starts_with(b"GBST")).expect("opens");
        assert_eq!(origin, Origin::Resumed);
        assert_eq!(state.as_deref(), Some(&b"GBSTgood"[..]));
        assert_eq!(resumed.run_id(), run.run_id());
    }

    #[test]
    fn a_checkpoint_is_written_by_rename() {
        let scratch = Scratch::new("runatomic");
        let target = scratch.0.join("state.gbst");
        std::fs::write(scratch.0.join("state.tmp"), b"leftover rubbish").expect("write");

        write_atomically(&target, b"the real thing").expect("write");
        assert_eq!(std::fs::read(&target).expect("read"), b"the real thing");
        assert!(!scratch.0.join("state.tmp").exists(), "the temporary file is consumed by the rename");
    }

    #[test]
    fn run_ids_do_not_collide_within_a_second() {
        let scratch = Scratch::new("runids");
        let now = epoch(1_770_000_000);
        let first = unique_run_id(&scratch.0, now);
        std::fs::create_dir_all(scratch.0.join(&first)).expect("mkdir");
        let second = unique_run_id(&scratch.0, now);
        assert_eq!(first, "run-20260202-024000");
        assert_eq!(second, "run-20260202-024000-2");
    }
}
