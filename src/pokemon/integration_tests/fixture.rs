//! The shared harness every test in this module drives: a `GameBoy` restored from a snapshot, wired
//! to a `PokemonAgent` running a `DeterministicPolicy`, plus the stall/budget guards that turn a wedged
//! agent into a fast failure with a screenshot instead of a test that runs to the cycle cap.

use super::*;

pub struct TestFixture {
    pub gb: GameBoy,
    map_cache: MapMetadataCache,
    pub agent: PokemonAgent,
    pub total_cycles: MachineCycles,
    pub max_cycles: MachineCycles,
    /// Cycles since the policy queue length last changed (stall detection).
    stall_cycles: MachineCycles,
    last_steps_remaining: Option<usize>,
    /// How long without queue progress before we declare a stall.
    stall_threshold: MachineCycles,
}

impl TestFixture {
    pub fn new(save_state: &[u8], max_game_time: Duration, policy_steps: Vec<PolicyStep>) -> Self {
        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(save_state).expect("failed to load save state");

        // The agent builds its world graph incrementally as it traverses.
        let policy = DeterministicPolicy::new(42, policy_steps);

        PokemonApi::new(&mut gb)
            .write_game_options(&GameOptions::default())
            .expect("failed to write game options");

        Self {
            gb,
            map_cache: MapMetadataCache::default(),
            total_cycles: MachineCycles::ZERO,
            max_cycles: MachineCycles::from_duration(max_game_time),
            stall_cycles: MachineCycles::ZERO,
            last_steps_remaining: None,
            // 10 minutes of game time without a queue step change → stall
            stall_threshold: MachineCycles::from_duration(Duration::from_secs(10 * 60)),
            agent: PokemonAgent::new(Box::new(policy)),
        }
    }

    pub fn step(&mut self) {
        let cycles = self.gb.run(AGENT_RESOLUTION);

        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        self.agent.update(&mut api, cycles).ok();

        self.total_cycles += cycles;

        // Stall detection: GrindUntilLevel and CatchPokemon legitimately sit on the
        // same step for long stretches — exempt them regardless of queue length. A battle gets a
        // *longer leash* rather than an exemption: the queue cannot advance mid-fight, so a six-mon
        // rival or an Elite Four room needs far more than the usual threshold — but a fight can also
        // deadlock outright (an attacker out of PP, healing itself forever against a mon it cannot KO),
        // and that has to keep failing fast instead of running to the cycle cap.
        const BATTLE_STALL_FACTOR: usize = 8;
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
                self.save_failure_artifacts("test_stall");
                panic!("policy stalled — queue unchanged for {:?} of game time", threshold);
            }
        }

        if self.total_cycles >= self.max_cycles {
            self.save_failure_artifacts("test_timeout");
            panic!("exceeded max cycles ({:?} game time)", self.max_cycles);
        }
    }

    /// Dropped under `target/` rather than the repo root: a failing run must not leave untracked
    /// junk in the working tree next to the fixtures it is being compared against.
    fn save_failure_artifacts(&self, name: &str) {
        let dir = std::path::Path::new("target/test-artifacts");
        std::fs::create_dir_all(dir).ok();
        let state = dir.join(format!("{name}_state.bin"));
        let shot = dir.join(format!("{name}_screenshot.png"));
        self.gb.save_state_to_file(&state.to_string_lossy()).ok();
        self.gb.save_screenshot_to_file(&shot.to_string_lossy()).ok();
        println!("saved failure artifacts: {}, {}", state.display(), shot.display());
    }

    pub fn step_until_exhausted(&mut self) {
        while !self.agent.policy_exhausted() {
            self.step();
        }
    }

    /// Drive until `done` is satisfied, tracing each map the run passes through.
    ///
    /// There is deliberately no step cap: the cycle budget in [`Self::step`] is the failsafe, and it
    /// fails with a screenshot and a saved state instead of silently falling out of a `for` loop with
    /// an assertion that then reports the wrong thing.
    ///
    /// Use this for steps that never pop on their own — `DefeatGymLeader` runs until the badge is in
    /// hand, so waiting for the queue to empty would wait forever.
    pub fn run_until(&mut self, done: impl Fn(&GameState) -> bool) -> GameState {
        let mut last_map = None;
        loop {
            let state = {
                let api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
                api.game_state()
            };
            if let Ok(state) = state {
                // Map changes only — a position trace would be one line per tile walked.
                if last_map != Some(state.map.map) {
                    last_map = Some(state.map.map);
                    println!("  → {} @ {}", state.map.map, state.map.player_position);
                }
                if done(&state) { return state; }
            }
            self.step();
        }
    }

    /// Drive the queued policy to exhaustion, then keep going until `done` is satisfied.
    ///
    /// `Interact` and `CollectItem` pop the moment they *issue* the walk, so the queue routinely
    /// empties while the effect it was queued for — an item landing in the bag, an NPC's text box —
    /// is still in flight. This is the "exhaust, then wait for the effect" idiom.
    pub fn run_leg(&mut self, done: impl Fn(&GameState) -> bool) -> GameState {
        self.step_until_exhausted();
        self.run_until(done)
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

    /// Rewrite a committed fixture — **only** with `--features regen-fixtures`.
    ///
    /// Every leg test snapshots its end state for the next leg to start from, which is how the chain
    /// is maintained. Doing that on an ordinary run means each run silently changes the next run's
    /// inputs, so a leg can "fail" purely because an earlier one re-saved its fixture slightly
    /// differently. Off by default; the call sites stay as documentation of the chain.
    pub fn save_state_named(&mut self, path: &str) -> Result<(), String> {
        if cfg!(feature = "regen-fixtures") {
            println!("regenerating fixture {path}");
            self.gb.save_state_to_file(path)
        } else {
            println!("skipping fixture write to {path} (enable --features regen-fixtures)");
            Ok(())
        }
    }
}

