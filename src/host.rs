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
use crate::web::published::{FrameSnapshot, Published, StatusSnapshot, UiEventBody};
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
    pub status_interval: Duration,
    /// What the status heartbeat reports as the decider — `"random"` or `"llm"`.
    pub policy_name: &'static str,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            target_speed: 1.0,
            video_interval: Duration::from_nanos(1_000_000_000 / 30),
            status_interval: Duration::from_millis(100),
            policy_name: "random",
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

        let now = Instant::now();
        let cycle_duration = REALTIME_CYCLE_DURATION.div_f64(config.target_speed.max(f64::MIN_POSITIVE));
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
        save_state: &'static [u8],
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
                let mut host = match Self::new(save_state, policy(), published, config) {
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

    fn publish_status(&mut self, now: Instant) {
        // `game_state` reads a lot of RAM and can legitimately fail mid-transition. A heartbeat that
        // says "no game state" is far easier to diagnose than one that stops arriving.
        let game = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache)
            .game_state()
            .ok()
            .map(|state| observe::status(&state));
        self.published.publish_event(UiEventBody::Status(Box::new(StatusSnapshot {
            wall_ms: now.duration_since(self.started).as_millis() as u64,
            emulated_ms: self.emulated.to_duration().as_millis() as u64,
            target_speed: self.config.target_speed,
            policy: self.config.policy_name,
            agent_state: self.agent.state_debug(),
            frame_seq: self.encoder.seq(),
            game,
        })));
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::policy::RandomPolicy;
    use crate::web::published::{UiEvent, UiEventBody};
    use crate::web::video::VideoDecoder;

    fn host(published: Arc<Published>) -> EmulatorHost {
        EmulatorHost::new(
            crate::pokemon::data::START_OF_GAME,
            Box::new(RandomPolicy),
            published,
            HostConfig {
                // Fast enough that a fraction of a second of wall clock is seconds of game time, so
                // the test is not at the mercy of how quickly the scheduler comes back to it.
                target_speed: 40.0,
                video_interval: Duration::from_millis(5),
                status_interval: Duration::from_millis(5),
                policy_name: "random",
            },
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
}
