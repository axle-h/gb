//! **W7 / §11** — the run directory: where a playthrough lives between processes.
//!
//! ```text
//! $GB_RUN_DIR/<run-id>/
//!     meta.json           run id, model, when it started, when it was last checkpointed
//!     state.gbst          GameBoy::save_state() — what a resume actually loads
//!     sram.bin            dump_sram(), as an ordinary .sav for anything else that reads one
//!     transcript.jsonl    one JSON object per UiEvent, append-only (see `transcript.rs`)
//!     memories/<slug>.md  W6b
//!     todo.json           W6b
//! ```
//!
//! `GB_RUN_DIR` defaults to `runs` beside the working directory, which is what makes the container
//! story a single volume mount.
//!
//! Two rules hold this together:
//!
//! 1. ⚠️ **Every file is written by rename.** A checkpoint is a `.tmp` write followed by
//!    `fs::rename`, which is atomic on every platform that matters. Without it, a SIGTERM landing
//!    inside the write leaves a truncated `state.gbst`, and the *next* start is the one that fails —
//!    at which point the run has silently lost everything since the last good checkpoint.
//! 2. ⚠️ **A run directory that cannot be read is not fatal.** A corrupt state, an unreadable
//!    `meta.json`, a directory someone deleted mid-run: each falls back to starting a fresh run and
//!    says so. The alternative — refusing to start — turns a recoverable bad checkpoint into an
//!    outage.

pub mod transcript;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Where runs live when `GB_RUN_DIR` is unset.
pub const DEFAULT_ROOT: &str = "runs";

/// File names inside a run directory. Constants because the resume path, the checkpoint path and
/// the tests all have to agree on them.
pub mod files {
    pub const META: &str = "meta.json";
    pub const STATE: &str = "state.gbst";
    pub const SRAM: &str = "sram.bin";
    pub const TRANSCRIPT: &str = "transcript.jsonl";
    pub const MEMORIES: &str = "memories";
    pub const TODO: &str = "todo.json";
}

/// What `meta.json` holds. Everything in it is for a person reading the directory later — nothing
/// here is load bearing for a resume, which needs only `state.gbst`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunMeta {
    pub run_id: String,
    /// `GB_MODEL`, or `"random"` under `--policy random`.
    pub model: String,
    pub started_at: String,
    pub last_checkpoint_at: Option<String>,
    /// Emulated milliseconds at the last checkpoint. The one number that says whether a resume
    /// actually picked up where it left off.
    #[serde(default)]
    pub emulated_ms: u64,
    #[serde(default)]
    pub checkpoints: u64,
    /// Runs before this one that it continues from, oldest first. A run resumed five times has five
    /// entries, which is how a directory listing stops looking like five unrelated attempts.
    #[serde(default)]
    pub resumed_from: Vec<String>,
}

