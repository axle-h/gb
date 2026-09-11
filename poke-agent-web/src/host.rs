use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gb::cycles::MachineCycles;
use gb::game_boy::GameBoy;
use gb::model::Model;
use poke_agent::pokemon::agent::{AgentEvent, PokemonAgent};
use poke_agent::pokemon::map_metadata::MapMetadataCache;
use poke_agent::pokemon::policy::{LLM_POLICY_NAME, Policy};
use poke_agent::pokemon::{PokemonApi, PokemonApiTrait, observe};
use poke_agent::run::{CurrentRun, RunProgress};
use poke_agent::published::{
    FrameSnapshot, Published, RunStatus, StatusSnapshot, UiEventBody, now_ms,
};
use crate::web::audio::{self, AudioEncoder};
use crate::web::video::{Frame, VideoEncoder};

/// One machine cycle at the real hardware's rate — the unit the pacing loop spends wall clock in.
const REALTIME_CYCLE_DURATION: Duration = MachineCycles::from_m(1).to_duration();

/// How long the loop sleeps when it is ahead of the clock.
const IDLE_SLEEP: Duration = Duration::from_millis(1);

/// The most wall clock one iteration will try to make up.
const MAX_CATCHUP: Duration = Duration::from_millis(250);

/// The emulator thread's stack.
const EMULATOR_STACK: usize = 8 * 1024 * 1024;

pub struct HostConfig {
    /// Emulation speed as a multiple of real time: 1.0 for a livestream, large in tests.
    pub target_speed: f64,
    /// The Opus stream's target, in bits per second, or `None` for no audio at all.
    pub audio_bitrate: Option<i32>,
    /// Wall-clock spacing of video messages, so running fast does not multiply bandwidth.
    pub video_interval: Duration,
    /// How often the game state is sampled; each sample costs a `game_state()` read.
    pub status_interval: Duration,
    /// How long a heartbeat may be suppressed for saying nothing new before one is sent anyway.
    pub status_keepalive: Duration,
    /// Where to checkpoint, and how often. `None` keeps nothing, which is what every test wants.
    pub run: Option<Arc<CurrentRun>>,
    pub checkpoint_interval: Duration,
    /// The admin endpoints' mailbox. `None` means the emulator never checks.
    pub control: Option<Arc<ControlRequests>>,
    /// Which Game Boy the cartridge runs on, from `GB_HARDWARE`. [`Model::Dmg`] by default.
    pub model: Model,
    /// Whether the state this host is starting from is a new game rather than a resumed one.
    pub fresh_game: bool,
}

/// What the HTTP layer is allowed to ask the emulator thread for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRequest {
    /// `POST /api/new-run` and `/reset-game`: checkpoint this run and start again in a new directory.
    NewRun,
    /// `POST /api/clear`: keep the game and throw away the model's conversation and plan.
    ClearConversation,
}

impl ControlRequest {
    /// How the refusal below names whatever is already outstanding.
    fn describe(self) -> &'static str {
        match self {
            Self::NewRun => "a new run is already being started",
            Self::ClearConversation => "the conversation is already being cleared",
        }
    }
}

/// Where the answer goes: the run id the emulator acted on, or why it could not.
type Answer = tokio::sync::oneshot::Sender<Result<String, String>>;

/// The one channel from the HTTP layer back into the emulator thread.
#[derive(Default)]
pub struct ControlRequests {
    pending: std::sync::Mutex<Option<(ControlRequest, Answer)>>,
}

impl ControlRequests {
    /// Ask for something. The receiver resolves with the run id once the emulator has acted.
    pub fn request(
        &self,
        what: ControlRequest,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<String, String>>, String> {
        let mut pending = self.pending.lock().expect("control mailbox poisoned");
        if let Some((outstanding, sender)) = pending.as_ref()
            && !sender.is_closed()
        {
            return Err(outstanding.describe().to_string());
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        *pending = Some((what, sender));
        Ok(receiver)
    }

    fn take(&self) -> Option<(ControlRequest, Answer)> {
        self.pending.lock().expect("control mailbox poisoned").take()
    }
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            target_speed: 1.0,
            audio_bitrate: Some(crate::web::audio::DEFAULT_BITRATE),
            video_interval: Duration::from_nanos(1_000_000_000 / 30),
            status_interval: Duration::from_millis(500),
            status_keepalive: Duration::from_secs(2),
            run: None,
            checkpoint_interval: Duration::from_secs(60),
            control: None,
            // Every test loads a game in progress, so the default leaves the trainer's name alone.
            fresh_game: false,
            // The DMG is the default and the tests depend on it.
            model: Model::Dmg,
        }
    }
}

/// Re-apply everything about the APU that a save state does not carry.
fn tune_audio(gb: &mut GameBoy, target_speed: f64) {
    let audio = gb.core_mut().mmu_mut().audio_mut();
    audio.set_output_sample_rate(audio::SAMPLE_RATE);
    audio.set_emulation_speed(target_speed.max(f64::MIN_POSITIVE));
}

pub struct EmulatorHost {
    gb: GameBoy,
    agent: PokemonAgent,
    map_cache: MapMetadataCache,
    published: Arc<Published>,
    encoder: VideoEncoder,
    /// `None` when `HostConfig::audio_bitrate` is, which is the whole of "audio is off".
    audio: Option<AudioEncoder>,
    /// One read's worth of PCM, sized past the blip buffer's 100 ms so one read empties it.
    audio_scratch: Vec<f32>,
    /// Reused so a packet costs one `Arc` and not a `Vec` as well.
    audio_packets: Vec<Arc<[u8]>>,
    /// Whether anyone was listening on the previous tick.
    audio_listeners: bool,
    config: HostConfig,

