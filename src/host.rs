//! **W1.1** — the headless emulator host: a `GameBoy`, a `PokemonAgent` and a video encoder on one
//! synchronous thread, publishing into [`Published`] for the HTTP layer to read.
//!
//! This is the server-side counterpart of `src/sdl/render.rs`, and the pacing algorithm is
//! transplanted from it unchanged: accumulate wall clock into `since_last_update`, drain it in
//! `cycle_duration` steps, and credit `ahead_by_cycles` for the overshoot `gb.run` produces by
//! finishing the instruction it is in the middle of.
//!
//! **`render.rs` is deliberately not refactored to use this.** It has F1–F12 debug affordances
//! tangled into the same event pump, and untangling them would buy nothing but risk in the one path
//! that is exercised by hand rather than by tests. The duplication is about thirty lines.
//!
//! The emulator is `&mut`-single-threaded and stays that way. Everything asynchronous talks to it
//! through [`Published`], which is read-only from the other side.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::model::Model;
use crate::pokemon::agent::{AgentEvent, PokemonAgent};
use crate::pokemon::map_metadata::MapMetadataCache;
use crate::pokemon::policy::Policy;
use crate::pokemon::{PokemonApi, PokemonApiTrait, observe};
use crate::run::{CurrentRun, RunProgress};
use crate::web::published::{
    FrameSnapshot, Published, RunStatus, StatusSnapshot, UiEventBody, now_ms,
};
use crate::web::video::{Frame, VideoEncoder};

/// One machine cycle at the real hardware's rate — the unit the pacing loop spends wall clock in.
const REALTIME_CYCLE_DURATION: Duration = MachineCycles::from_m(1).to_duration();

/// How long the loop sleeps when it is ahead of the clock. Short enough that a 30 fps video tick
/// lands within a millisecond of where it should, long enough that an idle server does not spin a
/// core the way `render.rs`'s `sleep(0)` does.
const IDLE_SLEEP: Duration = Duration::from_millis(1);

/// The most wall clock one iteration will try to make up. Without a cap, a process descheduled for
/// a few seconds — a container throttled, a laptop suspended — comes back owing thousands of frames
/// and emulates them flat out, which on a livestream looks like the game fast-forwarding for no
/// reason. Better to drop the time.
const MAX_CATCHUP: Duration = Duration::from_millis(250);

/// The emulator thread's stack.
///
/// ⚠️ **The default 2 MiB is not enough to *construct* the machine in a debug build**, and the
/// failure is an abort inside `Core::try_new` before a single cycle is emulated — a stack overflow,
/// so no panic message, no backtrace by default, and nothing that points at a cause. It looks like
/// the emulator is broken; it is the build profile.
///
/// The machine is large by value (`MMU` 142 KiB, `Core` and `GameBoy` 142 KiB, `PPU` 107 KiB,
/// `EmulatorHost` 143 KiB) and the construction chain moves it four times. Optimised, every one of
/// those moves is elided; unoptimised, each intermediate gets a slot of its own, which the stack
/// probes measure as:
///
/// | frame | debug | release |
/// |---|---|---|
/// | `EmulatorHost::new` | 568 KiB | 424 KiB |
/// | `GameBoy::new` | 140 KiB | elided |
/// | `Core::new` | 140 KiB | elided |
/// | `Core::try_new` | 848 KiB | 280 KiB |
/// | on the way down | **~1.7 MiB** | ~0.7 MiB |
///
/// So release has always fitted with room to spare and debug has always been just over. ⚠️ **This
/// costs release nothing**: a thread stack is reserved address space, and only the pages actually
/// touched are ever committed — so raising the number does not raise the process's footprint by a
/// byte. It buys `cargo run` behaving the same way as `cargo run --release`, which is worth more
/// than the tidiness of leaving the default alone.
///
/// ⚠️ Note this is the *construction* peak, not a running one: nothing in `tick` comes close, so
/// this is not a ceiling that ordinary work is growing towards.
const EMULATOR_STACK: usize = 8 * 1024 * 1024;

pub struct HostConfig {
    /// Emulation speed as a multiple of real time. 1.0 for a livestream; the tests use a large
    /// number so a bounded run covers real game time in a fraction of the wall clock.
    pub target_speed: f64,
    /// Wall-clock spacing of video messages, independent of the emulated frame rate so that running
    /// fast does not multiply bandwidth.
    pub video_interval: Duration,
    /// How often the game state is **sampled**. Every sample costs a `game_state()` read, which is
    /// not free, and the sample rate is also the ceiling on how promptly a change can be reported.
    pub status_interval: Duration,
    /// How long a heartbeat may be suppressed for saying nothing new before one is sent anyway.
    ///
    /// ⚠️ Without this the stream would go completely silent on an idle game, and `curl -N
    /// /api/events` ticking is the cheapest liveness check there is — an emulator thread that has
    /// died should show up as *absence*, not as a stream that was always quiet.
    pub status_keepalive: Duration,
    /// **W7** — where to checkpoint, and how often. `None` is a host that keeps nothing, which is
    /// what every test wants and what `gb serve` never is.
    pub run: Option<Arc<CurrentRun>>,
    pub checkpoint_interval: Duration,
    /// `POST /api/new-run`'s mailbox. `None` — the default, and every test — means the endpoint has
    /// nothing to talk to and the emulator never checks.
    pub new_runs: Option<Arc<NewRunRequests>>,
    /// Which Game Boy the cartridge runs on, from `GB_HARDWARE`. **[`Model::Dmg`] by default.**
    ///
    /// [`Model::Cgb`] puts Pokémon Red in compatibility mode, where the boot ROM's title-derived
    /// palette colours the same picture (see [`crate::boot_palette`]) — the red tint a real Game Boy
    /// Color gives it. The game plays identically: `full_playthrough` on a CGB ends on the same tile
    /// with the same party, the same money and the same bag as on a DMG, because at single speed
    /// nothing the cartridge can observe moves and the RNG stream it feeds does not re-roll.
    ///
    /// ⚠️ **It costs bandwidth, and the video format absorbs it without a change.** Six distinct
    /// colours rather than four widens [`crate::web::video`]'s `bits_per_pixel` from 2 to 4, which
    /// measured **1.63× on the wire** against the same footage — well under the 2× it costs before
    /// deflate, because the extra bits are mostly repeats of what the compressor is already
    /// matching.
    ///
    /// ⚠️ **A state is not tied to the machine that wrote it, and this is what makes that safe**:
    /// every fixture and every deployed `state.gbst` is a DMG capture carrying a DMG's all-white CGB
    /// palette RAM, and [`crate::mmu::MMU::read_sections`] re-installs the boot palette over it. Without
    /// that, switching this to `Cgb` renders every resumed run as a blank white screen.
    pub model: Model,
    /// Whether the state this host is starting from is a **new game** rather than a resumed one.
    ///
    /// The only thing it decides is whether the player is renamed from the policy
    /// ([`Policy::player_name`]). ⚠️ **A resume must not be**: the name is part of the save, so a
    /// process restarted under a different `GB_MODEL` would silently rename a trainer mid-run — and
    /// the game has already printed it in a dozen places by then.
    pub fresh_game: bool,
}

/// ⚠️ **The one channel from the HTTP layer back into the emulator thread**, and deliberately the
/// narrowest one that will do the job.
///
/// `src/web/mod.rs` was built so that it could reach [`Published`] and nothing else, which made
/// "the server cannot affect the run" a structural fact rather than a promise. `POST /api/new-run`
/// gives that up on purpose, so it gives up as little as possible: this carries **no data inwards**
/// — only the fact that someone asked — and it is answered by the emulator thread at a point of its
/// own choosing, between instructions, never by the request handler. There is nothing here to grow
/// into a general command channel without a deliberate edit.
///
/// The reply travels back on a `oneshot` so the handler can return the new run's id, which is the
/// only thing a caller actually needs.
#[derive(Default)]
pub struct NewRunRequests {
    pending: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Result<String, String>>>>,
}

