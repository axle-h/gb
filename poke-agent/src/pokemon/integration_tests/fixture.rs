//! The shared harness: a snapshot restored into a `GameBoy` driven by a `PokemonAgent`, with stall
//! and budget guards that fail a wedged agent fast, with a screenshot.

use super::*;

/// Why a run stopped early.
struct RunFailure(String);

impl RunFailure {
    /// Which `target/test-artifacts/` pair to write.
    fn artifact_name(&self) -> &'static str {
        if self.0.starts_with("policy stalled") { "test_stall" } else { "test_timeout" }
    }
}

pub struct TestFixture {
    pub gb: GameBoy,
    map_cache: MapMetadataCache,
    pub agent: PokemonAgent,
    pub total_cycles: MachineCycles,
    pub max_cycles: MachineCycles,
    /// Cycles since the policy queue length last changed.
    stall_cycles: MachineCycles,
    last_steps_remaining: Option<usize>,
    stall_threshold: MachineCycles,
    /// False until the first [`Self::step`], so the load-time write is not reported as a drift.
    options_reapplied: bool,
    /// Times the game restored its own `wOptions` over the harness's, one per save/reload.
    pub options_drifts: u32,
    /// The options this fixture holds the game to.
    options: GameOptions,
    /// How many policy steps the run was handed, so a failure can report how far it got.
    steps_at_start: Option<usize>,
    pub coverage: Option<super::coverage::CoverageLog>,
}

/// Whether this run may overwrite the fixtures it snapshots; off, so a run leaves the tree clean.
pub fn regenerating_fixtures() -> bool {
    std::env::var_os("GB_REGEN_FIXTURES").is_some_and(|v| v != "0" && v != "")
}

impl TestFixture {
    pub fn new(save_state: &[u8], max_game_time: Duration, policy_steps: Vec<PolicyStep>) -> Self {
        Self::with_policy(save_state, max_game_time, Box::new(DeterministicPolicy::new(42, policy_steps)))
    }

    /// [`Self::new`] with the policy supplied rather than built.
    pub fn with_policy(save_state: &[u8], max_game_time: Duration, policy: Box<dyn crate::pokemon::policy::Policy>) -> Self {
        // `GB_TEST_MODEL=cgb` runs the fixture on a Game Boy Color, in compatibility mode.
        let cgb = matches!(std::env::var("GB_TEST_MODEL").as_deref(), Ok("cgb"));
        let mut gb = if cgb { GameBoy::cgb(roms::POKERED) } else { GameBoy::dmg(roms::POKERED) };
        gb.load_state(save_state).expect("failed to load save state");
        if cgb {
            // Fixture sections come from a DMG machine, so check the load kept the switch.
            assert_eq!(gb.core().mmu().color_mode(), gb::model::ColorMode::CgbCompat,
                "GB_TEST_MODEL=cgb, so the loaded state must still be a CGB in compatibility mode");
        }

        let steps_at_start = policy.steps_remaining();

        let options = crate::pokemon::options::HEADLESS_OPTIONS;
        PokemonApi::new(&mut gb).debug_set_options(&options);

        // Nothing in this harness ever listens, so the APU does not mix or resample.
        gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);

