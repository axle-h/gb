//! Stalls the fuzzer found, each frozen into a save state and re-run in a second.
//!
//! **Default tier on purpose.** `soak` is where a jam gets *found* — five hours of random play, a few
//! minutes of wall clock, and it only visits what its seed happens to visit. That is the wrong tool
//! for proving a jam stays fixed. So every one it finds is promoted here: the emulator state at the
//! moment the agent went quiet, `include_bytes!`'d, replayed against a fresh agent, and asserted to
//! reach a decision point. Each costs about a second, so they run on every `cargo test --release`.
//!
//! # Adding one
//!
//! `soak` drops `target/test-artifacts/soak_stall_state.bin` when it fails. Copy it to
//! `src/pokemon/data/stall-<what>.bin`, add a case below, and **check it fails before the fix** —
//! that is the whole value, and it is not automatic (see the ⚠️).
//!
//! ⚠️ **Not every stall survives the trip, and one that does not must not be committed anyway.** The
//! save state holds the *emulator*, not the agent: a fresh `PokemonAgent` starts `Idle` with an empty
//! world graph and no route in flight. A jam the game's own screen re-creates — a menu that bounces,
//! a battle the agent cannot leave — reproduces perfectly. A jam that lived in the agent's own state,
//! like `OverworldMovement` committed to a route, does not: replaying it just picks a fresh action. So
//! a test added here without watching it go red first may be asserting nothing at all.

use super::*;
use crate::pokemon::policy::RandomPolicy;

/// Game time each case is given to reach a decision point.
const ESCAPE_BUDGET: Duration = Duration::from_secs(120);

/// The longest silence a case may show before it counts as still stuck.
///
/// ⚠️ **Stricter than `soak`'s 300 s, and measured rather than picked.** These start *inside* a jam,
/// so a working agent leaves almost at once and there is nothing legitimate to wait for — a tighter
/// limit catches a regression sooner. But not arbitrarily tighter: five hours of clean random play
/// measured its longest *healthy* silence at **46.8 s** (a battle resolving through its animations),
/// so anything under about 60 s would fail on ordinary play. This is roughly twice the measured worst
/// and well under the watchdog. The first draft used 20 s and failed on a perfectly healthy battle.
const QUIET_LIMIT: Duration = Duration::from_secs(90);

/// Replay `state` against a fresh agent and return the longest it went without reaching a decision
/// point, plus where it ended up.
///
/// The policy is seeded so a case cannot pass or fail on the draw — these assert that the agent can
/// get *out*, which must not depend on what it chooses once it has.
fn longest_silence(state: &[u8], seed: u64) -> (Duration, String, String) {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).expect("a committed stall fixture should load");
    let mut cache = MapMetadataCache::default();
    let mut agent = PokemonAgent::new(Box::new(RandomPolicy::seeded(seed)));

    let budget = MachineCycles::from_duration(ESCAPE_BUDGET);
    let mut emulated = MachineCycles::ZERO;
    let mut worst = Duration::ZERO;
    let mut worst_state = String::new();

    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        agent.drain_events();

        let gap = agent.since_last_policy_poll();
        if gap > worst {
            worst = gap;
            worst_state = agent.state_debug();
        }
    }

    let where_it_is = PokemonApi::with_cache(&mut gb, &mut cache)
        .game_state()
        .map_or_else(|_| "unreadable".into(), |s| format!("{} at {}", s.map.map, s.map.player_position));
    (worst, worst_state, where_it_is)
}

/// Assert a fixture is no longer a stall, reporting what it did if it still is.
fn assert_escapes(name: &str, state: &[u8]) {
    // Three seeds, because escaping must not depend on what the policy picks once it is free.
    for seed in [1, 2, 3] {
        let (worst, worst_state, where_it_is) = longest_silence(state, seed);
        assert!(
            worst < QUIET_LIMIT,
            "{name} (seed {seed}): the agent went {worst:?} of game time without reaching a decision \
             point — still stuck.\n  state: {worst_state}\n  where: {where_it_is}",
        );
        println!("[stall] {name} (seed {seed}): out in {worst:?}, {where_it_is}");
    }
}

/// **`soak` seed 1, 3600 s in** — a Bulbasaur out of PP against a Weedle in Viridian Forest.
///
/// The move list refuses with "No PP left for this move!" and drops back to *itself*, cursor still on
/// the spent move, so the agent's A-mash re-selects it and bounces again — the same closed loop as the
/// PC menus, which is the second time that exact shape has wedged a run.
///
/// ⚠️ **The offered moves were already filtered on `pp > 0`** (`Pokemon::available_battle_moves`), so
/// this is not a bad choice by the policy — it is the game and the party data disagreeing, which no
/// filter over the party data can fix. The agent has to handle the refusal.
#[test]
fn a_move_with_no_pp_left_does_not_trap_the_battle() {
    assert_escapes("no-pp-move", include_bytes!("../data/stall-no-pp-move.bin"));
}