impl NewRunRequests {
    /// Ask for a fresh run. The receiver resolves with the new run's id once the emulator has acted.
    ///
    /// Refuses while one is already outstanding rather than replacing it: two resets a few
    /// milliseconds apart would otherwise leave the first caller's channel dropped and
    /// indistinguishable from an emulator that had died.
    ///
    /// ⚠️ **"Outstanding" means a caller is still listening.** A request whose handler has given up
    /// waiting — the emulator thread died, so nothing will ever `take()` it — leaves a sender whose
    /// receiver is dropped, and treating *that* as outstanding would wedge the endpoint permanently
    /// on the one failure it most needs to stay usable through.
    pub fn request(&self) -> Result<tokio::sync::oneshot::Receiver<Result<String, String>>, String> {
        let mut pending = self.pending.lock().expect("new-run mailbox poisoned");
        if pending.as_ref().is_some_and(|sender| !sender.is_closed()) {
            return Err("a new run is already being started".to_string());
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        *pending = Some(sender);
        Ok(receiver)
    }

    fn take(&self) -> Option<tokio::sync::oneshot::Sender<Result<String, String>>> {
        self.pending.lock().expect("new-run mailbox poisoned").take()
    }
}

// ⚠️ **Nothing may stop this loop while the game is being played.** W4 briefly had a
// `GB_PAUSE_WHILE_THINKING` that froze `gb.run` while the model thought (§2.1 of
// `docs/llm-web-playthrough-plan.md`); it was removed the same day. A live picture is the whole
// point of the server, and freezing it is never the trade anyone wants — and because
// `Policy::service_tools` only runs when the emulator advances, any pause spanning an LLM tool call
// deadlocked the run outright. If a future change wants to pause here, that is the hazard to think
// about first.

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            target_speed: 1.0,
            video_interval: Duration::from_nanos(1_000_000_000 / 30),
            // 2 Hz. It was 10, which measured at **49.7 kbit/s per viewer** — six times the idle
            // video feed and 90% of `/api/events` — for a payload that was byte-identical to the one
            // before it nine times out of ten. Nothing on the page needs a fresher answer than this:
            // the picture is streamed at 30 fps and is where movement is actually watched.
            status_interval: Duration::from_millis(500),
            status_keepalive: Duration::from_secs(2),
            run: None,
            checkpoint_interval: Duration::from_secs(60),
            new_runs: None,
            // Every test loads a fixture of a game already in progress, so the default is the one
            // that leaves the trainer's name alone.
            fresh_game: false,
            // ⚠️ **The DMG is the default and the tests depend on it.** `web::video::bench` asserts
            // a four-shade screen, and every fixture in the suite was captured on one.
            model: Model::Dmg,
        }
    }
}

pub struct EmulatorHost {
    gb: GameBoy,
    agent: PokemonAgent,
    map_cache: MapMetadataCache,
    published: Arc<Published>,
    encoder: VideoEncoder,
    config: HostConfig,

    cycle_duration: Duration,
    last_iteration: Instant,
    since_last_update: Duration,
    /// Emulated time the catch-up clamp has thrown away on this run — the sum of every iteration's
    /// overrun past [`MAX_CATCHUP`].
    ///
    /// ⚠️ **Dropping it is deliberate** (see the constant) **and until this existed it was also
    /// invisible**, which is what made a healthy host look like a slow one. The panel used to report
    /// speed as `emulated_ms / wall_ms` over the life of the process, so a single busy startup — a
    /// container competing for a core while the server, the transcript thread and the LLM worker all
    /// come up — subtracted from that ratio for ever and read as "the emulator cannot keep up". It is
    /// the opposite: the time was dropped *once*, on purpose, and the run has been at 1× since. The
    /// page now derives the rate from consecutive heartbeats and shows this beside it, so "fell
    /// behind once" and "is behind now" are two different readings rather than one.
    dropped: Duration,
    /// The last tick on which the run was found parked, and the total wall clock it has spent that
    /// way. See [`Self::paused_for`].
    ///
    /// ⚠️ **The total is subtracted from the `wall_ms` this run reports**, so a run parked overnight
    /// on a spent quota does not report the wait as time it spent playing. The cartridge's own clock
    /// needs no such help: it is emulated, so stopping the emulator stops it, which is the whole
    /// reason the pause exists rather than merely holding back the requests.
    paused_since: Option<Instant>,
    paused_total: Duration,
    /// Cycles `gb.run` has already delivered beyond what was asked for, spent down before any more
    /// are requested. Without it the emulator drifts steadily fast.
    ahead_by_cycles: MachineCycles,
    emulated: MachineCycles,
    next_video: Instant,
    next_status: Instant,
    next_checkpoint: Instant,
    /// The last heartbeat *sent*, and when. Together they are the whole of the send-on-change rule.
    last_status: Option<StatusSnapshot>,
    last_status_at: Instant,
    /// Whether the first cycle has been emulated — see the `RunStatus::Playing` transition in
    /// [`Self::tick`].
    booted: bool,

    // ── What this process has contributed to the run ─────────────────────────────────────────────
    /// When the **current run** started being played here. ⚠️ Not `started`, which is the process's
    /// uptime and is what `/api/healthz` and `StatusSnapshot::wall_ms` mean; this one is reset by
    /// [`Self::start_new_run`] because it measures *this game*.
    run_started: Instant,
    /// [`Published::turns`] as of the moment this run became current. Turns are counted per process,
    /// so the run's share is the difference.
    turns_at_run_start: u64,
    /// [`AgentEvent::WatchdogFired`]s seen this run. Counted here rather than on the agent because
    /// this is where events are already being walked.
    watchdog_firings: u64,
    /// The completion event and the sequence number it was published with, parked for the top of the
    /// next tick — see [`Self::file_completed_run`] for why it is not acted on where it is found.
    completed: Option<(AgentEvent, u64)>,
    /// The last `agent.update` failure that was published.
    ///
    /// ⚠️ **Publishing every failure is a flood, and the game reaches a state that produces one on
    /// every tick.** After the credits pokered does `jp Init`: WRAM is cleared, `game_mode()` is
    /// `None`, and `agent.update` answers `Err("Not in game")` fifty times a second — into the
    /// transcript, into every open browser, for as long as the process lives. Publishing only on
    /// change keeps the report and drops the flood.
    last_agent_failure: Option<String>,
}

impl EmulatorHost {
    /// Build a host running `policy` from `save_state`.
    ///
    /// The state is a **DMG** save — every committed fixture is, including the one `gb serve` starts
    /// from — but the machine it is restored into is [`HostConfig::model`]'s, which `GB_HARDWARE`
    /// chooses. A DMG snapshot in a Game Boy Color is a supported combination now rather than an
    /// untried one: `MMU::read_sections` re-installs the compatibility palette the state cannot
    /// carry, and `full_playthrough` passes on both.
    pub fn new(
        save_state: &[u8],
        policy: Box<dyn Policy>,
        published: Arc<Published>,
        config: HostConfig,
    ) -> Result<Self, String> {
        let mut gb = GameBoy::new(crate::pokemon::roms::POKERED, config.model);
        gb.load_state(save_state).map_err(|e| format!("could not load the starting state: {e}"))?;
        // ⚠️ **§12's note, left here because this is where it will bite.** `Audio::set_output_sample_rate`
        // is not serialised, so anything that loads a state has to re-apply it — see the `F9` handler
        // in `render.rs`. Nothing here consumes audio while §12 is deferred; the moment a stream is
        // added, this line is where the rate goes back on.

        let now = Instant::now();
        let cycle_duration = REALTIME_CYCLE_DURATION.div_f64(config.target_speed.max(f64::MIN_POSITIVE));
        let first_checkpoint = now + config.checkpoint_interval;
        // Zero in every caller today, since `Published` is built beside the host — read rather than
        // assumed so that a future one that shares a buffer does not silently credit this run with
        // someone else's turns.
        let turns_at_run_start = published.turns();
        let mut host = Self {
            gb,
            agent: PokemonAgent::new(policy),
            map_cache: MapMetadataCache::default(),
            published,
            encoder: VideoEncoder::default(),
            config,
            cycle_duration,
            last_iteration: now,
            since_last_update: Duration::ZERO,
            dropped: Duration::ZERO,
            paused_since: None,
            paused_total: Duration::ZERO,
            ahead_by_cycles: MachineCycles::ZERO,
            emulated: MachineCycles::ZERO,
            next_video: now,
            next_status: now,
            // Deliberately one interval out rather than `now`: the state that has just been loaded
            // is the state that was just checkpointed, and rewriting it at startup would mean a
            // process that crash-loops rewrites its own save every few seconds.
            next_checkpoint: first_checkpoint,
            last_status: None,
            last_status_at: now,
            booted: false,
            run_started: now,
            turns_at_run_start,
            watchdog_firings: 0,
            completed: None,
            last_agent_failure: None,
        };
        if host.config.fresh_game {
            host.name_the_player();
        }
        Ok(host)
    }