/// Diagnostic, not a test: print where each committed fixture stands — map, badges, money, party and
/// bag. When a leg test fails, this is the first thing to look at, because the usual cause is that its
/// input snapshot no longer matches what the leg's `PolicyStep`s assume (a party member in a different
/// slot, a missing HM, an empty wallet). Run with
/// `cargo test --release --bin gb -- dump_fixture_states --exact --ignored --nocapture`.
#[test]
#[ignore = "diagnostic, not a test; run with --ignored --nocapture"]
fn dump_fixture_states() {
    // Every fixture some leg reads, in chain order.
    const FIXTURES: &[(&str, &[u8])] = &[
        ("post-cascade", include_bytes!("../data/post-cascade.bin")),
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
        ("post-volcano-lone", include_bytes!("../data/post-volcano-lone.bin")),
        ("at-mansion-blizzard", include_bytes!("../data/at-mansion-blizzard.bin")),
        ("post-earth-badge", include_bytes!("../data/post-earth-badge.bin")),
        ("vr1f-strength", include_bytes!("../data/vr1f-strength.bin")),
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

/// Coverage probe — task 0.1 of `docs/postgame-coverage-plan.md`, and the first thing every postgame
/// workstream runs against its entry fixture (§4.4).
///
/// Deliberately more than [`dump_fixture_states`] prints: the postgame plan turns on numbers that
/// test aren't in it — **dex owned/seen counts** (the Oak's-aide gates are 10/30/50 owned),
/// `wBoxCount`/`wCurrentBoxNum` (the storage system), `wPlayerCoins` (the Game Corner economy), and a
/// **raw** bag read.
///
/// The bag is read from `wNumBagItems`/`wBagItems` rather than `GameState::bag`, because the latter
/// silently drops every id [`ItemId`] cannot name — most of the TMs — so it under-reports occupancy
/// against the 20-slot ceiling that is the whole reason Phase 0 exists. Unnamed ids print as `$xx`.
///
/// Run with
/// `cargo test --release --bin gb -- probe_coverage --exact --ignored --nocapture`.
#[test]
#[ignore = "diagnostic, not a test; run with --ignored --nocapture"]
fn probe_coverage() {
    // Postgame entry fixtures. Append each `postgame-*.bin` here as its workstream commits one.
    const FIXTURES: &[(&str, &[u8])] = &[
        ("post-hall-of-fame", include_bytes!("../data/post-hall-of-fame.bin")),
        ("postgame-post-credits", include_bytes!("../data/postgame-post-credits.bin")),
        ("postgame-phase0", include_bytes!("../data/postgame-phase0.bin")),
        ("postgame-pc-box", include_bytes!("../data/postgame-pc-box.bin")),
        // Workstream B's chain: voucher → Bicycle → HM02 → Fly proven → Cycling Road.
        ("postgame-bike-voucher", include_bytes!("../data/postgame-bike-voucher.bin")),
        ("postgame-bicycle", include_bytes!("../data/postgame-bicycle.bin")),
        ("postgame-hm02", include_bytes!("../data/postgame-hm02.bin")),
        ("postgame-fly", include_bytes!("../data/postgame-fly.bin")),
        ("postgame-fly-bike", include_bytes!("../data/postgame-fly-bike.bin")),
    ];
    for (name, bytes) in FIXTURES {
        print_coverage(name, bytes);
    }
}

/// The body of [`probe_coverage`] for one snapshot. Separate so a workstream can call it on a state
/// it has just driven to, not only on a committed file.
pub fn print_coverage(name: &str, save_state: &[u8]) {
    use crate::pokemon::bag::Bag;
    use crate::pokemon::item::ItemId;
    use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};

    let mut fixture = TestFixture::new(save_state, Duration::from_mins(1), vec![]);
    let state = fixture.game_state();
    let api = fixture.api();
    let mmu = api.mmu();

    let badge_bits = mmu.read_pointer(&pokered_symbols::wObtainedBadges);
    let owned = state.pokedex_owned.species();
    let seen = state.pokedex_seen.species();

    println!("== {name}");
    println!("   map:     {} @ {}", state.map.map, state.map.player_position);
    println!("   badges:  {badge_bits} ({:?})", state.badges);
    println!("   money:   ¥{}", state.money);
    println!("   coins:   {} (wPlayerCoins)",
        encoding::reverse_bcd(mmu.read_pointer_u16_be(&pokered_symbols::wPlayerCoins) as u32));
    println!("   dex:     OWNED {} / SEEN {}", owned.len(), seen.len());
    let owned_names: Vec<String> = owned.iter().map(|s| format!("{s:?}")).collect();
    println!("   owned:   {}", owned_names.join(", "));
    println!("   storage: wBoxCount={} wCurrentBoxNum={}",
        mmu.read_pointer(&pokered_symbols::wBoxCount),
        mmu.read_pointer(&pokered_symbols::wCurrentBoxNum));

    // The open box only — the other eleven are in SRAM banks the pointer reader can't reach yet.
    if state.boxed_pokemon.is_empty() {
        println!("   box{}: empty", state.current_box + 1);
    } else {
        println!("   box{}[{}]:", state.current_box + 1, state.boxed_pokemon.len());
        for (i, p) in state.boxed_pokemon.iter().enumerate() {
            let moves: Vec<String> = p.moves.iter().flatten()
                .map(|m| format!("{:?}(pp{})", m.name, m.pp)).collect();
            println!("     box{i}: {:?} \"{}\" lv{} {}hp — {}",
                p.species, p.nickname, p.level, p.current_hp, moves.join(", "));
        }
    }

    println!("   party[{}]:", state.pokemon.len());
    for (i, p) in state.pokemon.iter().enumerate() {
        let moves: Vec<String> = p.moves.iter().flatten()
            .map(|m| format!("{:?}(pp{})", m.name, m.pp)).collect();
        println!("     slot{i}: {:?} lv{} {}/{}hp — {}",
            p.species, p.level, p.current_hp, p.stats.hp, moves.join(", "));
    }

    // Raw bag: `wBagItems` is (id, qty) pairs, `wNumBagItems` long.
    let count = mmu.read_pointer(&pokered_symbols::wNumBagItems) as usize;
    let base = pokered_symbols::wBagItems.address;
    let items: Vec<String> = (0..count).map(|i| {
        let id = mmu.read(base + i as u16 * 2);
        let qty = mmu.read(base + i as u16 * 2 + 1);
        // `ItemId` names only a handful of the machines, but the postgame plan cares which ones are
        // in the bag (they are the obvious things to toss for space, and H wants HM05). Decode the
        // rest by id: `constants/item_constants.asm` puts HM01–HM05 at $C4 and TM01–TM50 at $C9.
        let label = if let Some(item) = ItemId::from_repr(id) {
            format!("{item:?}")
        } else if (0xC4..=0xC8).contains(&id) {
            format!("HM{:02}", id - 0xC3)
        } else if (0xC9..=0xFA).contains(&id) {
            format!("TM{:02}", id - 0xC8)
        } else {
            format!("${id:02x}")
        };
        format!("{label}x{qty}")
    }).collect();
    println!("   bag[{count}/{}]: {}", Bag::MAX_ITEMS, items.join(", "));
}

/// Micro-benchmark, not a test: raw emulation throughput vs. the full agent step, from a mid-game
/// fixture. Establishes which half of the loop to optimise — as of writing, **23× realtime raw and
/// 20× with the agent**, i.e. the emulator is the cost and the agent's observe/policy/input work is
/// only ~11% on top. Run with
/// `cargo test --release --bin gb -- bench_emulation_throughput --exact --ignored --nocapture`.
#[test]
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
    // (b) Full agent step (observe + policy + input synthesis) — the real playthrough cost.
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