/// One run's directory, and the only thing that writes into it.
pub struct RunDir {
    path: PathBuf,
    meta: Mutex<RunMeta>,
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
    ///
    /// With `new_run`, or when nothing resumable is there, this creates a fresh directory. Otherwise
    /// the **newest** directory holding a loadable `state.gbst` is continued — in place, so a run
    /// resumed nightly stays one directory rather than becoming thirty.
    ///
    /// `validate` is handed the bytes of a candidate `state.gbst` and answers whether they load. It
    /// is a parameter rather than a call to `GameBoy::load_state` so this module stays testable
    /// without a ROM.
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
                let mut meta = read_meta(&candidate).unwrap_or_else(|| RunMeta {
                    run_id: directory_name(&candidate),
                    model: model.to_string(),
                    started_at: iso8601(SystemTime::now()),
                    last_checkpoint_at: None,
                    emulated_ms: 0,
                    checkpoints: 0,
                    resumed_from: Vec::new(),
                });
                // The model can legitimately change between runs, and the current one is the useful
                // one to see in the directory.
                meta.model = model.to_string();
                meta.resumed_from.push(iso8601(SystemTime::now()));
                let run = Self { path: candidate, meta: Mutex::new(meta) };
                run.write_meta()?;
                return Ok((run, Origin::Resumed, Some(state)));
            }
        }

        let run_id = unique_run_id(root, SystemTime::now());
        let path = root.join(&run_id);
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("could not create the run directory {}: {e}", path.display()))?;
        let run = Self {
            meta: Mutex::new(RunMeta {
                run_id,
                model: model.to_string(),
                started_at: iso8601(SystemTime::now()),
                last_checkpoint_at: None,
                emulated_ms: 0,
                checkpoints: 0,
                resumed_from: Vec::new(),
            }),
            path,
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

    /// Write a checkpoint: the save state, the SRAM beside it, and the updated `meta.json`.
    ///
    /// ⚠️ Called from the **emulator thread**, between instructions, so it costs the stream whatever
    /// the write costs — a few milliseconds a minute. That is the reason it is not called more often
    /// and the reason it does not compress anything itself (`save_state` already has).
    pub fn checkpoint(&self, state: &[u8], sram: &[u8], emulated_ms: u64) -> Result<(), String> {
        write_atomically(&self.path.join(files::STATE), state)?;
        write_atomically(&self.path.join(files::SRAM), sram)?;
        {
            let mut meta = self.meta.lock().expect("run meta lock poisoned");
            meta.last_checkpoint_at = Some(iso8601(SystemTime::now()));
            meta.emulated_ms = emulated_ms;
            meta.checkpoints += 1;
        }
        self.write_meta()
    }

    fn write_meta(&self) -> Result<(), String> {
        let meta = self.meta.lock().expect("run meta lock poisoned");
        let json = serde_json::to_vec_pretty(&*meta).map_err(|e| format!("could not encode meta.json: {e}"))?;
        write_atomically(&self.path.join(files::META), &json)
    }
}

/// Which [`RunDir`] the process is writing to **right now**, and the only thing allowed to change it.
///
/// Before the reset endpoint there was no such question — the directory was chosen once at startup
/// and moved into four places that each kept their own copy (the host's checkpointer, the transcript
/// thread's open file, `/api/history`'s path and `/api/healthz`'s run id, and the LLM worker's
/// notes). Starting a fresh run without restarting means all five have to change together, so they
/// all read it from here instead.
///
/// ⚠️ **This does not make a run directory multi-writer.** The rule in the module note still holds
/// and is what `RwLock` is protecting: exactly one `RunDir` is current at a time, the emulator thread
/// is the only caller of [`Self::start_new`], and it checkpoints the outgoing run before swapping —
/// so the directory that is left behind is complete and resumable rather than truncated mid-play.
pub struct CurrentRun {
    root: PathBuf,
    model: String,
    inner: std::sync::RwLock<Arc<RunDir>>,
}

impl CurrentRun {
    pub fn new(root: PathBuf, model: String, run: RunDir) -> Self {
        Self { root, model, inner: std::sync::RwLock::new(Arc::new(run)) }
    }

    /// The run directory as of now. Cheap enough for the transcript thread to ask per event.
    pub fn get(&self) -> Arc<RunDir> {
        Arc::clone(&self.inner.read().expect("current run lock poisoned"))
    }

    /// Open a fresh run directory beside the current one and make it current, returning it.
    ///
    /// The `validate` callback [`RunDir::open`] takes is never reached: `new_run` short-circuits the
    /// resume scan, which is the only thing that reads a candidate state.
    pub fn start_new(&self) -> Result<Arc<RunDir>, String> {
        let (run, _, _) = RunDir::open(&self.root, true, &self.model, &|_| false)?;
        let run = Arc::new(run);
        *self.inner.write().expect("current run lock poisoned") = Arc::clone(&run);
        Ok(run)
    }
}

/// Every child of `root` that has a `state.gbst`, newest first.
///
/// "Newest" is the state file's modification time rather than the directory name, because a resumed
/// run keeps the name it was created with and would otherwise sort behind a fresh one that was
/// abandoned immediately.
fn resumable(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut candidates: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
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

fn directory_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "run".into())
}

/// ⚠️ **Write, then rename.** See the module note: a half-written `state.gbst` is a failure that
/// only shows up on the next start, by which time the good copy is gone.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("could not write {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("could not replace {}: {e}", path.display()))
}

/// `run-20260810-143205`, and `-2` appended if that second already has a directory.
fn unique_run_id(root: &Path, now: SystemTime) -> String {
    let base = format!("run-{}", compact_timestamp(now));
    if !root.join(&base).exists() {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !root.join(candidate).exists())
        .expect("the range is unbounded")
}