    /// Put the policy's name on the trainer card, if it has one.
    ///
    /// ⚠️ **A RAM write, because there is no screen left to type it on.** A run starts from
    /// `data::START_OF_GAME` — a save state captured in Red's bedroom, past the title screen, past
    /// Oak's speech and past both name screens — so the name entry the game itself offers happened
    /// once, when the fixture was made, and cannot happen again. (It is not merely inconvenient to
    /// get back to: `game_mode` answers `None` for the whole intro, so `agent.update` returns
    /// `Err("Not in game")` and no policy is polled at any point during it.)
    ///
    /// ⚠️ **Only on a new game.** See `HostConfig::fresh_game`.
    ///
    /// A failure here is reported and stepped over. The name is on the trainer card and in the
    /// game's own prose; it is not worth refusing to play over.
    fn name_the_player(&mut self) {
        let Some(name) = self.agent.policy_player_name() else { return };
        let written = {
            let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
            api.write_player_name(&name)
        };
        match written {
            Ok(()) => println!("gb — the player is called {name}"),
            Err(error) => {
                self.published.publish_event(UiEventBody::Notice {
                    level: "warn",
                    message: format!("could not name the player {name}: {error}"),
                });
            }
        }
    }

    /// Build a host on a new thread and run it there until `shutdown` is set.
    ///
    /// The policy arrives as a **factory** rather than a `Box<dyn Policy>` because `Policy` is not
    /// declared `Send` — `ConsolePolicy` and, from W4, `LlmPolicy` own channel endpoints, and adding
    /// the bound to the trait would constrain every implementation for the benefit of this one call
    /// site. Building the policy on the thread that will own it sidesteps the question entirely.
    ///
    /// Construction still reports on *this* thread: a starting state that will not load is a clean
    /// error before anything is listening, not a thread that dies quietly behind a live server.
    pub fn spawn(
        save_state: Vec<u8>,
        policy: Box<dyn FnOnce() -> Box<dyn Policy> + Send>,
        published: Arc<Published>,
        config: HostConfig,
        shutdown: Arc<AtomicBool>,
    ) -> Result<std::thread::JoinHandle<()>, String> {
        let (ready, started) = std::sync::mpsc::channel();
        let obituary = Obituary(Arc::clone(&published));
        let handle = std::thread::Builder::new()
            .name("emulator".to_string())
            .stack_size(EMULATOR_STACK)
            .spawn(move || {
                let _obituary = obituary;
                let mut host = match Self::new(&save_state, policy(), published, config) {
                    Ok(host) => {
                        let _ = ready.send(Ok(()));
                        host
                    }
                    Err(failure) => {
                        let _ = ready.send(Err(failure));
                        return;
                    }
                };
                host.run(&shutdown);
            })
            .map_err(|e| format!("could not start the emulator thread: {e}"))?;

        match started.recv() {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(failure)) => {
                let _ = handle.join();
                Err(failure)
            }
            Err(_) => Err("the emulator thread stopped before it started".to_string()),
        }
    }

    /// Run until `shutdown` is set. Blocking; this is the thread's whole life.
    pub fn run(&mut self, shutdown: &AtomicBool) {
        while !shutdown.load(Ordering::Relaxed) {
            if !self.tick() {
                std::thread::sleep(IDLE_SLEEP);
            }
        }
        // **W7 / §11.** The clean-shutdown checkpoint, and the reason `Ctrl-C` and `SIGTERM` both
        // stop the server rather than killing the process: everything since the last periodic
        // checkpoint — up to a minute of play — is only here.
        self.checkpoint();
    }

    /// Write the run's state to disk, if this host has somewhere to write it.
    ///
    /// Failures are published rather than propagated. A disk that cannot be written is worth
    /// shouting about; it is not worth stopping a run that is otherwise playing perfectly well, and
    /// the next attempt is sixty seconds away.
    fn checkpoint(&mut self) {
        let Some(run) = self.config.run.as_ref().map(|current| current.get()) else { return };
        let state = match self.gb.save_state() {
            Ok(state) => state,
            Err(failure) => {
                self.published.publish_event(UiEventBody::Notice {
                    level: "error",
                    message: format!("could not save the state: {failure}"),
                });
                return;
            }
        };
        if let Err(failure) = run.checkpoint(&state, &self.gb.dump_sram(), self.progress()) {
            self.published.publish_event(UiEventBody::Notice {
                level: "error",
                message: format!("could not checkpoint the run: {failure}"),
            });
        }
    }

    /// What **this process** has contributed to the current run since it opened it.
    ///
    /// ⚠️ Cumulative since the run became current, not since the last checkpoint:
    /// [`RunDir::checkpoint`] rebases it onto the baseline it read at open, so a delta here would be
    /// counted once per checkpoint. [`crate::run::RunProgress`] has the argument in full.
    fn progress(&self) -> RunProgress {
        let usage = self.published.usage();
        RunProgress {
            emulated_ms: self.emulated.to_duration().as_millis() as u64,
            // ⚠️ **Minus the park, the same as the heartbeat's.** The field this reads says it is
            // subtracted from "the `wall_ms` this run reports", and it was true on that path and not
            // on this one — so a run parked overnight on a spent quota wrote the wait into
            // `meta.json` and into its Hall of Fame row as time it had spent playing. Both operands
            // are per-run-per-process (`start_new_run` resets them together) and `RunDir::checkpoint`
            // adds the persisted baseline on top, so the rebasing is unaffected.
            wall_ms: self.run_started.elapsed().saturating_sub(self.paused_total).as_millis() as u64,
            prompt_tokens: usage.map_or(0, |u| u.prompt_tokens),
            completion_tokens: usage.map_or(0, |u| u.completion_tokens),
            completions: usage.map_or(0, |u| u.completions),
            turns: self.published.turns().saturating_sub(self.turns_at_run_start),
            watchdog_firings: self.watchdog_firings,
        }
    }

    /// File a finished run in the Hall of Fame and begin the next one.
    ///
    /// Called at the top of [`Self::tick`], beside the new-run mailbox, for the same reason: it is
    /// the one point in the process where nothing is half-done. The order is the design:
    ///
    /// 1. **Checkpoint**, so the save state at the moment of victory is on disk in the live run
    ///    directory as well as in the archive.
    /// 2. **Archive, blocking, before the swap.** ⚠️ `transcript.rs` re-reads
    ///    `CurrentRun::get().transcript_path()` *per event*, so once a new run is current, an event
    ///    published before the swap but written after it lands in the new run's transcript. Doing
    ///    the follow first is what makes "which file is the victory in?" a question with an answer.
    /// 3. **Stamp `meta.json`**, so a process restarted from a checkpoint taken a moment before the
    ///    counter moved does not replay those seconds and file the same win twice. The agent's edge
    ///    trigger cannot see across a process boundary; this can.
    /// 4. **Start the next run**, which checkpoints again (harmless) and leaves the finished
    ///    directory exactly where it is.
    ///
    /// ⚠️ **A failed archive must not restart the run.** A run that won and could not be recorded is
    /// better left playing — the next `/reset-game` is a human decision, and the directory is still
    /// intact and resumable.
    ///
    /// ⚠️ **And it is not retried, which loses the trophy.** The edge is a rising edge: the counter
    /// has already moved, so nothing will fire again, and a process restarted from the checkpoint
    /// taken above seeds its baseline from a save that has already won and stays silent. A retry
    /// loop is not the answer — this runs fifty times a second, and a disk that is full now is full
    /// in 20 ms — so the failure notice names the directory instead, because filing it by hand is
    /// then a `cp`. If this ever happens in practice, a delayed bounded retry is the fix, not a busy
    /// one.
    fn file_completed_run(&mut self, event: &AgentEvent, seq: u64) {
        let AgentEvent::HallOfFame { teams, playtime, playtime_seconds, .. } = event else { return };
        let Some(current) = self.config.run.clone() else { return };
        let run = current.get();
        if run.already_archived(*teams) {
            return;
        }

        self.checkpoint();
        let state = match self.gb.save_state() {
            Ok(state) => state,
            Err(failure) => return self.complain(format!("could not save the winning state: {failure}")),
        };

        // Read before the archive, because `game_state()` needs the emulator and the archive does
        // not. A failure here costs the row its final-team detail, not the archive.
        let (badges, pokedex_owned, pokedex_seen, money, party, playtime_maxed) = self.final_state();
        // ⚠️ After the checkpoint above, which is what folds this process's figures onto the run's
        // baseline. Reading it first would file the run with the totals it had a minute ago.
        let meta = run.meta();
        let usage = self.published.usage();
        let job = crate::run::hall_of_fame::ArchiveJob {
            root: current.root().to_path_buf(),
            run_dir: run.path().to_path_buf(),
            state,
            sram: self.gb.dump_sram(),
            until_seq: seq,
            completion: crate::run::hall_of_fame::Completion {
                archive: String::new(), // filled in by `archive`, which chooses the directory
                run_id: meta.run_id.clone(),
                teams: *teams,
                completed_at: crate::run::iso8601(std::time::SystemTime::now()),
                started_at: meta.started_at.clone(),
                app_version: crate::cli::VERSION.to_string(),
                policy: self.agent.policy_name().to_string(),
                // `RunMeta::model` is the literal `"random"` under `--policy random`; a leaderboard
                // column that says "random" as if it were a model name would be a small lie.
                model: (meta.model != "random").then(|| meta.model.clone()),
                playtime_seconds: *playtime_seconds,
                playtime: playtime.clone(),
                playtime_maxed,
                emulated_ms: meta.emulated_ms,
                wall_ms: meta.wall_ms,
                turns: meta.turns,
                completions: meta.completions,
                prompt_tokens: meta.prompt_tokens,
                completion_tokens: meta.completion_tokens,
                tokens_estimated: usage.is_some_and(|u| u.estimated),
                watchdog_firings: meta.watchdog_firings,
                resumes: meta.resumed_from.len(),
                checkpoints: meta.checkpoints,
                badges,
                pokedex_owned,
                pokedex_seen,
                money,
                party,
            },
            meta,
        };

        let archived = match crate::run::hall_of_fame::archive(&job) {
            Ok(name) => name,
            // Names the directory, because this is not retried and filing it by hand is then a `cp`.
            Err(failure) => {
                return self.complain(format!(
                    "could not file the finished run: {failure}. {} won the game and is complete on \
                     disk, but is not in the leaderboard",
                    job.run_dir.display(),
                ));
            }
        };
        if let Err(failure) = run.record_completion(crate::run::hall_of_fame::recorded(*teams, archived.clone())) {
            // The archive is written and the ledger row is appended; only the idempotence stamp
            // failed. Say so and carry on — a duplicate row is a much smaller problem than refusing
            // to start the next run.
            self.complain(format!("could not stamp the finished run's meta.json: {failure}"));
        }

        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: format!(
                "🏆 {} finished the game in {playtime}, filed as {}/{archived}",
                run.run_id(),
                crate::run::files::HALL_OF_FAME,
            ),
        });
        println!("gb serve — {} finished the game, filed as {archived}", run.run_id());

        match self.start_new_run() {
            Ok(run_id) => println!("gb serve — playing again as {run_id}"),
            Err(failure) => self.complain(format!("could not start the next run: {failure}")),
        }
    }

    /// The winning run's final tally, for the ledger row. Zeroes if the game is momentarily
    /// unreadable — which it may well be, this being the first frame of a cutscene.
    fn final_state(&mut self) -> (u32, usize, usize, u32, Vec<crate::run::hall_of_fame::PartyMember>, bool) {
        use crate::pokemon::symbols::{DmgPointerRead, pokered_symbols};
        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        let maxed = api.mmu().read_pointer(&pokered_symbols::wPlayTimeMaxed) != 0;
        let Ok(state) = api.game_state() else { return (0, 0, 0, 0, Vec::new(), maxed) };
        let party = state
            .pokemon
            .iter()
            .map(|mon| crate::run::hall_of_fame::PartyMember {
                nickname: mon.nickname.to_default_string(),
                species: format!("{:?}", mon.species),
                level: mon.level,
            })
            .collect();
        (
            state.badges.bits().count_ones(),
            state.pokedex_owned.species().len(),
            state.pokedex_seen.species().len(),
            state.money,
            party,
            maxed,
        )
    }

    /// Publish an error notice and print it. Every failure in the completion path takes this route:
    /// none of them is worth stopping a run that is otherwise playing perfectly well.
    fn complain(&self, message: String) {
        eprintln!("gb serve — {message}");
        self.published.publish_event(UiEventBody::Notice { level: "error", message });
    }

    /// Abandon the current run and start the game again in a fresh run directory, without stopping.
    ///
    /// Runs on the **emulator thread** (see the call site in [`Self::tick`]), which is what makes it
    /// safe to touch `gb`, the agent and the encoder at all. The order below is the whole design:
    ///
    /// 1. **Checkpoint the outgoing run first.** It is about to stop being current, and everything
    ///    since its last periodic checkpoint lives only in memory — the directory left behind has to
    ///    be a complete, resumable run, not one truncated up to a minute short.
    /// 2. **Open the new directory before touching the emulator.** A full disk should fail with the
    ///    old run still playing, not halfway into a reset with nothing to go back to.
    /// 3. Load the start-of-game state, reset the agent, and tell the policy — [`Policy::restart`]
    ///    is where `LlmPolicy` throws away a conversation that is now about a game that no longer
    ///    exists, and repoints the model's notes at the new directory.
    /// 4. **Restart the video encoder** (its ⚠️ explains why `default()` will not do) and clear
    ///    `last_status`, or the send-on-change rule would suppress the heartbeat that says the run
    ///    changed — the one heartbeat a viewer most needs.
    ///
    /// `emulated` restarts at zero because it measures *this game*, and it is what `meta.json`'s
    /// `emulated_ms` reports. ⚠️ **Everything paired with it has to restart with it** — `run_started`,
    /// `paused_total` and `dropped` all do, a few lines below. The host used to keep a second clock
    /// stamped once at construction and report the heartbeat's `wall_ms` from that, which paired a
    /// counter that resets with one that does not: a run started here read as a fraction of realtime
    /// for as long as the process had been up before it. (The comment that justified that clock
    /// claimed it was what `/api/healthz` reports as uptime. It is not — that is `state.started` on
    /// the HTTP state in `web::serve_http`, which nothing here touches.)
    fn start_new_run(&mut self) -> Result<String, String> {
        let Some(current) = self.config.run.clone() else {
            return Err("this host has no run directory, so there is no new run to start".to_string());
        };
        self.checkpoint();
        let previous = current.get().run_id();
        let run = current.start_new()?;
        let run_id = run.run_id();

        self.gb = GameBoy::new(crate::pokemon::roms::POKERED, self.config.model);
        self.gb
            .load_state(crate::pokemon::data::START_OF_GAME)
            .map_err(|e| format!("could not load the start-of-game state: {e}"))?;
        self.map_cache = MapMetadataCache::default();
        self.agent.restart(Some(run.path()));
        // A new run is a new game, so the trainer is named again — the policy may well have changed
        // under a process that has been up for days.
        self.name_the_player();

        self.encoder.restart();
        self.last_status = None;
        self.emulated = MachineCycles::ZERO;
        // Everything that measures *this game* rather than this process starts again with it. The
        // agent's own Hall of Fame baseline is reset by `agent.restart` above.
        self.run_started = Instant::now();
        self.turns_at_run_start = self.published.turns();
        self.published.forget_usage();
        self.watchdog_firings = 0;
        self.completed = None;
        self.last_agent_failure = None;
        self.ahead_by_cycles = MachineCycles::ZERO;
        self.since_last_update = Duration::ZERO;
        self.dropped = Duration::ZERO;
        self.last_iteration = Instant::now();
        // ⚠️ Released here as well as by the worker on its way out of the park. Starting a run is
        // how a park is escaped when something has gone wrong with it, so this seam is the one place
        // that must not depend on the parked thread doing anything.
        self.published.set_throttled_until(0);
        self.paused_since = None;
        self.paused_total = Duration::ZERO;
        self.next_checkpoint = Instant::now() + self.config.checkpoint_interval;
        self.published.set_status(RunStatus::Playing);
        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: format!("new run {run_id}. {previous} was checkpointed and left where it is"),
        });
        println!("gb serve — new run {run_id} (was {previous})");
        Ok(run_id)
    }

    /// One iteration of the loop. Returns whether any emulation happened, which is the loop's cue
    /// that it is behind and should come straight back rather than sleep.
    ///
    /// Separate from [`Self::run`] so a test can drive the host deterministically instead of racing
    /// a thread.
    pub fn tick(&mut self) -> bool {
        // **The reset seam.** Answered here, at the top of a tick, because that is the one place in
        // the process where nothing is half-done: the previous tick's instruction has retired, the
        // agent is between decisions and no borrow of `gb` is outstanding. A `Mutex::try_lock`-free
        // `take()` on an empty mailbox is a lock and a `None`, which is why this can sit on the hot
        // path unconditionally.
        if let Some(sender) = self.config.new_runs.as_ref().and_then(|mailbox| mailbox.take()) {
            let _ = sender.send(self.start_new_run());
        }
        // **The end of the game**, answered here for the reason above and not where the event is
        // found: filing a run swaps the run directory out from under the transcript thread, and the
        // middle of a tick is not where that should happen. See `file_completed_run`.
        if let Some((event, seq)) = self.completed.take() {
            self.file_completed_run(&event, seq);
        }

        let now = Instant::now();

        // **The pause seam.** The LLM worker parks the run when the endpoint's quota is spent and it
        // said when it reopens (`Worker::park_until`), and the game stops with it: a model that is
        // not allowed to think should not have the world move on without it, and the cartridge's own
        // clock is what the leaderboard ranks on.
        //
        // ⚠️ **Below the two seams above and above everything else.** A parked run must still answer
        // `POST /api/new-run` — that is how a wedged park is escaped — and the reset is handled
        // before this line for exactly that reason.
        //
        // ⚠️ **It skips the emulator and the agent only.** The heartbeat, the video and the
        // checkpoint at the bottom of this function all still run, or the page would see the stream
        // simply stop: silence and a dead connection are indistinguishable to it (`STALE_MS`), so a
        // paused run that published nothing would show as "reconnecting" rather than as paused.
        let paused = self.paused_for(now);
        if paused {
            // ⚠️ No catch-up debt. `since_last_update` is what makes the emulator make up for a
            // slow iteration, so leaving it to accumulate across a park of hours would have the
            // game fast-forward through `MAX_CATCHUP` the moment the quota reopened.
            self.last_iteration = now;
            self.since_last_update = Duration::ZERO;
        } else {
            let gap = now.saturating_duration_since(self.last_iteration);
            // What the clamp throws away, kept rather than merely discarded. See the field.
            self.dropped += gap.saturating_sub(MAX_CATCHUP);
            self.since_last_update += gap.min(MAX_CATCHUP);
            self.last_iteration = now;
        }

        let mut min_cycles = MachineCycles::ZERO;
        while self.since_last_update >= self.cycle_duration {
            self.since_last_update -= self.cycle_duration;
            if self.ahead_by_cycles > MachineCycles::ZERO {
                self.ahead_by_cycles -= MachineCycles::ONE;
            } else {
                min_cycles += MachineCycles::ONE;
            }
        }

        let mut ran = MachineCycles::ZERO;
        if min_cycles > MachineCycles::ZERO {
            // **W6** — the one status transition the emulator owns, and it happens **once**.
            // Everything after it belongs to the policy: under `--policy llm` the worker moves the
            // status to `AwaitingLlm` and back, and setting `Playing` from here every tick would
            // stamp on it fifty times a second. Under any other policy `Playing` is the whole state
            // machine, which is why there is nothing else here.
            if !self.booted {
                self.booted = true;
                self.published.set_status(RunStatus::Playing);
            }
            ran = self.gb.run(min_cycles);
            self.emulated += ran;
            self.ahead_by_cycles += ran - min_cycles;

            let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
            // ⚠️ **On change, not on every failure.** `agent.update` answers `Err("Not in game")`
            // for as long as the game is unreadable, which after the Hall of Fame credits — pokered
            // ends them with `jp Init`, clearing WRAM — is *for ever*. Publishing each one would put
            // fifty notices a second into the transcript and every open browser. See
            // `last_agent_failure`.
            match self.agent.update(&mut api, ran) {
                Ok(()) => self.last_agent_failure = None,
                Err(failure) => {
                    if self.last_agent_failure.as_deref() != Some(failure.as_str()) {
                        self.last_agent_failure = Some(failure.clone());
                        self.published.publish_event(UiEventBody::Notice {
                            level: "error",
                            message: format!("agent tick failed: {failure}"),
                        });
                    }
                }
            }
            let events = self.agent.drain_events();
            for event in events {
                let seq = self.published.publish_event(UiEventBody::Agent {
                    kind: event_kind(&event),
                    text: format!("{event}"),
                });
                match event {
                    AgentEvent::WatchdogFired { .. } => self.watchdog_firings += 1,
                    // Parked rather than acted on: the run directory is about to be swapped, and
                    // the top of the next tick is the place for that. The seq travels with it —
                    // `hall_of_fame::archive` follows the transcript up to this line.
                    event @ AgentEvent::HallOfFame { .. } => {
                        self.completed.get_or_insert((event, seq));
                    }
                    _ => {}
                }
            }
        }

        if now >= self.next_video {
            self.next_video = schedule_next(self.next_video, now, self.config.video_interval);
            self.publish_video();
        }
        if now >= self.next_status {
            self.next_status = schedule_next(self.next_status, now, self.config.status_interval);
            self.publish_status(now);
        }
        if now >= self.next_checkpoint {
            self.next_checkpoint = schedule_next(self.next_checkpoint, now, self.config.checkpoint_interval);
            self.checkpoint();
        }

        ran > MachineCycles::ZERO
    }

    /// Whether the run is parked on a spent quota, and the wall-clock bookkeeping that goes with it.
    ///
    /// ⚠️ **The deadline is re-read rather than trusted to be cleared.** The worker releases the cell
    /// itself on its way out of the park, but a worker that died holding it would otherwise stop the
    /// emulator for the rest of the process — so a deadline that has passed reads as running here,
    /// whatever the cell still says.
    fn paused_for(&mut self, now: Instant) -> bool {
        let parked = self.published.throttled_until().is_some_and(|until| now_ms() < until);
        match (parked, self.paused_since) {
            (true, Some(previous)) => {
                self.paused_total += now.saturating_duration_since(previous);
                self.paused_since = Some(now);
            }
            (true, None) => self.paused_since = Some(now),
            (false, _) => self.paused_since = None,
        }
        parked
    }

    fn publish_video(&mut self) {
        // Copied out before encoding: the encoder borrows `self` mutably and the LCD borrows the
        // `GameBoy` inside it. The copy is not waste — the snapshot below needs one anyway, and it
        // is what W5's `screenshot` reads without an emulator round trip.
        let frame: Box<Frame> = Box::new(*self.gb.core().mmu().ppu().lcd());
        let Some(delta) = self.encoder.encode(&frame) else {
            return; // nothing moved on screen, so nothing goes on the wire
        };
        let keyframe = self.encoder.keyframe().expect("something was just encoded");
        self.published.publish_frame(FrameSnapshot { seq: delta.seq, pixels: frame });
        self.published.publish_video(keyframe, delta);
    }

    /// Sample the game state and publish it — **if it says anything the last one did not**, or if
    /// the keepalive is due.
    ///
    /// The read happens either way: knowing whether anything changed means looking. What is saved is
    /// the bytes on every viewer's connection, which is where the cost actually was.
    fn publish_status(&mut self, now: Instant) {
        // `game_state` reads a lot of RAM and can legitimately fail mid-transition. A heartbeat that
        // says "no game state" is far easier to diagnose than one that stops arriving.
        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        let game = api.game_state().ok().map(|state| observe::status(&state, &api));
        let snapshot = StatusSnapshot {
            // ⚠️ **`run_started`, not the process's own clock.** `emulated` beside it is zeroed by
            // `start_new_run`, so pairing it with a clock that is not would report the new run's
            // first seconds against the whole process's age — which is what made the panel read
            // near-0× for hours after a `/reset-game` or a Hall of Fame swap.
            //
            // Minus whatever the run has spent parked on a spent quota: that is not time it was
            // playing, and it is the figure the ledger reports beside the cartridge's own clock.
            wall_ms: now
                .duration_since(self.run_started)
                .saturating_sub(self.paused_total)
                .as_millis() as u64,
            emulated_ms: self.emulated.to_duration().as_millis() as u64,
            dropped_ms: self.dropped.as_millis() as u64,
            target_speed: self.config.target_speed,
            // Asked of the decider itself rather than configured alongside it: two places naming the
            // same thing is two places to disagree, and this one is a wire contract the page renders.
            policy: self.agent.policy_name(),
            // Who is playing, for the page's title and header. Cloned per *published* heartbeat
            // rather than per sample, and the heartbeat is send-on-change, so this is a handful of
            // short strings a minute. `None` under `--policy random` — see the field.
            model: self
                .config
                .run
                .as_ref()
                .map(|run| run.model())
                .filter(|model| *model != "random")
                .map(str::to_string),
            agent_state: self.agent.state_debug(),
            frame_seq: self.encoder.seq(),
            game,
            run: self.published.run_status(),
        };

        let unchanged = self.last_status.as_ref().is_some_and(|last| snapshot.says_the_same_as(last));
        if unchanged && now.duration_since(self.last_status_at) < self.config.status_keepalive {
            return;
        }
        self.last_status = Some(snapshot.clone());
        self.last_status_at = now;
        self.published.publish_status(snapshot);
    }
}