    cycle_duration: Duration,
    last_iteration: Instant,
    since_last_update: Duration,
    /// Emulated time the catch-up clamp has thrown away on this run.
    dropped: Duration,
    /// The last tick the run was found parked, and the total wall clock it has spent parked.
    paused_since: Option<Instant>,
    paused_total: Duration,
    /// Cycles `gb.run` delivered beyond what was asked, spent down before more are requested.
    ahead_by_cycles: MachineCycles,
    emulated: MachineCycles,
    next_video: Instant,
    next_status: Instant,
    next_checkpoint: Instant,
    /// The last heartbeat *sent*, and when.
    last_status: Option<StatusSnapshot>,
    last_status_at: Instant,
    /// Whether the first cycle has been emulated; see [`Self::tick`].
    booted: bool,

    /// When the current run started being played here.
    run_started: Instant,
    /// [`Published::turns`] as of the moment this run became current.
    turns_at_run_start: u64,
    /// [`AgentEvent::WatchdogFired`]s seen this run.
    watchdog_firings: u64,
    completed: Option<(AgentEvent, u64)>,
    /// Whether the run was waiting on the model at the previous tick, to spot the transition.
    awaiting_llm: bool,
    /// The last `agent.update` failure that was published.
    last_agent_failure: Option<String>,
}

impl EmulatorHost {
    /// Build a host running `policy` from `save_state`.
    pub fn new(
        save_state: &[u8],
        policy: Box<dyn Policy>,
        published: Arc<Published>,
        config: HostConfig,
    ) -> Result<Self, String> {
        let mut gb = GameBoy::new(poke_agent::pokemon::roms::POKERED, config.model);
        gb.load_state(save_state).map_err(|e| format!("could not load the starting state: {e}"))?;
        tune_audio(&mut gb, config.target_speed);

        let now = Instant::now();
        let cycle_duration = REALTIME_CYCLE_DURATION.div_f64(config.target_speed.max(f64::MIN_POSITIVE));
        let first_checkpoint = now + config.checkpoint_interval;
        // Read rather than assumed zero, so a shared `Published` cannot credit this run with
        // another's turns.
        let turns_at_run_start = published.turns();
        let mut host = Self {
            gb,
            agent: PokemonAgent::new(policy),
            map_cache: MapMetadataCache::default(),
            published,
            encoder: VideoEncoder::default(),
            audio: config.audio_bitrate.map(AudioEncoder::new),
            audio_scratch: vec![0.0; audio::SAMPLE_RATE as usize / 8 * 2],
            audio_packets: Vec::new(),
            audio_listeners: false,
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
            // One interval out: the state just loaded was just checkpointed, and rewriting it at
            // startup would have a crash-looping process rewrite its save every few seconds.
            next_checkpoint: first_checkpoint,
            awaiting_llm: false,
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
    fn name_the_player(&mut self) {
        let Some(name) = self.agent.policy_player_name() else { return };
        let written = {
            let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
            api.write_player_name(&name)
        };
        match written {
            Ok(()) => println!("poke-agent-web — the player is called {name}"),
            Err(error) => {
                self.published.publish_event(UiEventBody::Notice {
                    level: "warn",
                    message: format!("could not name the player {name}: {error}"),
                });
            }
        }
    }

    /// Build a host on a new thread and run it there until `shutdown` is set.
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
        self.checkpoint();
    }

    /// Write the run's state to disk, if this host has somewhere to write it.
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

    /// What the current run had been played for before this process opened it; zero if fresh.
    fn run_baseline(&self) -> RunProgress {
        self.config.run.as_ref().map(|current| current.get().baseline()).unwrap_or_default()
    }

    /// What this process has contributed to the current run since it opened it.
    fn progress(&self) -> RunProgress {
        let usage = self.published.usage();
        RunProgress {
            emulated_ms: self.emulated.to_duration().as_millis() as u64,
            // Minus the park, the same as the heartbeat's.
            wall_ms: self.run_started.elapsed().saturating_sub(self.paused_total).as_millis() as u64,
            prompt_tokens: usage.map_or(0, |u| u.prompt_tokens),
            completion_tokens: usage.map_or(0, |u| u.completion_tokens),
            completions: usage.map_or(0, |u| u.completions),
            turns: self.published.turns().saturating_sub(self.turns_at_run_start),
            watchdog_firings: self.watchdog_firings,
        }
    }

    /// File a finished run in the Hall of Fame and begin the next one.
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

        // Read here: `game_state()` needs the emulator, and the archive job does not have it.
        let (badges, pokedex_owned, pokedex_seen, money, party, playtime_maxed) = self.final_state();
        // After the checkpoint above, which folds this process's figures onto the run's baseline.
        let meta = run.meta();
        let usage = self.published.usage();
        let job = poke_agent::run::hall_of_fame::ArchiveJob {
            root: current.root().to_path_buf(),
            run_dir: run.path().to_path_buf(),
            state,
            sram: self.gb.dump_sram(),
            until_seq: seq,
            completion: poke_agent::run::hall_of_fame::Completion {
                archive: String::new(), // filled in by `archive`, which chooses the directory
                run_id: meta.run_id.clone(),
                teams: *teams,
                completed_at: poke_agent::run::iso8601(std::time::SystemTime::now()),
                started_at: meta.started_at.clone(),
                app_version: crate::cli::VERSION.to_string(),
                policy: self.agent.policy_name().to_string(),
                // `RunMeta::model` is the policy's name under every policy but the LLM.
                model: (self.agent.policy_name() == LLM_POLICY_NAME)
                    .then(|| meta.model.clone()),
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

        let archived = match poke_agent::run::hall_of_fame::archive(&job) {
            Ok(name) => name,
            // Names the directory: this is not retried, so filing it by hand is a `cp`.
            Err(failure) => {
                return self.complain(format!(
                    "could not file the finished run: {failure}. {} won the game and is complete on \
                     disk, but is not in the leaderboard",
                    job.run_dir.display(),
                ));
            }
        };
        if let Err(failure) = run.record_completion(poke_agent::run::hall_of_fame::recorded(*teams, archived.clone())) {
            // The archive and the ledger row are written; only the idempotence stamp failed.
            self.complain(format!("could not stamp the finished run's meta.json: {failure}"));
        }

        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: format!(
                "🏆 {} finished the game in {playtime}, filed as {}/{archived}",
                run.run_id(),
                poke_agent::run::files::HALL_OF_FAME,
            ),
        });
        println!("poke-agent-web — {} finished the game, filed as {archived}", run.run_id());

        match self.start_new_run() {
            Ok(run_id) => println!("poke-agent-web — playing again as {run_id}"),
            Err(failure) => self.complain(format!("could not start the next run: {failure}")),
        }
    }