// ── Time, without a dependency ───────────────────────────────────────────────────────────────────

/// `2026-08-10T14:32:05Z`. UTC, because a run directory is read by whoever is on call and a local
/// timestamp with no offset in it is a lie half the year.
pub fn iso8601(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The same instant as a directory name: `20260810-143205`. Sorts chronologically as a string, which
/// is the only property the listing needs.
pub fn compact_timestamp(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Seconds since the epoch to a civil date, by Howard Hinnant's `civil_from_days`. Twelve lines
/// against a `chrono` dependency whose entire value here would be these twelve lines.
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    /// A directory of our own, cleaned up by the caller. No `tempfile` dependency for six lines.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new(name: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("gb-{name}-{}-{unique}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a temp directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn epoch(seconds: u64) -> SystemTime {
        std::time::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// The date arithmetic, against dates whose answers are known independently — including a leap
    /// day and a century that is not a leap year.
    #[test]
    fn the_clock_agrees_with_a_calendar() {
        assert_eq!(iso8601(epoch(0)), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(epoch(1_770_000_000)), "2026-02-02T02:40:00Z");
        // 2024-02-29, a leap day; and 2100-03-01, the day after a February that has no 29th.
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

        run.checkpoint(b"a save state", b"sram", 61_000).expect("checkpoint");
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

    /// `--new-run` leaves the old directory alone and starts beside it.
    #[test]
    fn new_run_starts_beside_the_old_one_without_touching_it() {
        let scratch = Scratch::new("runnew");
        let (first, _, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("first");
        first.checkpoint(b"first state", b"", 0).expect("checkpoint");

        let (second, origin, state) = RunDir::open(&scratch.0, true, "m", &|_| true).expect("second");
        assert_eq!(origin, Origin::Fresh);
        assert!(state.is_none());
        assert_ne!(second.run_id(), first.run_id());
        assert_eq!(
            std::fs::read(first.path().join(files::STATE)).expect("still there"),
            b"first state",
        );
    }

    /// ⚠️ §11's rule: a checkpoint that does not load is not a reason to refuse to start. The next
    /// resumable run down the list is tried, and a fresh one is the floor.
    #[test]
    fn a_corrupt_checkpoint_falls_through_to_a_fresh_run() {
        let scratch = Scratch::new("runcorrupt");
        let (broken, _, _) = RunDir::open(&scratch.0, false, "m", &|_| true).expect("first");
        broken.checkpoint(b"not a save state", b"", 0).expect("checkpoint");
        let broken_id = broken.run_id();

        let (run, origin, state) =
            RunDir::open(&scratch.0, false, "m", &|bytes| bytes.starts_with(b"GBST")).expect("opens");
        assert_eq!(origin, Origin::Fresh, "the only candidate was unloadable");
        assert!(state.is_none());
        assert_ne!(run.run_id(), broken_id);

        // …and a *good* checkpoint beside it wins, however old the broken one is.
        run.checkpoint(b"GBSTgood", b"", 0).expect("checkpoint");
        let (resumed, origin, state) =
            RunDir::open(&scratch.0, false, "m", &|bytes| bytes.starts_with(b"GBST")).expect("opens");
        assert_eq!(origin, Origin::Resumed);
        assert_eq!(state.as_deref(), Some(&b"GBSTgood"[..]));
        assert_eq!(resumed.run_id(), run.run_id());
    }

    /// ⚠️ The rename. A `.tmp` left behind by a killed process must never be mistaken for the file,
    /// and the real file must never be seen half-written.
    #[test]
    fn a_checkpoint_is_written_by_rename() {
        let scratch = Scratch::new("runatomic");
        let target = scratch.0.join("state.gbst");
        std::fs::write(scratch.0.join("state.tmp"), b"leftover rubbish").expect("write");

        write_atomically(&target, b"the real thing").expect("write");
        assert_eq!(std::fs::read(&target).expect("read"), b"the real thing");
        assert!(!scratch.0.join("state.tmp").exists(), "the temporary file is consumed by the rename");
    }

    /// Two runs created in the same second must not collide.
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