        Self {
            options,
            steps_at_start,
            gb,
            map_cache: MapMetadataCache::default(),
            total_cycles: MachineCycles::ZERO,
            max_cycles: MachineCycles::from_duration(max_game_time),
            stall_cycles: MachineCycles::ZERO,
            last_steps_remaining: None,
            stall_threshold: MachineCycles::from_duration(Duration::from_secs(10 * 60)),
            options_reapplied: false,
            options_drifts: 0,
            coverage: None,
            agent: PokemonAgent::new(policy),
        }
    }

    pub fn with_coverage(mut self) -> Self {
        self.coverage = Some(super::coverage::CoverageLog::new());
        self
    }

    pub fn with_stall_tolerance(mut self, tolerance: Duration) -> Self {
        self.stall_threshold = MachineCycles::from_duration(tolerance);
        self
    }

    /// Hold this fixture to battle animations on.
    pub fn with_original_battle_timing(self) -> Self {
        self.with_options(crate::pokemon::options::SERVED_OPTIONS)
    }

    /// Hold this fixture to `options` rather than [`crate::pokemon::options::HEADLESS_OPTIONS`].
    pub fn with_options(mut self, options: GameOptions) -> Self {
        self.options = options;
        PokemonApi::with_cache(&mut self.gb, &mut self.map_cache).debug_set_options(&self.options);
        self
    }

    /// Advance one agent tick, failing the test if the run stalls or exhausts its budget.
    pub fn step(&mut self) {
        if let Err(failure) = self.try_step() {
            self.save_failure_artifacts(failure.artifact_name());
            panic!("{}", failure.0);
        }
    }

    /// [`Self::step`] without the panic.
    fn try_step(&mut self) -> Result<(), RunFailure> {
        let cycles = self.gb.run(AGENT_RESOLUTION);

        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        // Re-applied every tick, because a save/reload restores the cartridge's own options.
        if api.debug_set_options(&self.options) && self.options_reapplied {
            self.options_drifts += 1;
            println!("[fixture] wOptions drifted (a save/reload restored the cartridge's) — re-applied");
        }
        self.options_reapplied = true;
        self.agent.update(&mut api, cycles).ok();
        self.observe_coverage();

        self.total_cycles += cycles;

        // GrindUntilLevel and CatchPokemon sit on one step for long stretches.
        const BATTLE_STALL_FACTOR: u64 = 8;
        let steps = self.agent.policy_steps_remaining();
        let long_running = self.agent.policy_current_step_is_long_running();
        let threshold = if self.agent.in_battle() {
            self.stall_threshold * BATTLE_STALL_FACTOR
        } else {
            self.stall_threshold
        };
        if steps != self.last_steps_remaining {
            self.last_steps_remaining = steps;
            self.stall_cycles = MachineCycles::ZERO;
        } else if !long_running && steps.map_or(false, |n| n > 1) {
            self.stall_cycles += cycles;
            if self.stall_cycles >= threshold {
                return Err(RunFailure(format!(
                    "policy stalled — queue unchanged for {:?} of game time{}",
                    threshold, self.progress_note())));
            }
        }

        if self.total_cycles >= self.max_cycles {
            return Err(RunFailure(format!(
                "exceeded max cycles ({:?} game time){}", self.max_cycles, self.progress_note())));
        }
        Ok(())
    }

    /// How far the run got, appended to every failure panic.
    fn progress_note(&self) -> String {
        match (self.steps_at_start, self.agent.policy_steps_remaining()) {
            (Some(total), Some(left)) if total > 0 => format!(
                " — completed {}/{} policy steps ({}%); resume with RESUME_QUEUE_LEN={left}",
                total - left, total, (total - left) * 100 / total),
            _ => String::new(),
        }
    }

    /// Fold this tick's events into the coverage log, saving a state wherever one was a defect.
    fn observe_coverage(&mut self) {
        let Some(log) = self.coverage.as_mut() else { return };
        for event in self.agent.drain_events() {
            log.observe(&event);
        }
        let defects = log.take_new_defects();
        for id in defects {
            let name: String = id
                .chars()
                .map(|c| match c {
                    ':' | ',' | '/' | ' ' => '-',
                    other => other,
                })
                .collect();
            self.save_failure_artifacts(&format!("coverage/defect-{name}"));
        }
    }

    /// Under `target/`, so a failing run leaves no untracked files beside the fixtures.
    fn save_failure_artifacts(&self, name: &str) {
        let dir = std::path::Path::new("target/test-artifacts");
        let state = dir.join(format!("{name}_state.bin"));
        let shot = dir.join(format!("{name}_screenshot.png"));
        for path in [&state, &shot] {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        self.gb.save_state_to_file(&state.to_string_lossy()).ok();
        self.gb.save_screenshot_to_file(&shot.to_string_lossy()).ok();
        println!("saved failure artifacts: {}, {}", state.display(), shot.display());
    }

    /// One iteration of a host loop `min_cycles` behind the clock, rather than one agent tick.
    pub fn step_coarse(&mut self, min_cycles: MachineCycles) {
        PokemonApi::with_cache(&mut self.gb, &mut self.map_cache).debug_set_options(&self.options);
        let (ran, _) = self.agent.run(&mut self.gb, &mut self.map_cache, min_cycles);
        self.observe_coverage();
        self.total_cycles += ran;
        assert!(self.total_cycles < self.max_cycles,
            "exceeded max cycles ({:?} game time){}", self.max_cycles, self.progress_note());
    }

    pub fn step_until_exhausted(&mut self) {
        while !self.agent.policy_exhausted() {
            self.step();
        }
    }

    /// Drive until `done` is satisfied, tracing each map the run passes through.
    pub fn run_until(&mut self, done: impl Fn(&GameState) -> bool) -> GameState {
        let mut last_map = None;
        loop {
            let state = {
                let api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
                api.game_state()
            };
            if let Ok(state) = state {
                // Map changes only; a position trace would be a line per tile.
                if last_map != Some(state.map.map) {
                    last_map = Some(state.map.map);
                    println!("  → {} @ {}", state.map.map, state.map.player_position);
                }
                if done(&state) { return state; }
            }
            self.step();
        }
    }

    /// [`Self::run_until`] without the panic: `None` is a stall or a spent budget, already printed.
    pub fn try_run_until(&mut self, done: impl Fn(&GameState) -> bool) -> Option<GameState> {
        let mut last_map = None;
        loop {
            let state = {
                let api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
                api.game_state()
            };
            if let Ok(state) = state {
                if last_map != Some(state.map.map) {
                    last_map = Some(state.map.map);
                    println!("  → {} @ {}", state.map.map, state.map.player_position);
                }
                if done(&state) {
                    return Some(state);
                }
            }
            if let Err(failure) = self.try_step() {
                println!("  ✗ gave up: {}", failure.0);
                return None;
            }
        }
    }

    /// Drive the queued policy to exhaustion, then keep going until `done` is satisfied.
    pub fn run_leg(&mut self, done: impl Fn(&GameState) -> bool) -> GameState {
        self.step_until_exhausted();
        let before = self.total_cycles;
        let state = self.run_until(done);
        let slack = (self.total_cycles - before).to_duration();
        if slack > Duration::from_secs(5) {
            println!("[fixture] run_leg waited {slack:?} of game time AFTER the queue emptied — \
                      the step list does not actually finish this leg, and `complete_game_steps` \
                      will not wait. See the run_leg doc comment.");
        }
        state
    }

    pub fn pimp_pokemon(&mut self) {
        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        api.pimp_out_pokemon().expect("cannot pimp pokemon");
    }

    pub fn api(&mut self) -> PokemonApi<'_> {
        PokemonApi::with_cache(&mut self.gb, &mut self.map_cache)
    }

    pub fn game_state(&mut self) -> GameState {
        self.api().game_state().unwrap()
    }

    /// As [`Self::game_state`], for fixtures cut mid-transition or in battle.
    pub fn try_game_state(&mut self) -> Result<GameState, String> {
        self.api().game_state()
    }

    /// Rewrite a committed fixture — only under `GB_REGEN_FIXTURES=1`.
    pub fn save_state_named(&mut self, path: &str) -> Result<(), String> {
        if regenerating_fixtures() {
            println!("regenerating fixture {path}");
            self.gb.save_state_to_file(path)
        } else {
            println!("skipping fixture write to {path} (set GB_REGEN_FIXTURES=1)");
            Ok(())
        }
    }
}

