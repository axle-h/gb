//! The stall hunt: hours of random play, watching for the agent to stop asking the policy anything.
//!
//! Behind the `soak-tests` feature, and **gated as a module rather than with `#[ignore]`** — the
//! ignored list is a backlog of *blocked* tests (see `CLAUDE.md`), and this is neither blocked nor a
//! backlog item. Without the feature it does not exist.
//!
//! # What it is for
//!
//! `RandomPolicy` is not how the game gets played; it is how the *agent* gets tested. A random
//! decider walks into map corners, menus and states a scripted policy never visits, which makes it a
//! far better explorer of the agent's state machine than `full_playthrough` — which walks one
//! carefully chosen route and proves that route still works.
//!
//! What it watches is [`PokemonAgent::since_last_policy_poll`], the **same** measure W9's watchdog
//! reads. That is deliberate: this test fails exactly when a deployed `LlmPolicy` would have its
//! watchdog fire. So a pass means "nothing here would have woken the model", and a failure is a real
//! bug report rather than a threshold argument.
//!
//! ⚠️ **`RandomPolicy` deliberately has no watchdog of its own** (`stuck_timeout()` is `None`). That
//! is what makes it a detector: a policy that nudged itself out of a jam would hide the jam, which is
//! the one thing this test exists to see.
//!
//! # Two real bugs it would have caught
//!
//! Both found by hand, in production, on the deployed instance — which is the argument for the test:
//!
//! - **The PC menus** (`PokemonApiTrait::in_pc_menu`). Every Gen 1 PC menu is a closed loop under
//!   A-only input, and the agent A-mashed one for fifteen minutes eight tiles from a fresh save.
//! - **Grass with nothing in it** (`MetaTileMap::has_grass_encounters`). Every town and city draws
//!   real tall grass over `wGrassRate == 0`, so pacing there waits for an encounter that cannot
//!   happen. It paced Pallet Town for eleven minutes.
//!
//! Both are reachable within minutes of a fresh save under random play, and neither was reachable by
//! any scripted test — the scripted policy never chooses to walk into them.

use super::*;
use crate::pokemon::policy::RandomPolicy;

/// How much **game time** one soak covers.
///
/// Random play measures at ~73× realtime (2026-08-11, Ryzen 9 7900X) — faster than the ~50× in
/// `CLAUDE.md`, which is for `DeterministicPolicy` doing real work — so five hours of game time is
/// about four minutes of wall clock. That is the budget this was sized to.
const SOAK_GAME_TIME: Duration = Duration::from_secs(5 * 60 * 60);

/// How often to print, in game time. libtest shows nothing until a test finishes, so a multi-minute
/// run that says nothing is indistinguishable from a hung one.
const PROGRESS_EVERY: Duration = Duration::from_secs(30 * 60);

/// Events kept for the failure report — enough to see what the agent was doing on the way in.
const EVENT_TAIL: usize = 12;

/// The seed the fuzzer plays by default.
///
/// ⚠️ **Fixed, not drawn.** A soak that reseeds itself every run is a lottery: a failure vanishes the
/// moment you go back to look at it, a "pass" proves only that *this* draw was clean, and CI flakes.
/// Pinning it makes a green run mean "this sequence is still clean" — which is what a regression test
/// has to mean. To hunt for *new* jams, vary it:
///
/// ```shell
/// for seed in $(seq 1 20); do
///   GB_SOAK_SEED=$seed cargo test --release --features soak-tests --bin gb -- soak --nocapture
/// done
/// ```
const DEFAULT_SEED: u64 = 1;

