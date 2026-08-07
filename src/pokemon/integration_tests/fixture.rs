//! The shared harness every test in this module drives: a `GameBoy` restored from a snapshot, wired
//! to a `PokemonAgent` running a `DeterministicPolicy`, plus the stall/budget guards that turn a wedged
//! agent into a fast failure with a screenshot instead of a test that runs to the cycle cap.

use super::*;

/// Why a run stopped early. Carries the message `step` would have panicked with, so the panicking
/// and non-panicking paths report identically.
struct RunFailure(String);

impl RunFailure {
    /// Which `target/test-artifacts/` pair to write. A stall and a timeout are debugged
    /// differently, so they are kept apart — `probe_stall_artifact` and `probe_timeout_artifact`
    /// each read one of them.
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
    /// Cycles since the policy queue length last changed (stall detection).
    stall_cycles: MachineCycles,
    last_steps_remaining: Option<usize>,
    /// How long without queue progress before we declare a stall.
    stall_threshold: MachineCycles,
    /// False until the first [`Self::step`], so the load-time write is not reported as a drift.
    options_reapplied: bool,
    /// **J3** — how many times the game has restored its own `wOptions` over the harness's, i.e. how
    /// many save/reload boundaries this run has crossed. `> 0` is the evidence that re-applying every
    /// tick is load-bearing rather than belt-and-braces; see `options_survive_the_hall_of_fame_reset`.
    pub options_drifts: u32,
    /// The options this fixture holds the game to. Normally
    /// [`crate::pokemon::postgame::debug::FAST_FIXTURE_OPTIONS`]; a leg pinned to the *original*
    /// battle timing swaps it with [`Self::with_original_battle_timing`].
    options: GameOptions,
    /// How many policy steps the run was handed, so a failure can report how far it got.
    steps_at_start: Option<usize>,
}

impl TestFixture {
    pub fn new(save_state: &[u8], max_game_time: Duration, policy_steps: Vec<PolicyStep>) -> Self {
        Self::with_policy(save_state, max_game_time, Box::new(DeterministicPolicy::new(42, policy_steps)))
    }

    /// [`Self::new`] with the policy supplied rather than built. Everything else — the options pin,
    /// the stall detector, the cycle budget — is identical, so a test that needs to observe what the
    /// agent asks its policy gets the whole harness rather than hand-rolling a `GameBoy` beside it.
    pub fn with_policy(save_state: &[u8], max_game_time: Duration, policy: Box<dyn crate::pokemon::policy::Policy>) -> Self {
        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(save_state).expect("failed to load save state");

        // The agent builds its world graph incrementally as it traverses.
        let steps_at_start = policy.steps_remaining();

        // **Workstream J2.** Applied here rather than by regenerating the 27 committed `.bin`s: one
        // place, every tier, and no chance of a half-regenerated chain. See
        // [`crate::pokemon::postgame::debug::FAST_FIXTURE_OPTIONS`] for what the three bits are and
        // why the plan's proposed byte was wrong.
        let options = crate::pokemon::postgame::debug::FAST_FIXTURE_OPTIONS;
        PokemonApi::new(&mut gb).debug_set_options(&options);

        Self {
            options,
            steps_at_start,
            gb,
            map_cache: MapMetadataCache::default(),
            total_cycles: MachineCycles::ZERO,
            max_cycles: MachineCycles::from_duration(max_game_time),
            stall_cycles: MachineCycles::ZERO,
            last_steps_remaining: None,
            // 10 minutes of game time without a queue step change → stall
            stall_threshold: MachineCycles::from_duration(Duration::from_secs(10 * 60)),
            options_reapplied: false,
            options_drifts: 0,
            agent: PokemonAgent::new(policy),
        }
    }