/// Diagnostic: print each committed fixture's map, badges, money, party and bag.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "diagnostic, not a test; run with --ignored --nocapture"]
fn dump_fixture_states() {
    // Every fixture some leg reads, in chain order.
    const FIXTURES: &[(&str, &[u8])] = &[
        ("at-cerulean", include_bytes!("../data/at-cerulean.bin")),
        ("at-vermilion", include_bytes!("../data/at-vermilion.bin")),
        ("post-ss-anne", include_bytes!("../data/post-ss-anne.bin")),
        ("post-teach-cut", include_bytes!("../data/post-teach-cut.bin")),
        ("post-thunder-badge", include_bytes!("../data/post-thunder-badge.bin")),
        ("back-in-cerulean", include_bytes!("../data/back-in-cerulean.bin")),
        ("at-lavender", include_bytes!("../data/at-lavender.bin")),
        ("at-celadon", include_bytes!("../data/at-celadon.bin")),
        ("at-rocket-hideout", include_bytes!("../data/at-rocket-hideout.bin")),
        ("post-silph-scope", include_bytes!("../data/post-silph-scope.bin")),
        ("post-poke-flute", include_bytes!("../data/post-poke-flute.bin")),
        ("post-snorlax", include_bytes!("../data/post-snorlax.bin")),
        ("post-soul-badge", include_bytes!("../data/post-soul-badge.bin")),
        ("post-safari-surf", include_bytes!("../data/post-safari-surf.bin")),
        ("post-safari", include_bytes!("../data/post-safari.bin")),
        ("at-saffron", include_bytes!("../data/at-saffron.bin")),
        ("silph-card-key", include_bytes!("../data/silph-card-key.bin")),
        ("post-silph-giovanni", include_bytes!("../data/post-silph-giovanni.bin")),
        ("post-marsh-badge", include_bytes!("../data/post-marsh-badge.bin")),
        ("at-cinnabar", include_bytes!("../data/at-cinnabar.bin")),
        ("post-secret-key", include_bytes!("../data/post-secret-key.bin")),
        ("post-volcano-badge", include_bytes!("../data/post-volcano-badge.bin")),
        ("seafoam-b3f", include_bytes!("../data/seafoam-b3f.bin")),
        ("post-articuno", include_bytes!("../data/post-articuno.bin")),
        ("post-earth-badge", include_bytes!("../data/post-earth-badge.bin")),
        ("vr1f-strength", include_bytes!("../data/vr1f-strength.bin")),
        ("vr3f-strength", include_bytes!("../data/vr3f-strength.bin")),
        ("vr2f-ladder", include_bytes!("../data/vr2f-ladder.bin")),
        ("at-indigo-articuno", include_bytes!("../data/at-indigo-articuno.bin")),
        ("post-champion", include_bytes!("../data/post-champion.bin")),
    ];
    for (name, bytes) in FIXTURES {
        let mut fixture = TestFixture::new(bytes, Duration::from_mins(1), vec![]);
        let s = fixture.game_state();
        println!("== {name}: {} @ {} | badges {:?} | ¥{}", s.map.map, s.map.player_position, s.badges, s.money);
        for (i, p) in s.pokemon.iter().enumerate() {
            let moves: Vec<String> = p.moves.iter().flatten().map(|m| format!("{:?}(pp{})", m.name, m.pp)).collect();
            println!("   slot{i}: {:?} lv{} {}/{}hp — {}", p.species, p.level, p.current_hp, p.stats.hp, moves.join(", "));
        }
        let bag: Vec<String> = s.bag.iter().map(|it| format!("{:?}x{}", it.id, it.quantity)).collect();
        println!("   bag[{}/20]: {}", s.bag.iter().count(), bag.join(", "));
    }
}

