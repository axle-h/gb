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
use crate::pokemon::agent::{AgentEvent, PokemonAgent};
use crate::pokemon::map_metadata::MapMetadataCache;
use crate::pokemon::policy::Policy;
use crate::pokemon::{PokemonApi, PokemonApiTrait, observe};
use crate::run::RunDir;
use crate::web::published::{FrameSnapshot, Published, RunStatus, StatusSnapshot, UiEventBody};
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
    /// What the status heartbeat reports as the decider — `"random"` or `"llm"`.
    pub policy_name: &'static str,
    /// **W7** — where to checkpoint, and how often. `None` is a host that keeps nothing, which is
    /// what every test wants and what `gb serve` never is.
    pub run: Option<Arc<RunDir>>,
    pub checkpoint_interval: Duration,
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
            policy_name: "random",
            run: None,
            checkpoint_interval: Duration::from_secs(60),
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
    started: Instant,
    last_iteration: Instant,
    since_last_update: Duration,
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
}

impl EmulatorHost {
    /// Build a host running `policy` from `save_state`.
    ///
    /// The state is a **DMG** save — every committed fixture is, including the one `gb serve` starts
    /// from — so the emulator is built as a DMG to match. Colour is reachable through the API (see
    /// `CLAUDE.md`) but a CGB emulator restoring a DMG snapshot is not a combination anything has
    /// ever exercised, and the video path is hard enough to debug without it.
    pub fn new(
        save_state: &[u8],
        policy: Box<dyn Policy>,
        published: Arc<Published>,
        config: HostConfig,
    ) -> Result<Self, String> {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(save_state).map_err(|e| format!("could not load the starting state: {e}"))?;
        // ⚠️ **§12's note, left here because this is where it will bite.** `Audio::set_output_sample_rate`
        // is not serialised, so anything that loads a state has to re-apply it — see the `F9` handler
        // in `render.rs`. Nothing here consumes audio while §12 is deferred; the moment a stream is
        // added, this line is where the rate goes back on.

        let now = Instant::now();
        let cycle_duration = REALTIME_CYCLE_DURATION.div_f64(config.target_speed.max(f64::MIN_POSITIVE));
        let first_checkpoint = now + config.checkpoint_interval;
        Ok(Self {
            gb,
            agent: PokemonAgent::new(policy),
            map_cache: MapMetadataCache::default(),
            published,
            encoder: VideoEncoder::default(),
            config,
            cycle_duration,
            started: now,
            last_iteration: now,
            since_last_update: Duration::ZERO,
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
        })
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
        let Some(run) = self.config.run.clone() else { return };
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
        let emulated_ms = self.emulated.to_duration().as_millis() as u64;
        if let Err(failure) = run.checkpoint(&state, &self.gb.dump_sram(), emulated_ms) {
            self.published.publish_event(UiEventBody::Notice {
                level: "error",
                message: format!("could not checkpoint the run: {failure}"),
            });
        }
    }

    /// One iteration of the loop. Returns whether any emulation happened, which is the loop's cue
    /// that it is behind and should come straight back rather than sleep.
    ///
    /// Separate from [`Self::run`] so a test can drive the host deterministically instead of racing
    /// a thread.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        self.since_last_update += now.saturating_duration_since(self.last_iteration).min(MAX_CATCHUP);
        self.last_iteration = now;

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
            if let Err(failure) = self.agent.update(&mut api, ran) {
                self.published.publish_event(UiEventBody::Notice {
                    level: "error",
                    message: format!("agent tick failed: {failure}"),
                });
            }
            let events = self.agent.drain_events();
            for event in events {
                self.published.publish_event(UiEventBody::Agent {
                    kind: event_kind(&event),
                    text: format!("{event}"),
                });
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
            wall_ms: now.duration_since(self.started).as_millis() as u64,
            emulated_ms: self.emulated.to_duration().as_millis() as u64,
            target_speed: self.config.target_speed,
            policy: self.config.policy_name,
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
            let message = "the emulator thread panicked — the stream is frozen from here";
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
        AgentEvent::BattleStarted => "battle_started",
        AgentEvent::BattleActionStarted { .. } => "battle_action_started",
        AgentEvent::BattleEnded => "battle_ended",
        AgentEvent::TextBox { .. } => "text_box",
        // **W9.** Styled loudly by the page, and the one agent event that is a bug report rather
        // than a narration — see `AgentEvent::WatchdogFired`.
        AgentEvent::WatchdogFired { .. } => "watchdog",
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
        let mut config = HostConfig {
            // Fast enough that a fraction of a second of wall clock is seconds of game time, so the
            // test is not at the mercy of how quickly the scheduler comes back to it.
            target_speed: 40.0,
            video_interval: Duration::from_millis(5),
            status_interval: Duration::from_millis(5),
            ..HostConfig::default()
        };
        tweak(&mut config);
        EmulatorHost::new(
            crate::pokemon::data::START_OF_GAME,
            Box::new(RandomPolicy),
            published,
            config,
        )
        .expect("the committed start-of-game fixture should load")
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
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &*keyframe.data)
            .expect("the host produced this");
        let mut decoder = VideoDecoder::default();
        decoder.apply(&bytes).expect("the host's own keyframe should decode");

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
        let run = Arc::new(run);
        let mut host = host_with(Arc::clone(&published), |config| {
            config.run = Some(Arc::clone(&run));
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
            Box::new(RandomPolicy),
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
}