/// Says so if the emulator thread dies.
///
/// A panic there is otherwise **invisible**: the HTTP layer only reads published buffers, so it keeps
/// serving perfectly well — the last frame, forever, with no error anywhere. `Drop` runs during
/// unwinding, which is what makes this work where code after `host.run()` would not.
struct Obituary(Arc<Published>);

impl Drop for Obituary {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let message = "the emulator thread panicked; the stream is frozen from here";
            eprintln!("{message}");
            self.0.publish_event(UiEventBody::Notice { level: "error", message: message.to_string() });
        }
    }
}

/// Advance a periodic deadline. Normally one interval on; if the loop fell far enough behind that
/// the deadline is already history, it re-bases on `now` rather than firing repeatedly to catch up —
/// nobody wants a burst of eight identical status frames after a stall.
fn schedule_next(deadline: Instant, now: Instant, interval: Duration) -> Instant {
    let next = deadline + interval;
    if next > now { next } else { now + interval }
}

fn event_kind(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::StartedOverworldAction { .. } => "started_overworld_action",
        AgentEvent::OverworldActionAborted { .. } => "overworld_action_aborted",
        AgentEvent::OverworldActionCompleted { .. } => "overworld_action_completed",
        // ⚠️ **The page keys off this to *not* draw a row** (`useEventStream`'s `fold`), so it is
        // load-bearing rather than decorative: folded back into the line above, every conversation
        // in the run would reappear in the log as a "✓" the dialogue underneath already said.
        AgentEvent::OverworldInteractionCompleted { .. } => "overworld_interaction_completed",
        AgentEvent::BattleStarted => "battle_started",
        AgentEvent::BattleActionStarted { .. } => "battle_action_started",
        AgentEvent::BattleEnded => "battle_ended",
        AgentEvent::TextBox { .. } => "text_box",
        // **W9.** Styled loudly by the page, and the one agent event that is a bug report rather
        // than a narration — see `AgentEvent::WatchdogFired`.
        AgentEvent::WatchdogFired { .. } => "watchdog",
        // ⚠️ **Must not join the kinds `useEventStream`'s `fold` drops.** This is the one line the
        // whole log exists to arrive at, and it is also what [`Self::tick`] keys off to archive the
        // run — so the string is read in two places, not one.
        AgentEvent::HallOfFame { .. } => "hall_of_fame",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::policy::RandomPolicy;
    use crate::web::published::{UiEvent, UiEventBody};
    use crate::web::video::VideoDecoder;

    fn host(published: Arc<Published>) -> EmulatorHost {
        host_with(published, |_| {})
    }

    fn host_with(published: Arc<Published>, tweak: impl FnOnce(&mut HostConfig)) -> EmulatorHost {
        host_from(crate::pokemon::data::START_OF_GAME, published, tweak)
    }

    fn host_from(
        state: &[u8],
        published: Arc<Published>,
        tweak: impl FnOnce(&mut HostConfig),
    ) -> EmulatorHost {
        let mut config = HostConfig {
            // Fast enough that a fraction of a second of wall clock is seconds of game time, so the
            // test is not at the mercy of how quickly the scheduler comes back to it.
            target_speed: 40.0,
            video_interval: Duration::from_millis(5),
            status_interval: Duration::from_millis(5),
            ..HostConfig::default()
        };
        tweak(&mut config);
        EmulatorHost::new(state, Box::new(RandomPolicy::default()), published, config)
            .expect("the committed fixture should load")
    }

    /// The W1 acceptance criterion, without the `curl`: status heartbeats arrive, they carry a game
    /// state, and the game state moves.
    #[test]
    fn the_host_publishes_a_moving_game_state() {
        let published = Published::new();
        let mut events = published.subscribe_events();
        let mut host = host(Arc::clone(&published));

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut statuses: Vec<StatusSnapshot> = Vec::new();
        while statuses.len() < 40 && Instant::now() < deadline {
            host.tick();
            while let Ok(UiEvent { body, .. }) = events.try_recv() {
                if let UiEventBody::Status(status) = body {
                    statuses.push(*status);
                }
            }
            std::thread::sleep(Duration::from_micros(500));
        }

        assert!(statuses.len() >= 40, "only {} status heartbeats arrived", statuses.len());
        assert!(statuses.iter().all(|s| s.game.is_some()), "a heartbeat could not read the game state");
        assert!(statuses.last().unwrap().emulated_ms > 0, "no emulated time was published");

        // The agent under `RandomPolicy` walks Red around his bedroom, so *something* has to move.
        // Position rather than map: leaving `RedsHouse2F` needs a warp the random walk may not find.
        let positions: std::collections::HashSet<_> =
            statuses.iter().filter_map(|s| s.game.as_ref()).map(|g| (g.position.x, g.position.y)).collect();
        assert!(positions.len() > 1, "the player never moved: {positions:?}");
    }

    /// **The pause seam.** A run parked on a spent quota stops the emulator, and — the half that is
    /// easy to get wrong — keeps talking to the page while it does.
    ///
    /// ⚠️ A paused run that published nothing would look *identical to a dead connection* from the
    /// browser: both are silence, which is the whole reason `STALE_MS` exists. So the assertion that
    /// heartbeats keep arriving is not padding, it is the bug this seam invites.
    #[test]
    fn a_parked_run_stops_the_game_but_keeps_the_page_fed() {
        let published = Published::new();
        let mut events = published.subscribe_events();
        // ⚠️ **The keepalive is the only thing that speaks while a run is parked, and that is the
        // point.** Send-on-change is silent by construction here: the screen is frozen, so every
        // sampled heartbeat says exactly what the last one did. Shortened rather than waited out, so
        // the test spends 200 ms rather than the 2 s the default would need.
        let mut host = host_with(Arc::clone(&published), |config| {
            config.status_keepalive = Duration::from_millis(20);
        });

        // Run normally for a moment, so there is emulated time to compare against.
        let warmup = Instant::now() + Duration::from_millis(200);
        while Instant::now() < warmup {
            host.tick();
        }
        let moving = host.emulated;
        assert!(moving > MachineCycles::ZERO, "the emulator never started");

        published.set_throttled_until(now_ms() + 30_000);
        while let Ok(_) = events.try_recv() {}

        let parked = Instant::now() + Duration::from_millis(200);
        let mut heartbeats = 0;
        while Instant::now() < parked {
            host.tick();
            while let Ok(UiEvent { body, .. }) = events.try_recv() {
                if matches!(body, UiEventBody::Status(_)) {
                    heartbeats += 1;
                }
            }
            std::thread::sleep(Duration::from_micros(500));
        }

        assert_eq!(host.emulated, moving, "the emulator ran while the run was parked");
        assert!(heartbeats > 0, "a parked run went silent, which a browser cannot tell from a dead one");

        // ⚠️ And it releases on the deadline alone. The worker clears the cell on its way out, but a
        // worker that died holding it must not stop the game for the rest of the process.
        published.set_throttled_until(now_ms() - 1);
        let resumed = Instant::now() + Duration::from_millis(200);
        while Instant::now() < resumed {
            host.tick();
        }
        assert!(host.emulated > moving, "the game did not resume once the deadline had passed");

        // ⚠️ And it resumes *where it was*: no catch-up debt is owed for the park, or the game would
        // fast-forward through `MAX_CATCHUP` the moment the quota reopened.
        assert!(
            host.since_last_update < host.cycle_duration * 2,
            "the park left catch-up debt: {:?}",
            host.since_last_update,
        );

        // ⚠️ **And the run does not report the wait as time it spent playing.** This is what the run
        // writes to `meta.json` and to its Hall of Fame row, and it used to be the one path that did
        // *not* subtract the park — the field's own doc claimed it did. A run parked overnight on a
        // spent quota logged the whole night as play. Both paths now agree.
        let progress = host.progress();
        assert!(
            progress.wall_ms + 50 < host.run_started.elapsed().as_millis() as u64,
            "the ledger counted the park as play: {} ms of {:?}",
            progress.wall_ms,
            host.run_started.elapsed(),
        );
    }

    /// ⚠️ **Both halves of the speed figure have to be the *run's*.** `emulated` restarts with the
    /// game, and the heartbeat's `wall_ms` used to be measured from a clock stamped once at
    /// construction and never reset — so a run started in a process that had been up for hours
    /// opened by reporting its first seconds of play against those hours. The page divides one by
    /// the other, so what that showed was a host at full speed reading as a fraction of realtime,
    /// converging on the truth over the following hours without ever arriving.
    #[test]
    fn a_new_run_measures_itself_against_its_own_clock() {
        use crate::run::RunDir;

        let scratch = crate::run::tests::Scratch::new("host-run-clock");
        let validate = |bytes: &[u8]| GameBoy::dmg(crate::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, _, _) = RunDir::open(&scratch.0, false, "random", &validate).expect("a fresh run");
        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let mut host = host_with(Arc::clone(&published), |config| {
            config.run = Some(Arc::clone(&current));
        });

        // Stand in for a process that had been up a while before this run began.
        let aged = Instant::now() + Duration::from_millis(300);
        while Instant::now() < aged {
            host.tick();
        }
        host.publish_status(Instant::now());
        let before = host.last_status.clone().expect("the first heartbeat is never suppressed");
        assert!(before.wall_ms >= 250, "the warm-up did not happen: {before:?}");

        host.start_new_run().expect("a host built with a run directory can start another");
        host.last_status = None;
        host.publish_status(Instant::now());
        let after = host.last_status.clone().expect("the run change is always published");
        assert!(
            after.wall_ms < before.wall_ms,
            "the new run inherited the old one's wall clock: {} ms against {} ms",
            after.wall_ms,
            before.wall_ms,
        );
        assert_eq!(after.emulated_ms, 0, "the new game has not been played yet");
        assert_eq!(after.dropped_ms, 0, "and it starts owing nothing");
    }

    /// **Send on change.** Every heartbeat that goes out must either say something new or be the
    /// keepalive; a stream of identical ones is 50 kbit/s per viewer of nothing at all, which is
    /// what this used to be.
    #[test]
    fn a_heartbeat_that_says_nothing_new_is_not_sent() {
        let published = Published::new();
        let mut events = published.subscribe_events();
        let mut host = host_with(Arc::clone(&published), |config| {
            // Sampled far faster than anything can change, so every suppression is exercised.
            config.status_interval = Duration::from_millis(1);
            config.status_keepalive = Duration::from_secs(3_600);
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut sent: Vec<StatusSnapshot> = Vec::new();
        while sent.len() < 8 && Instant::now() < deadline {
            host.tick();
            while let Ok(UiEvent { body, .. }) = events.try_recv() {
                if let UiEventBody::Status(status) = body {
                    sent.push(*status);
                }
            }
            std::thread::sleep(Duration::from_micros(200));
        }

        assert!(sent.len() >= 8, "only {} heartbeats arrived — the game was not moving", sent.len());
        for (previous, next) in sent.iter().zip(&sent[1..]) {
            assert!(
                !next.says_the_same_as(previous),
                "a heartbeat repeated what the one before it said, with no keepalive due:\n{previous:?}\n{next:?}",
            );
        }
        // …and the sampling really was faster than the sending, or the assertion above is vacuous.
        let span = sent.last().unwrap().wall_ms - sent[0].wall_ms;
        assert!(span > 0, "every heartbeat landed in the same millisecond");
    }

    /// The other half: a game that is not moving still has to prove it is alive.
    #[test]
    fn an_idle_run_still_sends_a_keepalive() {
        let published = Published::new();
        let mut events = published.subscribe_events();
        // Speed 0.001× — the emulator advances so slowly that nothing observable changes, which is
        // the closest thing to a frozen game a real host can be.
        let mut host = host_with(Arc::clone(&published), |config| {
            config.target_speed = 0.001;
            config.status_interval = Duration::from_millis(1);
            config.status_keepalive = Duration::from_millis(60);
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut sent = 0;
        while sent < 3 && Instant::now() < deadline {
            host.tick();
            while let Ok(UiEvent { body, .. }) = events.try_recv() {
                if matches!(body, UiEventBody::Status(_)) {
                    sent += 1;
                }
            }
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(sent >= 3, "an idle run went silent: {sent} heartbeats");
        assert!(
            Instant::now() < deadline,
            "three 60 ms keepalives should not have taken ten seconds",
        );
    }

    /// The video pipeline end to end from the host's side: what it publishes decodes back to the
    /// emulator's own frame buffer.
    #[test]
    fn the_host_publishes_decodable_video() {
        let published = Published::new();
        let mut host = host(Arc::clone(&published));

        let deadline = Instant::now() + Duration::from_secs(20);
        while published.latest_keyframe().is_none() && Instant::now() < deadline {
            host.tick();
            std::thread::sleep(Duration::from_micros(500));
        }

        let keyframe = published.latest_keyframe().expect("a keyframe should have been published");
        let mut decoder = VideoDecoder::default();
        decoder.apply(&keyframe.bytes).expect("the host's own keyframe should decode");

        let snapshot = published.latest_frame();
        assert_eq!(snapshot.seq, keyframe.seq, "the frame and the keyframe describe the same moment");
        assert_eq!(decoder.pixels(), snapshot.pixels.as_ref());
    }

    /// **W7's acceptance, without a process restart.** A host plays for a while and checkpoints; a
    /// second host, given nothing but the run directory, comes up in the same place — not at the
    /// start of the game, which is what a resume that silently did nothing would look like.
    #[test]
    fn a_checkpointed_run_resumes_where_it_stopped() {
        use crate::pokemon::PokemonApiTrait;
        use crate::run::{Origin, RunDir};

        let scratch = crate::run::tests::Scratch::new("host-resume");
        let validate = |bytes: &[u8]| GameBoy::dmg(crate::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, origin, state) =
            RunDir::open(&scratch.0, false, "random", &validate).expect("a fresh run");
        assert_eq!(origin, Origin::Fresh);
        assert!(state.is_none());

        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let run = current.get();
        let mut host = host_with(Arc::clone(&published), |config| {
            config.run = Some(Arc::clone(&current));
            config.checkpoint_interval = Duration::from_millis(20);
        });

        // Long enough for the agent to have moved and for at least one periodic checkpoint.
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            host.tick();
            if host.emulated.to_duration() > Duration::from_secs(3) && run.path().join("state.gbst").is_file() {
                break;
            }
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(host.emulated.to_duration() > Duration::from_secs(3), "the first host barely ran");
        assert!(run.path().join("state.gbst").is_file(), "the periodic checkpoint never fired");
        // ⚠️ …and then the shutdown checkpoint, which is what makes the comparison below exact: a
        // periodic one is however many ticks old, and the agent has been walking in the meantime.
        host.checkpoint();
        let before = {
            let mut api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
            api.game_state().expect("a readable state")
        };
        drop(host);

        let (resumed, origin, state) =
            RunDir::open(&scratch.0, false, "random", &validate).expect("the run is resumable");
        assert_eq!(origin, Origin::Resumed);
        assert_eq!(resumed.run_id(), run.run_id(), "it continues the same run rather than forking");

        let mut second = EmulatorHost::new(
            &state.expect("a state was checkpointed"),
            Box::new(RandomPolicy::default()),
            Published::new(),
            HostConfig::default(),
        )
        .expect("the checkpoint loads");
        let after = {
            let mut api = PokemonApi::with_cache(&mut second.gb, &mut second.map_cache);
            api.game_state().expect("a readable state")
        };

        assert_eq!(after.map.map, before.map.map);
        assert_eq!(after.map.player_position, before.map.player_position,
                   "the second host started somewhere else — that is a resume that did nothing");
        // …and the SRAM was written beside it, for anything that reads an ordinary .sav.
        let sram = std::fs::read(run.path().join("sram.bin")).expect("sram.bin");
        assert!(!sram.is_empty() && sram.len() % 1024 == 0, "{} bytes is not a bank count", sram.len());
    }

    /// **`POST /api/new-run`'s acceptance, at the seam it actually crosses.** A run that has played
    /// for a while is asked — through the mailbox, exactly as the handler asks — to start again, and
    /// afterwards there are two run directories: the old one complete and resumable, the new one
    /// being played from the beginning.
    ///
    /// The two assertions that matter are the ones a reset is easy to get wrong on. **The outgoing
    /// run must be checkpointed by the swap**, not left however many seconds short at whatever its
    /// last periodic write was — this host is given a checkpoint interval far longer than it runs, so
    /// the only thing that can have written `state.gbst` is `start_new_run` itself. And **the game
    /// must actually be back at the start**: a reset that swapped the directory but left the
    /// emulator running would look identical from the outside for the first few seconds.
    #[test]
    fn a_new_run_starts_in_place_and_leaves_the_old_one_complete() {
        use crate::pokemon::PokemonApiTrait;
        use crate::run::{Origin, RunDir};

        let scratch = crate::run::tests::Scratch::new("host-new-run");
        let validate = |bytes: &[u8]| GameBoy::dmg(crate::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, origin, _) = RunDir::open(&scratch.0, false, "random", &validate).expect("a fresh run");
        assert_eq!(origin, Origin::Fresh);

        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let new_runs = Arc::new(NewRunRequests::default());
        let first = current.get();
        let mut host = host_with(Arc::clone(&published), |config| {
            config.run = Some(Arc::clone(&current));
            config.new_runs = Some(Arc::clone(&new_runs));
            // Longer than this test runs, so no *periodic* checkpoint can fire and the state file
            // below can only have been written by the swap.
            config.checkpoint_interval = Duration::from_secs(3600);
        });

        // Play far enough that the agent has left the start state behind.
        let deadline = Instant::now() + Duration::from_secs(20);
        while host.emulated.to_duration() < Duration::from_secs(8) && Instant::now() < deadline {
            host.tick();
            std::thread::sleep(Duration::from_micros(500));
        }
        let played = {
            let mut api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
            api.game_state().expect("a readable state")
        };
        assert!(host.emulated.to_duration() >= Duration::from_secs(8), "the host barely ran");
        assert!(!first.path().join("state.gbst").exists(), "a periodic checkpoint fired after all");

        // Ask exactly as the handler does, then give the emulator the one tick it needs.
        let receiver = new_runs.request().expect("the mailbox is empty");
        host.tick();
        let run_id = receiver.blocking_recv().expect("the emulator answered").expect("a new run");

        let second = current.get();
        assert_eq!(second.run_id(), run_id);
        assert_ne!(second.run_id(), first.run_id(), "the run directory did not change");
        assert!(first.path().join("state.gbst").is_file(),
                "the outgoing run was not checkpointed — everything since its last write is gone");

        // The game itself is back at the beginning, and the agent has forgotten the old map.
        let restarted = {
            let mut api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
            api.game_state().expect("a readable state")
        };
        let start = {
            let mut fresh = host_with(Published::new(), |_| {});
            let mut api = PokemonApi::with_cache(&mut fresh.gb, &mut fresh.map_cache);
            api.game_state().expect("a readable state")
        };
        assert_eq!(restarted.map.player_position, start.map.player_position,
                   "the emulator kept playing the old game — it was {:?} before the reset",
                   played.map.player_position);
        // Restarted, not merely carried on: the counter is zeroed by the swap and then the *rest of
        // that same tick* emulates into it, so this is "back near zero" rather than exactly it.
        assert!(host.emulated.to_duration() < Duration::from_secs(1),
                "emulated time measures *this* game, but reads {:?}", host.emulated.to_duration());

        // ⚠️ And the next frame is a keyframe, or a viewer keeps fragments of the abandoned run.
        host.publish_video();
        assert!(published.latest_keyframe().is_some_and(|frame| frame.keyframe),
                "the video encoder was not restarted");
    }

    /// **The whole ending, in one test**: the win is noticed, the run is filed, and the next one
    /// starts — with the emulator, a real run directory and the transcript thread all in play.
    ///
    /// Seeded from `post-hall-of-fame.bin`, which is a few emulated seconds short of
    /// `AnimateHallOfFame` (see `postgame::phase0::the_hall_of_fame_is_announced_once_…` for why
    /// that fixture and not one further on). `RandomPolicy` never reaches a decision point during a
    /// cutscene, so what drives the ceremony here is the agent's own text-box reader — which is
    /// exactly what drives it in the deployment.
    #[test]
    #[cfg_attr(not(feature = "slow-tests"), ignore = "drives a cutscene; run with --features slow-tests")]
    fn a_finished_run_is_filed_and_the_next_one_starts() {
        use crate::run::{RunDir, files, hall_of_fame};

        let scratch = crate::run::tests::Scratch::new("host-hall-of-fame");
        let validate = |bytes: &[u8]| GameBoy::dmg(crate::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, _, _) = RunDir::open(&scratch.0, false, "gpt-test", &validate).expect("a fresh run");

        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "gpt-test".to_string(), run));
        let finished = current.get();
        let finished_id = finished.run_id();
        // The transcript thread is running, because the archive follows the file it writes — the one
        // ordering hazard in the whole path, and the thing this test exists to exercise.
        let stop = Arc::new(AtomicBool::new(false));
        let transcript = crate::run::transcript::spawn(
            Arc::clone(&current),
            Arc::clone(&published),
            Arc::clone(&stop),
        )
        .expect("a transcript writer");

        let mut host = host_from(
            include_bytes!("pokemon/data/post-hall-of-fame.bin"),
            Arc::clone(&published),
            |config| {
                config.run = Some(Arc::clone(&current));
                // Longer than this test runs, so nothing periodic can write and every file below can
                // only have been written by the completion path.
                config.checkpoint_interval = Duration::from_secs(3_600);
            },
        );

        let deadline = Instant::now() + Duration::from_secs(60);
        while current.get().run_id() == finished_id && Instant::now() < deadline {
            host.tick();
        }
        stop.store(true, Ordering::Relaxed);

        assert_ne!(current.get().run_id(), finished_id,
                   "the game was won and nothing started the next run");

        // The ledger has it, and it points at an archive that is really there.
        let rows = hall_of_fame::top(&scratch.0, 10);
        assert_eq!(rows.len(), 1, "one championship, one row");
        let row = &rows[0];
        assert_eq!(row.run_id, finished_id);
        assert_eq!(row.teams, 1);
        assert_eq!(row.policy, "random", "the decider names itself");
        assert_eq!(row.model, Some("gpt-test".into()));
        assert_eq!(row.app_version, crate::cli::VERSION);
        assert_eq!(row.badges, 8, "the winning tally is read at the moment of victory");
        assert!(row.playtime_seconds > 0 && !row.playtime_maxed);

        let archive = scratch.0.join(files::HALL_OF_FAME).join(&row.archive);
        assert!(archive.join(files::STATE).is_file(), "the save state at the moment of victory");
        assert!(archive.join(files::SRAM).is_file());
        assert!(archive.join(files::META).is_file());
        assert!(archive.join("transcript.jsonl.gz").is_file(),
                "the run's own story is the point of keeping the directory at all");

        // ⚠️ The victory is *in* the archived transcript. This is what the seq-bounded follow buys:
        // a plain `fs::copy` at this moment gets a file the writer has not reached the end of.
        let gz = std::fs::read(archive.join("transcript.jsonl.gz")).expect("the transcript");
        let mut story = String::new();
        std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&gz[..]), &mut story)
            .expect("it inflates");
        assert!(story.contains("hall_of_fame"),
                "the archived transcript stops short of the event it is a record of");

        // The outgoing directory is complete and stamped, so a resume of it never files this twice.
        assert!(finished.path().join(files::STATE).is_file(), "the outgoing run was checkpointed");
        assert_eq!(finished.meta().completed.len(), 1);
        assert!(finished.already_archived(1));

        // ⚠️ And the archive is invisible to the resume scan — otherwise the next `gb serve` would
        // continue a game that has already been won and filed.
        let (resumed, _, _) = RunDir::open(&scratch.0, false, "gpt-test", &validate).expect("a resume");
        assert_ne!(resumed.run_id(), row.archive, "hall-of-fame/ must not be resumable");

        // ⚠️ The writer is a `blocking_recv` loop, so it only notices `stop` when something wakes
        // it — the same reason `web::run` publishes a parting notice before joining. Without this
        // the test hangs on a run that has nothing left to say.
        published.publish_event(UiEventBody::Notice { level: "info", message: "done".into() });
        let _ = transcript.join();
    }

    /// The mailbox refuses a second request only while someone is still listening for the first —
    /// otherwise a handler that gave up would wedge the endpoint for the life of the process.
    #[test]
    fn the_new_run_mailbox_refuses_a_concurrent_request_but_not_an_abandoned_one() {
        let mailbox = NewRunRequests::default();

        let receiver = mailbox.request().expect("the first is accepted");
        assert!(mailbox.request().is_err(), "a second request while the first is outstanding");

        drop(receiver);
        assert!(mailbox.request().is_ok(), "an abandoned request must not block the next one");
    }
}