/// Micro-benchmark: raw emulation throughput against the full agent step, from a mid-game fixture.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "benchmark, not a test; run with --ignored --nocapture"]
fn bench_emulation_throughput() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-celadon.bin"),
        Duration::from_mins(60),
        PolicyStep::celadon_rainbow_steps(),
    );
    // (a) Raw emulation only.
    {
        let game_secs = 30.0;
        let target = MachineCycles::from_duration(Duration::from_secs_f64(game_secs));
        let start = std::time::Instant::now();
        let mut emulated = MachineCycles::ZERO;
        while emulated < target {
            emulated += fixture.gb.run(AGENT_RESOLUTION);
        }
        let wall = start.elapsed().as_secs_f64();
        println!("[raw run only]     {game_secs}s game in {wall:.3}s → {:.1}x realtime", game_secs / wall);
    }
    // (b) Full agent step: observe, policy, input synthesis.
    {
        let n = 3000u32;
        let before = fixture.total_cycles;
        let start = std::time::Instant::now();
        for _ in 0..n { fixture.step(); }
        let wall = start.elapsed().as_secs_f64();
        let game_secs = (fixture.total_cycles.m_cycles() - before.m_cycles()) as f64 / 1_048_576.0;
        println!("[full agent.step]  {game_secs:.1}s game in {wall:.3}s → {:.1}x realtime ({} steps)",
            game_secs / wall, n);
    }
}