    /// The winning run's final tally, for the ledger row.
    fn final_state(&mut self) -> (u32, usize, usize, u32, Vec<poke_agent::run::hall_of_fame::PartyMember>, bool) {
        use poke_agent::pokemon::symbols::{DmgPointerRead, pokered_symbols};
        let api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        let maxed = api.mmu().read_pointer(&pokered_symbols::wPlayTimeMaxed) != 0;
        let Ok(state) = api.game_state() else { return (0, 0, 0, 0, Vec::new(), maxed) };
        let party = state
            .pokemon
            .iter()
            .map(|mon| poke_agent::run::hall_of_fame::PartyMember {
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

    /// Publish an error notice and print it.
    fn complain(&self, message: String) {
        eprintln!("poke-agent-web — {message}");
        self.published.publish_event(UiEventBody::Notice { level: "error", message });
    }

    /// Abandon the current run and start the game again in a fresh run directory.
    fn start_new_run(&mut self) -> Result<String, String> {
        let Some(current) = self.config.run.clone() else {
            return Err("this host has no run directory, so there is no new run to start".to_string());
        };
        self.checkpoint();
        let previous = current.get().run_id();
        let run = current.start_new()?;
        let run_id = run.run_id();

        self.gb = GameBoy::new(poke_agent::pokemon::roms::POKERED, self.config.model);
        self.gb
            .load_state(poke_agent::pokemon::data::START_OF_GAME)
            .map_err(|e| format!("could not load the start-of-game state: {e}"))?;
        self.map_cache = MapMetadataCache::default();
        self.agent.restart(Some(run.path()));
        // A new game names its trainer again, since the policy may have changed.
        self.name_the_player();

        self.encoder.restart();
        // The reload above dropped both APU settings.
        tune_audio(&mut self.gb, self.config.target_speed);
        if let Some(audio) = self.audio.as_mut() {
            audio.restart();
        }
        self.audio_packets.clear();
        self.last_status = None;
        self.emulated = MachineCycles::ZERO;
        // Everything that measures *this game* rather than this process starts again with it.
        self.run_started = Instant::now();
        self.turns_at_run_start = self.published.turns();
        self.published.forget_usage();
        self.watchdog_firings = 0;
        self.completed = None;
        self.last_agent_failure = None;
        self.awaiting_llm = false;
        self.ahead_by_cycles = MachineCycles::ZERO;
        self.since_last_update = Duration::ZERO;
        self.dropped = Duration::ZERO;
        self.last_iteration = Instant::now();
        // Released here as well as by the worker on its way out of the park.
        self.published.set_throttled_until(0);
        self.paused_since = None;
        self.paused_total = Duration::ZERO;
        self.next_checkpoint = Instant::now() + self.config.checkpoint_interval;
        self.published.set_status(RunStatus::Playing);
        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: format!("new run {run_id}. {previous} was checkpointed and left where it is"),
        });
        println!("poke-agent-web — new run {run_id} (was {previous})");
        Ok(run_id)
    }

    /// Throw away the model's memory of this run and leave the run itself alone.
    fn clear_conversation(&mut self) -> Result<String, String> {
        let run = self.config.run.as_ref().map(|current| current.get());
        self.agent.clear_conversation(run.as_ref().map(|run| run.path()))?;
        let run_id = run.map(|run| run.run_id()).unwrap_or_default();
        self.published.publish_event(UiEventBody::Notice {
            level: "info",
            message: "the model's conversation and plan were cleared. It starts again from the \
                      system prompt at its next turn; the game, the run and the battle script are \
                      untouched"
                .to_string(),
        });
        println!("poke-agent-web — cleared the conversation and the plan for {run_id}");
        Ok(run_id)
    }

    /// One iteration. Returns whether anything was emulated, the loop's cue not to sleep.
    pub fn tick(&mut self) -> bool {
        // The reset seam.
        if let Some((what, sender)) = self.config.control.as_ref().and_then(|mailbox| mailbox.take()) {
            let answer = match what {
                ControlRequest::NewRun => self.start_new_run(),
                ControlRequest::ClearConversation => self.clear_conversation(),
            };
            let _ = sender.send(answer);
        }
        // The end of the game, answered here rather than where the event is found: filing a run
        // swaps the run directory under the transcript thread, which must not happen mid-tick.
        if let Some((event, seq)) = self.completed.take() {
            self.file_completed_run(&event, seq);
        }

        let now = Instant::now();

        // The pause seam.
        let paused = self.paused_for(now);
        if paused {
            // No catch-up debt.
            self.last_iteration = now;
            self.since_last_update = Duration::ZERO;
        } else {
            let gap = now.saturating_duration_since(self.last_iteration);
            // Counted, for the heartbeat's `dropped_ms`.
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
            // The one status transition the emulator owns, and it happens once.
            if !self.booted {
                self.booted = true;
                self.published.set_status(RunStatus::Playing);
            }
            // `agent.run`, not `gb.run` and one `agent.update`.
            let result;
            (ran, result) = self.agent.run(&mut self.gb, &mut self.map_cache, min_cycles);
            self.emulated += ran;
            self.ahead_by_cycles += ran - min_cycles;

            // On change, not on every failure.
            match result {
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
            // A save state for whatever the model is about to complain about.
            let awaiting = matches!(self.published.run_status(), RunStatus::AwaitingLlm { .. });
            if awaiting && !self.awaiting_llm {
                match self.gb.save_state() {
                    Ok(state) => self.published.publish_save_state(state),
                    // Nothing here may cost a tick.
                    Err(failure) => eprintln!("could not capture a turn's save state: {failure}"),
                }
            }
            self.awaiting_llm = awaiting;

            let events = self.agent.drain_events();
            for event in events {
                let seq = self.published.publish_event(UiEventBody::Agent {
                    kind: event_kind(&event),
                    text: format!("{event}"),
                });
                match event {
                    AgentEvent::WatchdogFired { .. } => self.watchdog_firings += 1,
                    // Filed at the top of the next tick, where swapping the run directory is safe.
                    event @ AgentEvent::HallOfFame { .. } => {
                        self.completed.get_or_insert((event, seq));
                    }
                    _ => {}
                }
            }
        }

        self.drain_audio();

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

    /// Whether the run is parked on a spent quota, and the wall-clock bookkeeping that goes with
    /// it.
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

    /// Take whatever the APU has synthesised since the last tick and put it on the wire.
    fn drain_audio(&mut self) {
        // Off, or given up.
        if self.audio.as_ref().is_none_or(AudioEncoder::silenced) {
            self.set_audio_output(false);
            return;
        }
        if self.published.audio_listeners() == 0 {
            self.audio_listeners = false;
            self.set_audio_output(false);
            return;
        }
        if !self.audio_listeners {
            self.audio_listeners = true;
            if let Some(audio) = self.audio.as_mut() {
                audio.restart();
            }
            // Ahead of the drain rather than after it, so the two throw the same moment away:
            // `set_output_enabled` clears the blip buffer, and the read below then finds nothing.
            self.set_audio_output(true);
            while self.gb.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut self.audio_scratch) > 0 {}
            return;
        }
        // Every tick, not just on the edge: `MMU::reset` replaces the whole `Audio` and a
        // `load_state` does not carry derived state, so a gate set once would silently reopen.
        self.set_audio_output(true);

        loop {
            let frames = self.gb.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut self.audio_scratch);
            if frames == 0 {
                break;
            }
            // Two statements: the borrow checker will not lend both fields at once through `self`.
            let (audio, scratch) = (self.audio.as_mut(), &self.audio_scratch[..frames * 2]);
            let Some(audio) = audio else { break };
            let silenced_before = audio.silenced();
            audio.push(scratch, &mut self.audio_packets);
            if !silenced_before && audio.silenced() {
                self.published.publish_event(UiEventBody::Notice {
                    level: "error",
                    message: "the Opus encoder failed; audio is off for the rest of this process"
                        .to_string(),
                });
            }
        }
        for packet in self.audio_packets.drain(..) {
            self.published.publish_audio(packet);
        }
    }

    /// Run the APU's mixer and resampler, or do not.
    fn set_audio_output(&mut self, enabled: bool) {
        self.gb.core_mut().mmu_mut().audio_mut().set_output_enabled(enabled);
    }

    fn publish_video(&mut self) {
        // Copied out: the encoder borrows `self` mutably and the LCD lives inside the `GameBoy`.
        let frame: Box<Frame> = Box::new(*self.gb.core().mmu().ppu().lcd());
        let Some(delta) = self.encoder.encode(&frame) else {
            return; // nothing moved on screen, so nothing goes on the wire
        };
        let keyframe = self.encoder.keyframe().expect("something was just encoded");
        self.published.publish_frame(FrameSnapshot { seq: delta.seq, pixels: frame });
        self.published.publish_video(keyframe, delta);
    }

    /// Sample the game state and publish it if it says something new or the keepalive is due.
    fn publish_status(&mut self, now: Instant) {
        // `game_state` reads a lot of RAM and can legitimately fail mid-transition.
        let api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        let game = api.game_state().ok().map(|state| observe::status(&state, &api));
        let snapshot = StatusSnapshot {
            // `run_started`, not the process's own clock.
            wall_ms: now
                .duration_since(self.run_started)
                .saturating_sub(self.paused_total)
                .as_millis() as u64,
            emulated_ms: self.emulated.to_duration().as_millis() as u64,
            // The run's clock, read from the run directory on every heartbeat.
            run_emulated_ms: self.run_baseline().emulated_ms
                + self.emulated.to_duration().as_millis() as u64,
            dropped_ms: self.dropped.as_millis() as u64,
            target_speed: self.config.target_speed,
            // Asked of the decider rather than configured beside it, so the two cannot disagree.
            policy: self.agent.policy_name(),
            // Who is playing, for the page's title and header.
            model: (self.agent.policy_name() == LLM_POLICY_NAME)
                .then(|| self.config.run.as_ref().map(|run| run.model().to_string()))
                .flatten(),
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

/// Advance a periodic deadline.
fn schedule_next(deadline: Instant, now: Instant, interval: Duration) -> Instant {
    let next = deadline + interval;
    if next > now { next } else { now + interval }
}

fn event_kind(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::StartedOverworldAction { .. } => "started_overworld_action",
        AgentEvent::OverworldActionAborted { .. } => "overworld_action_aborted",
        AgentEvent::OverworldActionCompleted { .. } => "overworld_action_completed",
        // The page keys off this to not draw a row (`useEventStream`'s `fold`); merged into the
        // line above, every conversation would reappear in the log.
        AgentEvent::OverworldInteractionCompleted { .. } => "overworld_interaction_completed",
        // Not in `useEventStream`'s `UNLOGGED` set, unlike the line above.
        AgentEvent::OverworldPickupFailed { .. } => "overworld_pickup_failed",
        AgentEvent::BattleStarted => "battle_started",
        AgentEvent::BattleActionStarted { .. } => "battle_action_started",
        AgentEvent::BattleEnded => "battle_ended",
        AgentEvent::TextBox { .. } => "text_box",
        AgentEvent::WatchdogFired { .. } => "watchdog",
        // Must not join the kinds `useEventStream`'s `fold` drops.
        AgentEvent::HallOfFame { .. } => "hall_of_fame",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poke_agent::pokemon::policy::RandomPolicy;
    use poke_agent::published::{UiEvent, UiEventBody};
    use crate::web::video::VideoDecoder;

    fn host(published: Arc<Published>) -> EmulatorHost {
        host_with(published, |_| {})
    }

    fn host_with(published: Arc<Published>, tweak: impl FnOnce(&mut HostConfig)) -> EmulatorHost {
        host_from(poke_agent::pokemon::data::START_OF_GAME, published, tweak)
    }

    fn host_from(
        state: &[u8],
        published: Arc<Published>,
        tweak: impl FnOnce(&mut HostConfig),
    ) -> EmulatorHost {
        let mut config = HostConfig {
            // Fast enough that a fraction of a second of wall clock is seconds of game time.
            target_speed: 40.0,
            video_interval: Duration::from_millis(5),
            status_interval: Duration::from_millis(5),
            ..HostConfig::default()
        };
        tweak(&mut config);
        EmulatorHost::new(state, Box::new(RandomPolicy::seeded(1)), published, config)
            .expect("the committed fixture should load")
    }

    #[test]
    fn the_host_publishes_a_moving_game_state() {
        let published = Published::new();
        let mut events = published.subscribe_events();
        let mut host = host(Arc::clone(&published));

        // A safety net: the loop leaves once forty heartbeats have arrived.
        let deadline = Instant::now() + Duration::from_secs(120);
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

        // `RandomPolicy` walks Red around his bedroom, so something has to move.
        let positions: std::collections::HashSet<_> =
            statuses.iter().filter_map(|s| s.game.as_ref()).map(|g| (g.position.x, g.position.y)).collect();
        assert!(positions.len() > 1, "the player never moved: {positions:?}");
    }

    /// The pause seam.
    #[test]
    fn a_parked_run_stops_the_game_but_keeps_the_page_fed() {
        let published = Published::new();
        let mut events = published.subscribe_events();
        // The keepalive is the only thing that speaks while a run is parked.
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

        // And it releases on the deadline alone.
        published.set_throttled_until(now_ms() - 1);
        let resumed = Instant::now() + Duration::from_millis(200);
        while Instant::now() < resumed {
            host.tick();
        }
        assert!(host.emulated > moving, "the game did not resume once the deadline had passed");

        assert!(
            host.since_last_update < host.cycle_duration * 2,
            "the park left catch-up debt: {:?}",
            host.since_last_update,
        );

        // And the run does not report the wait as time it spent playing.
        let progress = host.progress();
        assert!(
            progress.wall_ms + 50 < host.run_started.elapsed().as_millis() as u64,
            "the ledger counted the park as play: {} ms of {:?}",
            progress.wall_ms,
            host.run_started.elapsed(),
        );
    }

    /// Both halves of the speed figure have to be the *run's*.
    #[test]
    fn a_new_run_measures_itself_against_its_own_clock() {
        use poke_agent::run::RunDir;

        let scratch = poke_agent::run::Scratch::new("host-run-clock");
        let validate = |bytes: &[u8]| GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok();
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
        // Cleared so the heartbeat cannot be suppressed as unchanged.
        host.last_status = None;
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

    /// The panel's "played" is the run's total, so it survives the process serving it.
    #[test]
    fn a_resumed_run_reports_the_play_that_came_before_it() {
        use poke_agent::run::RunDir;

        let scratch = poke_agent::run::Scratch::new("host-run-total");
        let validate = |bytes: &[u8]| GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok();

        // The first process: play a little, then checkpoint, which is what a `SIGTERM` does.
        let (run, _, _) = RunDir::open(&scratch.0, false, "random", &validate).expect("a fresh run");
        let first = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let mut host = host_with(Published::new(), |config| config.run = Some(Arc::clone(&first)));
        assert_eq!(host.run_baseline(), RunProgress::default(), "a fresh run has nothing behind it");
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            host.tick();
        }
        host.checkpoint();
        let played = host.run_baseline().emulated_ms + host.emulated.to_duration().as_millis() as u64;
        assert!(played > 0, "the first process emulated nothing at all");
        drop(host);

        // The second process, resuming the same directory, the newest under the root.
        let (run, origin, state) = RunDir::open(&scratch.0, false, "random", &validate).expect("the resume");
        assert_eq!(origin, poke_agent::run::Origin::Resumed);
        let second = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let mut host = host_from(&state.expect("a resumed run has a state"), Published::new(), |config| {
            config.run = Some(Arc::clone(&second));
        });
        assert_eq!(
            host.run_baseline().emulated_ms,
            played,
            "the resume did not pick up what the first process wrote",
        );

        host.last_status = None;
        host.publish_status(Instant::now());
        let resumed = host.last_status.clone().expect("the first heartbeat is never suppressed");
        assert!(
            resumed.emulated_ms < played,
            "this process cannot have played the whole run: {} ms of {played}",
            resumed.emulated_ms,
        );
        assert_eq!(
            resumed.run_emulated_ms,
            played + resumed.emulated_ms,
            "the heartbeat reported this process's share as the run's total",
        );

        // A new run owes nothing: the baseline is the current directory's.
        host.start_new_run().expect("a host built with a run directory can start another");
        host.last_status = None;
        host.publish_status(Instant::now());
        let fresh = host.last_status.clone().expect("the run change is always published");
        assert_eq!(fresh.run_emulated_ms, 0, "the new run inherited the old one's clock");
    }

    /// Send on change.
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
        // …and the sampling was faster than the sending, or the assertion above is vacuous.
        let span = sent.last().unwrap().wall_ms - sent[0].wall_ms;
        assert!(span > 0, "every heartbeat landed in the same millisecond");
    }

    /// The other half: a game that is not moving still has to prove it is alive.
    #[test]
    fn an_idle_run_still_sends_a_keepalive() {
        let published = Published::new();
        let mut events = published.subscribe_events();
        // At 0.001× nothing observable changes: the closest a real host gets to a frozen game.
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

    /// What the host publishes decodes back to the emulator's own frame buffer.
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

    /// A parked run publishes no audio, and the sound picks up again from live on release.
    #[test]
    fn a_parked_run_stops_the_sound_and_picks_it_up_again_from_live() {
        let published = Published::new();
        let mut listener = published.join_audio();
        let mut host = host(Arc::clone(&published));

        // Playing: packets arrive.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut before = 0;
        while before < 5 && Instant::now() < deadline {
            host.tick();
            while listener.try_recv().is_ok() {
                before += 1;
            }
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(before >= 5, "the run was not producing audio to begin with");

        // Parked: the emulator is stopped, so there is nothing to encode and nothing is sent.
        published.set_throttled_until(now_ms() + 30_000);
        while listener.try_recv().is_ok() {}
        let parked_until = Instant::now() + Duration::from_millis(300);
        let mut during = 0;
        while Instant::now() < parked_until {
            host.tick();
            while listener.try_recv().is_ok() {
                during += 1;
            }
            std::thread::sleep(Duration::from_micros(500));
        }
        assert_eq!(during, 0, "{during} packets were published while the game was stopped");

        // Released: it resumes, and the listener is still attached to hear it.
        published.set_throttled_until(now_ms() - 1);
        let resumed_by = Instant::now() + Duration::from_secs(20);
        let mut after = 0;
        while after < 5 && Instant::now() < resumed_by {
            host.tick();
            while listener.try_recv().is_ok() {
                after += 1;
            }
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(after >= 5, "the sound never came back after the park; only {after} packets");
    }

    /// Nobody listening means the APU does not synthesise.
    #[test]
    fn nothing_is_synthesised_while_nobody_is_listening() {
        let published = Published::new();
        let mut host = host(Arc::clone(&published));

        for _ in 0..3 {
            host.tick();
        }
        assert!(
            !host.gb.core().mmu().audio().output_enabled(),
            "the APU was still mixing and resampling with nobody attached",
        );

        // A listener arrives and it opens again — on the same edge that restarts the encoder.
        let _listener = published.join_audio();
        host.tick();
        assert!(
            host.gb.core().mmu().audio().output_enabled(),
            "a listener attached and the APU never started synthesising again",
        );
    }

    /// What reaches a listener is Opus that a real decoder turns back into sound.
    #[test]
    fn the_host_publishes_decodable_audio() {
        let published = Published::new();
        let mut listener = published.join_audio();
        let mut host = host(Arc::clone(&published));

        let mut packets = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while packets.len() < 25 && Instant::now() < deadline {
            host.tick();
            while let Ok(packet) = listener.try_recv() {
                packets.push(packet);
            }
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(packets.len() >= 25, "only {} packets in 20 s", packets.len());

        let mut decoder =
            opus_rs::OpusDecoder::new(audio::SAMPLE_RATE as i32, audio::CHANNELS as usize)
                .expect("decoder");
        let mut frame = vec![0.0f32; audio::FRAME_SAMPLES];
        let mut loudest = 0.0f32;
        for packet in &packets {
            let samples = decoder.decode(packet, audio::FRAME_SAMPLES, &mut frame).expect("decode");
            assert_eq!(samples, audio::FRAME_SAMPLES, "a packet decoded to the wrong length");
            loudest = loudest.max(frame.iter().fold(0.0f32, |a, s| a.max(s.abs())));
        }
        assert!(loudest > 0.01, "half a second of the Pallet Town theme came back silent");
    }

    #[test]
    fn the_sample_rate_and_the_speed_survive_every_state_that_is_loaded() {
        use poke_agent::run::{Origin, RunDir};

        let scratch = poke_agent::run::Scratch::new("host-audio-tuning");
        let validate = |bytes: &[u8]| GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, origin, _) = RunDir::open(&scratch.0, false, "random", &validate).expect("a fresh run");
        assert_eq!(origin, Origin::Fresh);

        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let mut host = host_with(Arc::clone(&published), |config| {
            config.run = Some(Arc::clone(&current));
        });
        let speed = host.config.target_speed;

        let tuned = |host: &EmulatorHost| {
            let audio = host.gb.core().mmu().audio();
            (audio.output_sample_rate(), audio.emulation_speed())
        };
        assert_eq!(tuned(&host), (audio::SAMPLE_RATE, speed), "EmulatorHost::new left the APU untuned");

        host.start_new_run().expect("a new run");
        assert_eq!(tuned(&host), (audio::SAMPLE_RATE, speed), "start_new_run left the APU untuned");
    }

    /// The deployment's normal state: nobody is listening, so nothing is encoded at all.
    #[test]
    fn nothing_is_encoded_while_nobody_is_listening() {
        let published = Published::new();
        let mut host = host(Arc::clone(&published));

        for _ in 0..200 {
            host.tick();
            std::thread::sleep(Duration::from_micros(500));
        }
        assert_eq!(host.audio.as_ref().expect("audio on").packets(), 0, "encoded with no listener");

        let _listener = published.join_audio();
        let deadline = Instant::now() + Duration::from_secs(20);
        while host.audio.as_ref().expect("audio on").packets() == 0 && Instant::now() < deadline {
            host.tick();
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(host.audio.as_ref().expect("audio on").packets() > 0, "a listener got nothing");
    }

    #[test]
    fn audio_off_builds_no_encoder_and_drains_nothing() {
        let published = Published::new();
        let mut host = host_with(Arc::clone(&published), |config| config.audio_bitrate = None);
        let mut listener = published.join_audio();

        for _ in 0..200 {
            host.tick();
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(host.audio.is_none());
        assert!(listener.try_recv().is_err(), "audio was published with the encoder off");
    }

    #[test]
    fn a_checkpointed_run_resumes_where_it_stopped() {
        use poke_agent::pokemon::PokemonApiTrait;
        use poke_agent::run::{Origin, RunDir};

        let scratch = poke_agent::run::Scratch::new("host-resume");
        let validate = |bytes: &[u8]| GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok();
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
        // …then the shutdown checkpoint, so the comparison below is exact.
        host.checkpoint();
        let before = {
            let api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
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
            let api = PokemonApi::with_cache(&mut second.gb, &mut second.map_cache);
            api.game_state().expect("a readable state")
        };

        assert_eq!(after.map.map, before.map.map);
        assert_eq!(after.map.player_position, before.map.player_position,
                   "the second host started somewhere else — that is a resume that did nothing");
        // …and the SRAM was written beside it, for anything that reads an ordinary .sav.
        let sram = std::fs::read(run.path().join("sram.bin")).expect("sram.bin");
        assert!(!sram.is_empty() && sram.len() % 1024 == 0, "{} bytes is not a bank count", sram.len());
    }

    /// `POST /api/new-run`'s acceptance, at the seam it actually crosses.
    #[test]
    fn a_new_run_starts_in_place_and_leaves_the_old_one_complete() {
        use poke_agent::pokemon::PokemonApiTrait;
        use poke_agent::run::{Origin, RunDir};

        let scratch = poke_agent::run::Scratch::new("host-new-run");
        let validate = |bytes: &[u8]| GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, origin, _) = RunDir::open(&scratch.0, false, "random", &validate).expect("a fresh run");
        assert_eq!(origin, Origin::Fresh);

        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let control = Arc::new(ControlRequests::default());
        let first = current.get();
        let mut host = host_with(Arc::clone(&published), |config| {
            config.run = Some(Arc::clone(&current));
            config.control = Some(Arc::clone(&control));
            // Longer than the test, so only the swap can write the state file below.
            config.checkpoint_interval = Duration::from_secs(3600);
        });

        // Play far enough that the agent has left the start state behind.
        let deadline = Instant::now() + Duration::from_secs(20);
        while host.emulated.to_duration() < Duration::from_secs(8) && Instant::now() < deadline {
            host.tick();
            std::thread::sleep(Duration::from_micros(500));
        }
        let played = {
            let api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
            api.game_state().expect("a readable state")
        };
        assert!(host.emulated.to_duration() >= Duration::from_secs(8), "the host barely ran");
        assert!(!first.path().join("state.gbst").exists(), "a periodic checkpoint fired after all");

        // Ask exactly as the handler does, then give the emulator the one tick it needs.
        let receiver = control.request(ControlRequest::NewRun).expect("the mailbox is empty");
        host.tick();
        let run_id = receiver.blocking_recv().expect("the emulator answered").expect("a new run");

        let second = current.get();
        assert_eq!(second.run_id(), run_id);
        assert_ne!(second.run_id(), first.run_id(), "the run directory did not change");
        assert!(first.path().join("state.gbst").is_file(),
                "the outgoing run was not checkpointed — everything since its last write is gone");

        let restarted = {
            let api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
            api.game_state().expect("a readable state")
        };
        let start = {
            let mut fresh = host_with(Published::new(), |_| {});
            let api = PokemonApi::with_cache(&mut fresh.gb, &mut fresh.map_cache);
            api.game_state().expect("a readable state")
        };
        assert_eq!(restarted.map.player_position, start.map.player_position,
                   "the emulator kept playing the old game — it was {:?} before the reset",
                   played.map.player_position);
        // Near zero, not zero: the swap zeroes the counter and the rest of that tick emulates.
        assert!(host.emulated.to_duration() < Duration::from_secs(1),
                "emulated time measures *this* game, but reads {:?}", host.emulated.to_duration());

        // And the next frame is a keyframe, or a viewer keeps fragments of the abandoned run.
        host.publish_video();
        assert!(published.latest_keyframe().is_some_and(|frame| frame.keyframe),
                "the video encoder was not restarted");
    }

    /// The win is noticed, the run filed and the next one started, with the transcript thread live.
    #[test]
    #[cfg_attr(not(feature = "slow-tests"), ignore = "drives a cutscene; run with --features slow-tests")]
    fn a_finished_run_is_filed_and_the_next_one_starts() {
        use poke_agent::run::{RunDir, files, hall_of_fame};

        let scratch = poke_agent::run::Scratch::new("host-hall-of-fame");
        let validate = |bytes: &[u8]| GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, _, _) = RunDir::open(&scratch.0, false, "gpt-test", &validate).expect("a fresh run");

        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "gpt-test".to_string(), run));
        let finished = current.get();
        let finished_id = finished.run_id();
        // The archive follows the file the transcript thread writes: the path's ordering hazard.
        let stop = Arc::new(AtomicBool::new(false));
        let transcript = poke_agent::run::transcript::spawn(
            Arc::clone(&current),
            Arc::clone(&published),
            Arc::clone(&stop),
        )
        .expect("a transcript writer");

        let mut host = host_from(
            include_bytes!("../../poke-agent/src/pokemon/data/post-hall-of-fame.bin"),
            Arc::clone(&published),
            |config| {
                config.run = Some(Arc::clone(&current));
                // Longer than the test, so only the completion path can write the files below.
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
        assert_eq!(row.model, None, "only an LLM run names a model");
        assert_eq!(row.app_version, crate::cli::VERSION);
        assert_eq!(row.badges, 8, "the winning tally is read at the moment of victory");
        assert!(row.playtime_seconds > 0 && !row.playtime_maxed);

        let archive = scratch.0.join(files::HALL_OF_FAME).join(&row.archive);
        assert!(archive.join(files::STATE).is_file(), "the save state at the moment of victory");
        assert!(archive.join(files::SRAM).is_file());
        assert!(archive.join(files::META).is_file());
        assert!(archive.join("transcript.jsonl.gz").is_file(),
                "the run's own story is the point of keeping the directory at all");

        // The victory is *in* the archived transcript.
        let gz = std::fs::read(archive.join("transcript.jsonl.gz")).expect("the transcript");
        let mut story = String::new();
        std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&gz[..]), &mut story)
            .expect("it inflates");
        assert!(story.contains("hall_of_fame"),
                "the archived transcript stops short of the event it is a record of");

        // The outgoing directory is complete and stamped, so a resume never files it twice.
        assert!(finished.path().join(files::STATE).is_file(), "the outgoing run was checkpointed");
        assert_eq!(finished.meta().completed.len(), 1);
        assert!(finished.already_archived(1));

        // The archive is invisible to the resume scan, or the next start would continue a won game.
        let (resumed, _, _) = RunDir::open(&scratch.0, false, "gpt-test", &validate).expect("a resume");
        assert_ne!(resumed.run_id(), row.archive, "hall-of-fame/ must not be resumable");

        // The writer is a `blocking_recv` loop, so it only notices `stop` when something wakes it.
        published.publish_event(UiEventBody::Notice { level: "info", message: "done".into() });
        let _ = transcript.join();
    }

    /// `POST /api/clear`'s half of the same seam.
    #[test]
    fn a_clear_leaves_the_run_and_the_game_exactly_where_they_were() {
        use poke_agent::pokemon::PokemonApiTrait;
        use poke_agent::run::{Origin, RunDir};

        let scratch = poke_agent::run::Scratch::new("host-clear");
        let validate = |bytes: &[u8]| GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok();
        let (run, origin, _) = RunDir::open(&scratch.0, false, "random", &validate).expect("a fresh run");
        assert_eq!(origin, Origin::Fresh);

        let published = Published::new();
        let current = Arc::new(CurrentRun::new(scratch.0.clone(), "random".to_string(), run));
        let control = Arc::new(ControlRequests::default());
        let before = current.get();
        let mut host = host_with(Arc::clone(&published), |config| {
            config.run = Some(Arc::clone(&current));
            config.control = Some(Arc::clone(&control));
            config.checkpoint_interval = Duration::from_secs(3600);
        });

        let deadline = Instant::now() + Duration::from_secs(20);
        while host.emulated.to_duration() < Duration::from_secs(8) && Instant::now() < deadline {
            host.tick();
            std::thread::sleep(Duration::from_micros(500));
        }
        assert!(host.emulated.to_duration() >= Duration::from_secs(8), "the host barely ran");
        let played = {
            let api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
            api.game_state().expect("a readable state")
        };

        let receiver = control.request(ControlRequest::ClearConversation).expect("the mailbox is empty");
        host.tick();
        let answer = receiver.blocking_recv().expect("the emulator answered");
        let refusal = answer.expect_err("a run nothing is thinking about has no conversation to clear");
        assert!(refusal.contains("not being played by a model"), "{refusal}");

        assert_eq!(current.get().run_id(), before.run_id(), "a clear must not swap the run directory");
        assert!(!before.path().join("state.gbst").exists(), "a clear must not checkpoint, let alone reset");
        let after = {
            let api = PokemonApi::with_cache(&mut host.gb, &mut host.map_cache);
            api.game_state().expect("a readable state")
        };
        // The trainer ID, not the map: a restart mints a new one, and the walk may take the stairs.
        assert_eq!(after.player_id, played.player_id, "the game was restarted rather than left alone");
        // …and the emulator is still running, on the same clock rather than one zeroed by a swap.
        let emulated = host.emulated;
        host.tick();
        assert!(host.emulated >= emulated, "the clock went backwards, so something reset it");
    }

    /// A second request is refused only while someone still waits on the first.
    #[test]
    fn the_control_mailbox_refuses_a_concurrent_request_but_not_an_abandoned_one() {
        let mailbox = ControlRequests::default();

        let receiver = mailbox.request(ControlRequest::NewRun).expect("the first is accepted");
        assert!(mailbox.request(ControlRequest::NewRun).is_err(), "a second while the first is outstanding");
        let refusal = mailbox.request(ControlRequest::ClearConversation).expect_err("nor a clear");
        assert!(refusal.contains("new run"), "the refusal names the wrong command: {refusal}");

        drop(receiver);
        // Bound: a `Receiver` dropped on the spot would itself be an abandoned request.
        let receiver = mailbox
            .request(ControlRequest::ClearConversation)
            .expect("an abandoned request must not block the next one");
        // …and the other way round, so neither command is the privileged one.
        let refusal = mailbox.request(ControlRequest::NewRun).expect_err("the clear is outstanding");
        assert!(refusal.contains("conversation"), "the refusal names the wrong command: {refusal}");
        drop(receiver);
    }
}