    /// **J, opt-out** — hold this fixture to the options the suite used *before* workstream J, i.e.
    /// with battle animations **on**.
    ///
    /// ⚠️ This is not a preference, it is a **pin to an RNG stream**. Fewer frames per battle means
    /// the ROM samples `hRandomAdd`/`hRandomSub` at different moments, so turning animations off
    /// changes every encounter roll and every catch roll downstream — and a leg whose *route* was
    /// tuned against one stream can land on the wrong side of a map on another. Four mainline legs do
    /// exactly that (`celadon::can_reach_lavender` stalls at Route 10 (12,20), unable to reach
    /// Lavender because a differently-timed wild battle left it in the route's southern pocket; the
    /// same shape for `saffron::can_enter_saffron`, `cinnabar::can_get_volcano_badge` and
    /// `endgame::can_beat_elite_four`).
    ///
    /// Re-cutting them is not the answer: §4.2 of the plan **freezes** `complete_game_steps` and
    /// `full_playthrough`, and §3 puts battle tactics out of scope on the grounds that in deployment
    /// they are the LLM's decisions. So the mainline chain keeps the stream it was cut against and
    /// the postgame chain — all 66 legs of it, cut after J — gets the 20 % .
    pub fn with_original_battle_timing(mut self) -> Self {
        self.options = GameOptions { battle_animations_on: true,
                                     ..crate::pokemon::postgame::debug::FAST_FIXTURE_OPTIONS };
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

    /// [`Self::step`] without the panic. Diagnostics that sweep several targets use this so that
    /// one unreachable target reports itself and the sweep carries on, instead of taking the whole
    /// probe down with it.
    fn try_step(&mut self) -> Result<(), RunFailure> {
        let cycles = self.gb.run(AGENT_RESOLUTION);

        let mut api = PokemonApi::with_cache(&mut self.gb, &mut self.map_cache);
        // **Workstream J3.** `wOptions` is inside the block the game copies to SRAM on a save and
        // copies back on CONTINUE, so a soft reset — which Phase 0's Hall-of-Fame walk-out performs,
        // and `change_box` sets up — restores the cartridge's own options over the ones J2 wrote.
        // Re-applying is a byte compare per tick and it cannot be undone by anything the game does;
        // poking the SRAM copy instead would fail `sMainDataCheckSum`. `probe_fixture_options` is the
        // proof.
        if api.debug_set_options(&self.options) && self.options_reapplied {
            self.options_drifts += 1;
            println!("[fixture] wOptions drifted (a save/reload restored the cartridge's) — re-applied");
        }
        self.options_reapplied = true;
        self.agent.update(&mut api, cycles).ok();

        self.total_cycles += cycles;

        // Stall detection: GrindUntilLevel and CatchPokemon legitimately sit on the
        // same step for long stretches — exempt them regardless of queue length. A battle gets a
        // *longer leash* rather than an exemption: the queue cannot advance mid-fight, so a six-mon
        // rival or an Elite Four room needs far more than the usual threshold — but a fight can also
        // deadlock outright (an attacker out of PP, healing itself forever against a mon it cannot KO),
        // and that has to keep failing fast instead of running to the cycle cap.
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
    ///
    /// A bare "policy stalled" says nothing about whether the run died on its first step or its last,
    /// and for a long run that is the difference between a typo and a regression. `full_playthrough`
    /// sat broken for a long time partly because its failure looked identical at 40 % and at 99 %.
    fn progress_note(&self) -> String {
        match (self.steps_at_start, self.agent.policy_steps_remaining()) {
            (Some(total), Some(left)) if total > 0 => format!(
                " — completed {}/{} policy steps ({}%); resume with RESUME_QUEUE_LEN={left}",
                total - left, total, (total - left) * 100 / total),
            _ => String::new(),
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

    /// [`Self::run_until`] without the panic: `None` means the run stalled or ran out of budget
    /// before `done` was satisfied, and the reason has already been printed.
    ///
    /// For diagnostics only. A *test* wants `run_until`, so that a route that stops working fails
    /// loudly instead of quietly reporting nothing.
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
    ///
    /// `Interact` and `CollectItem` pop the moment they *issue* the walk, so the queue routinely
    /// empties while the effect it was queued for — an item landing in the bag, an NPC's text box —
    /// is still in flight. This is the "exhaust, then wait for the effect" idiom.
    ///
    /// ⚠️ **This is the one harness call that can hide a mainline bug**, so it reports its own slack.
    /// A leg test ends when the queue empties; `complete_game_steps` does not — it moves straight on to
    /// the next leg's steps. So a leg that only reaches `done` because the agent kept acting *after*
    /// exhaustion is green here and wedges in [`super::playthrough::full_playthrough`], which is
    /// exactly what happened with the Poké Flute: `Interact(MRFUJISHOUSE_MR_FUJI)` popped before the
    /// conversation, `run_leg` waited and the agent finished it unprompted, and the mainline instead
    /// walked away without the Flute and stalled 200 steps later. The slack is printed rather than
    /// asserted because a handful of ticks is the normal in-flight case; a *large* number means the
    /// step list is leaning on the agent's autonomy and will not survive composition.
    pub fn run_leg(&mut self, done: impl Fn(&GameState) -> bool) -> GameState {
        self.step_until_exhausted();
        let before = self.total_cycles;
        let state = self.run_until(done);
        let slack = (self.total_cycles - before).to_duration();
        if slack > Duration::from_secs(5) {
            println!("[fixture] ⚠️  run_leg waited {slack:?} of game time AFTER the queue emptied — \
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
#[cfg(feature = "diagnostics")]
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
#[cfg(feature = "diagnostics")]
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
        // Workstream C's chain, rooted on B's output: Old Rod → a Magikarp → Good Rod + a Goldeen →
        // Super Rod + a Tentacool (with the first two banked at the Viridian PC).
        ("postgame-old-rod", include_bytes!("../data/postgame-old-rod.bin")),
        ("postgame-magikarp", include_bytes!("../data/postgame-magikarp.bin")),
        ("postgame-good-rod", include_bytes!("../data/postgame-good-rod.bin")),
        ("postgame-fishing", include_bytes!("../data/postgame-fishing.bin")),
        // Workstream F's chain, also rooted on B's output: the Coin Case → 200 coins → three junk
        // TMs sold → an Abra out of the prize room.
        ("postgame-coin-case", include_bytes!("../data/postgame-coin-case.bin")),
        ("postgame-coins", include_bytes!("../data/postgame-coins.bin")),
        ("postgame-sold", include_bytes!("../data/postgame-sold.bin")),
        ("postgame-game-corner", include_bytes!("../data/postgame-game-corner.bin")),
        // Workstream G-gifts' chain, also rooted on B's output: Omanyte → Aerodactyl → Lapras (boxed,
        // full party) → Hitmonlee (in the party, a slot banked first).
        ("postgame-omanyte", include_bytes!("../data/postgame-omanyte.bin")),
        ("postgame-aerodactyl", include_bytes!("../data/postgame-aerodactyl.bin")),
        ("postgame-lapras", include_bytes!("../data/postgame-lapras.bin")),
        ("postgame-hitmonlee", include_bytes!("../data/postgame-hitmonlee.bin")),
        ("postgame-silph-floors", include_bytes!("../data/postgame-silph-floors.bin")),
        ("postgame-gifts", include_bytes!("../data/postgame-gifts.bin")),
        ("postgame-daycare", include_bytes!("../data/postgame-daycare.bin")),
        ("postgame-name-rater", include_bytes!("../data/postgame-name-rater.bin")),
        // Workstream G-trades' chain, rooted on G-gifts' output.
        ("postgame-mr-mime", include_bytes!("../data/postgame-mr-mime.bin")),
        ("postgame-farfetchd", include_bytes!("../data/postgame-farfetchd.bin")),
        ("postgame-trades", include_bytes!("../data/postgame-trades.bin")),
        ("postgame-tangela", include_bytes!("../data/postgame-tangela.bin")),
        // Workstream H, rooted on G-trades' output — the dex count is what gates it.
        ("postgame-flash", include_bytes!("../data/postgame-flash.bin")),
        // Workstream E, rooted on H's output — the four-area sweep that takes the dex past H3's gate.
        ("postgame-safari", include_bytes!("../data/postgame-safari.bin")),
        // …and H again, resumed on E's output: the Itemfinder at 30 owned, then a hidden item.
        ("postgame-itemfinder", include_bytes!("../data/postgame-itemfinder.bin")),
        ("postgame-hidden-item", include_bytes!("../data/postgame-hidden-item.bin")),
        // H5's sweep chain.
        ("postgame-sweep-vermilion", include_bytes!("../data/postgame-sweep-vermilion.bin")),
        ("postgame-sweep-viridian", include_bytes!("../data/postgame-sweep-viridian.bin")),
        ("postgame-sweep-lavender", include_bytes!("../data/postgame-sweep-lavender.bin")),
        ("postgame-aides", include_bytes!("../data/postgame-aides.bin")),
        // Workstream K, rooted on H's output.
        ("postgame-seel", include_bytes!("../data/postgame-seel.bin")),
        // Workstream I's chain, also rooted on H's output: medicine → the Itemfinder and a Repel →
        // an Ether and the hidden PP Up → the stat items and a Poké Doll.
        ("postgame-medicine", include_bytes!("../data/postgame-medicine.bin")),
        ("postgame-finder", include_bytes!("../data/postgame-finder.bin")),
        ("postgame-ether", include_bytes!("../data/postgame-ether.bin")),
        ("postgame-items", include_bytes!("../data/postgame-items.bin")),
    ];
    for (name, bytes) in FIXTURES {
        print_coverage(name, bytes);
    }
}

/// The body of [`probe_coverage`] for one snapshot. Separate so a workstream can call it on a state
/// it has just driven to, not only on a committed file.
pub fn print_coverage(name: &str, save_state: &[u8]) {
    // A fixture that has never been regenerated is a zero-byte placeholder — `include_bytes!` needs
    // the file to exist before the leg that writes it can compile. Say so rather than panicking and
    // taking the other thirty snapshots' output with it.
    if save_state.is_empty() {
        println!("== {name}: EMPTY placeholder — run its leg with --features regen-fixtures");
        return;
    }
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
    // Task E1: the Safari trip's two budgets. `None` off the clock, which is every fixture that was
    // not saved mid-hunt — printed anyway, because "not in the zone" is the thing a leg starting with
    // `Fly` needs to be true.
    println!("   safari:  {}", match state.safari {
        Some(s) => format!("{} steps, {} balls left{}",
            s.steps_left, s.balls_left, if s.game_over { ", GAME OVER" } else { "" }),
        None => "not in the zone".into(),
    });

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

    // Raw inventories: `wBagItems` / `wBoxItems` are (id, qty) pairs, `wNum*Items` long.
    let read_inventory = |count_at: &_, base: u16| -> (usize, Vec<String>) {
        let count = mmu.read_pointer(count_at) as usize;
        let items = (0..count).map(|i| {
            let id = mmu.read(base + i as u16 * 2);
            let qty = mmu.read(base + i as u16 * 2 + 1);
            // `ItemId` names only a handful of the machines, but the postgame plan cares which ones
            // are held (they are the obvious things to shed for space, and H wants HM05). Decode the
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
        (count, items)
    };
    let (count, items) = read_inventory(&pokered_symbols::wNumBagItems, pokered_symbols::wBagItems.address);
    println!("   bag[{count}/{}]: {}", Bag::MAX_ITEMS, items.join(", "));
    // Item PC storage (Phase 0 tasks 0.5/0.6) — the only reason the 20-slot bag is survivable, so a
    // probe that shows the bag without it shows half the picture.
    let (stored, items) = read_inventory(&pokered_symbols::wNumBoxItems, pokered_symbols::wBoxItems.address);
    println!("   PC items[{stored}]: {}", items.join(", "));
}

/// Micro-benchmark, not a test: raw emulation throughput vs. the full agent step, from a mid-game
/// fixture. Establishes which half of the loop to optimise — re-measured 2026-08-05 on a Ryzen 9
/// 7900X at **33.6× realtime raw and 28.3× with the agent**, i.e. the emulator is the cost and the
/// agent's observe/policy/input work is only ~16% on top.
///
/// For the emulator core measured *alone*, which is what Phase C of
/// `docs/compatibility/10-implementation-plan.md` is scored against, see
/// `game_boy::tests::bench_core_throughput`.
///
/// ```text
/// cargo test --release --features bench --bin gb -- \
///   pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture
/// ```
#[test]
#[cfg(feature = "bench")]
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