/// Hours of random play without the agent going quiet for longer than the deployed watchdog allows.
///
/// The limit is `GB_STUCK_TIMEOUT_SECS`' default rather than a number of this test's own, so there is
/// exactly one definition of "stuck" in the project and this cannot drift away from the thing it is
/// meant to predict.
#[test]
fn random_play_never_goes_quiet_for_longer_than_the_watchdog_allows() {
    let limit = Duration::from_secs(crate::llm::config::DEFAULT_STUCK_TIMEOUT_SECS);
    let seed: u64 = std::env::var("GB_SOAK_SEED").ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_SEED);
    println!("[soak] seed {seed} — {SOAK_GAME_TIME:?} of game time, watchdog limit {limit:?}");

    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(crate::pokemon::data::START_OF_GAME).expect("the committed start-of-game fixture loads");
    // ⚠️ No `debug_set_options` here, unlike `TestFixture`. Its fast-text options make a leg cheaper,
    // but they also change how long the agent spends in every text box — and this test is trying to
    // reproduce what the *deployment* does, which runs on the cartridge's own settings.
    let mut cache = MapMetadataCache::default();
    let mut agent = PokemonAgent::new(Box::new(RandomPolicy::seeded(seed)));

    let target = MachineCycles::from_duration(SOAK_GAME_TIME);
    let mut emulated = MachineCycles::ZERO;
    let mut next_progress = PROGRESS_EVERY;
    let mut worst = Duration::ZERO;
    let mut worst_state = String::from("idle");
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    while emulated < target {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;

        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        // A failure here is not what is being tested — `update` reports transient read errors during
        // map transitions — but a wedged agent shows up as silence either way.
        agent.update(&mut api, ran).ok();

        // ⚠️ Drained every tick, not left to accumulate. Nothing here reads them for their own sake;
        // the buffer is unbounded and a five-hour run would otherwise grow one event per decision for
        // the whole test.
        for event in agent.drain_events() {
            tail.push_back(format!("{event:?}"));
            if tail.len() > EVENT_TAIL { tail.pop_front(); }
        }

        let gap = agent.since_last_policy_poll();
        if gap > worst {
            worst = gap;
            worst_state = agent.state_debug();
        }
        if gap >= limit {
            let state = PokemonApi::with_cache(&mut gb, &mut cache).game_state().ok();
            let dir = std::path::Path::new("target/test-artifacts");
            std::fs::create_dir_all(dir).ok();
            gb.save_state_to_file(&dir.join("soak_stall_state.bin").to_string_lossy()).ok();
            gb.save_screenshot_to_file(&dir.join("soak_stall_screenshot.png").to_string_lossy()).ok();

            panic!(
                "the agent went {gap:?} of game time without reaching a decision point — \
                 a deployed LlmPolicy's watchdog would have fired here.\n\
                 \x20 agent state: {}\n\
                 \x20 where: {}\n\
                 \x20 after: {:?} of game time\n\
                 \x20 last {} events:\n{}\n\
                 \x20 reproduce: GB_SOAK_SEED={seed} cargo test --release --features soak-tests \
                    --bin gb -- soak --nocapture\n\
                 \x20 artifacts: target/test-artifacts/soak_stall_{{state.bin,screenshot.png}}",
                agent.state_debug(),
                state.as_ref().map_or_else(
                    || "unreadable".to_string(),
                    |s| format!("{} at {}", s.map.map, s.map.player_position)),
                emulated.to_duration(),
                tail.len(),
                tail.iter().map(|e| format!("    {e}")).collect::<Vec<_>>().join("\n"),
            );
        }

        if emulated.to_duration() >= next_progress {
            next_progress += PROGRESS_EVERY;
            println!("[soak] {:?} of game time — longest quiet stretch so far {worst:?} ({worst_state})",
                     emulated.to_duration());
        }
    }

    // The *emulated* total, not the constant it was asked for: a summary that prints its own target
    // back cannot tell you the loop exited early, which is exactly the failure it should surface.
    println!("[soak] seed {seed}: {:?} of random play, longest quiet stretch {worst:?} in state \
              {worst_state:?} (the watchdog fires at {limit:?})", emulated.to_duration());
    assert!(worst < limit, "checked in the loop above");
}
