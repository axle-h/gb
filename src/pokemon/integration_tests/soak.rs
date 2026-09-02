//! The stall hunt: random play from two dozen points in the game, watching for the agent to stop
//! asking the policy anything.
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
//! # ⚠️ Breadth beats depth, and that is a measurement rather than a preference
//!
//! The first version of this was **one** test: five hours of game time from a fresh save. Watching
//! the deployed `--policy random` run is what killed that shape — a random walker does not explore,
//! it *diffuses*. It picks uniformly from the tiles the current map offers, so it spends its time
//! re-crossing the handful of maps around wherever it started, and the second hour visits almost
//! exactly what the first one did. Five hours from Pallet Town is five hours of Pallet Town, Route 1,
//! Viridian and the bedroom it started in; it never sees a bicycle, a Safari step counter, a boulder,
//! a PC with anything in it, or a full bag.
//!
//! So the budget is spent the other way round: **[`SOAK_GAME_TIME`] of game time each, from every
//! entry in [`STATES`]**. Same fuzzer, same detector, one test per starting point, and libtest runs
//! them in parallel — the tier costs about the same wall clock as the single five-hour test did
//! while covering two dozen regions of the game instead of one.
//!
//! # Where the starting points come from
//!
//! Two places, and the split is a budget rather than a taxonomy. A dozen of them are **hand-cut**:
//! saves the leg chain already commits, plus one lifted off a deployed pod, kept because a fuzzer
//! cannot reach what they hold inside forty minutes — a bicycle, a full PC box, every field move
//! learnt. The other dozen are **checkpoints**, cut off the mainline by
//! [`playthrough::regen_soak_checkpoints`](super::playthrough::regen_soak_checkpoints): the route
//! already walks through Mt Moon, an unlit Rock Tunnel, Silph Co, the Safari Zone, the Mansion
//! switches and the Cinnabar quiz, so a copy taken on the way past costs one run of a test that has
//! to be run anyway and no fixture-cutting at all. They carry an `expect_map` and the regeneration
//! fails on a map the route no longer crosses, so neither half can rot quietly.
//!
//! # ⚠️ The walker is biased away from what it has just done
//!
//! Breadth over depth is also applied *within* a soak, because the diffusion above is not only a
//! property of long runs: a uniform walker's distance from where it started grows as the square root
//! of its steps, so the last ten minutes of a forty-minute soak were largely re-crossing the first
//! ten. [`RandomPolicy::exploring`] keeps the ids of the last two dozen overworld actions it chose
//! and weights the draw down for each repeat, which is enough to stop the two-action ping-pong the
//! diffusion is mostly made of (through the door, back through the door) without forbidding
//! anything — Red has floors whose only exit is the one just used. The battle draw is untouched;
//! the argument for that is on `exploring` too.
//!
//! # ⚠️ The options are forced to the *cartridge's* defaults, not the harness's
//!
//! Unlike [`TestFixture`], which applies `FAST_FIXTURE_OPTIONS`, this writes what `InitOptions`
//! writes: medium text, battle animations **on**, battle style SHIFT. `gb serve` runs on those, and
//! the soak exists to reproduce the deployment rather than to be cheap — the no-PP jam was a race
//! with the character-by-character text renderer, and fast text may well not reproduce it.
//!
//! It has to be *written*, not merely left alone: every fixture past `start-of-game-state.bin` was
//! captured mid-leg by `TestFixture`, so its `wOptions` carries fast text and animations off. A soak
//! seeded from one and left alone would silently fuzz the harness's settings instead of the
//! deployment's.
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
use crate::pokemon::options::{BattleStyle, GameOptions, TextSpeed};
use crate::pokemon::policy::RandomPolicy;
use super::playthrough::SOAK_CHECKPOINTS;

/// How much **game time** one soak covers, per starting state.
///
/// Random play measures at ~62× realtime, so this is about 39 s of wall clock each (2026-09-02,
/// 16 threads); libtest runs them together and the tier as a whole lands at about 5.5 min, which is
/// memory bandwidth rather than cores — 26 emulators and 26 world graphs do not fit in a cache
/// between them. Sized by what a random walker actually adds per minute rather than by what the
/// machine can afford: the deployed run makes it plain that the *n*th minute from one starting point
/// is worth far less than the first minute from a new one, so the budget buys starting points.
///
/// Override with `GB_SOAK_MINUTES` to go deeper from a state that looks interesting.
const SOAK_GAME_TIME: Duration = Duration::from_secs(40 * 60);

/// How often to print, in game time. libtest shows nothing until a test finishes, so a multi-minute
/// run that says nothing is indistinguishable from a hung one.
const PROGRESS_EVERY: Duration = Duration::from_secs(10 * 60);

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

/// The options `gb serve` plays on: pokered's own `InitOptions` byte, `TEXT_DELAY_MEDIUM` — which is
/// battle animations **on** and battle style SHIFT, neither of which any other tier ever sees.
const DEPLOYMENT_OPTIONS: GameOptions = GameOptions {
    battle_animations_on: true,
    battle_style: BattleStyle::Shift,
    text_speed: TextSpeed::Medium,
};

/// One place the fuzzer is turned loose from.
struct SoakState {
    /// Matches the fixture's file stem, so a failure names something greppable.
    name: &'static str,
    state: &'static [u8],
    /// What this state puts within reach that the others do not — the reason it is in the list.
    /// Printed on every run, so the log says what was actually covered.
    covers: &'static str,
    /// Where the capture is supposed to have been taken, asserted on the way in.
    ///
    /// ⚠️ **Only the checkpoints carry one, and that is what makes them safe to regenerate.** A
    /// hand-cut fixture is where somebody put it; a checkpoint is wherever the route happened to be
    /// when [`regen_soak_checkpoints`](super::playthrough::regen_soak_checkpoints) last ran, and a
    /// route that stops crossing that map would otherwise leave a stale `.bin` fuzzing somewhere
    /// nobody chose. The regen fails on a checkpoint it could not reach; this fails on one whose
    /// bytes were never replaced.
    expect_map: Option<Map>,
}

/// Where the fuzzer starts, chosen for **what is reachable from each**, not for progression.
///
/// The rule for adding one: it must put a *mechanism* within a random walker's reach that nothing
/// else here does — a bicycle, a step-limited zone, a boulder, a PC with something in it. A state
/// that merely has more badges than its neighbour buys nothing, because the fuzzer does not care
/// about badges; it cares about which screens it can end up in front of.
const STATES: &[SoakState] = &[
    SoakState {
        name: "start-of-game",
        state: crate::pokemon::data::START_OF_GAME,
        covers: "a fresh save: no party, no bag, Oak's script, and the item PC eight tiles away",
        expect_map: None,
    },
    SoakState {
        name: "viridian-forest",
        state: include_bytes!("../data/viridian-forest.bin"),
        covers: "the rarest grass in the game (8/256) — where the pacing budget was found",
        expect_map: None,
    },
    SoakState {
        name: "route3-ledge-pocket",
        state: include_bytes!("../data/route3-ledge-pocket.bin"),
        covers: "a map border sealed by one-way ledges: the only way on is five tiles west of the \
                 only way the player can see",
        expect_map: None,
    },
    SoakState {
        name: "at-vermilion",
        state: include_bytes!("../data/at-vermilion.bin"),
        covers: "a port city: gym, mart, Pokémon Centre PC, the S.S. Anne gangway",
        expect_map: None,
    },
    SoakState {
        name: "at-lavender",
        state: include_bytes!("../data/at-lavender.bin"),
        covers: "Pokémon Tower with no Silph Scope — the ghosts that cannot be fought",
        expect_map: None,
    },
    SoakState {
        name: "at-celadon",
        state: include_bytes!("../data/at-celadon.bin"),
        covers: "the department store's nested buy/sell menus and the Game Corner",
        expect_map: None,
    },
    SoakState {
        name: "at-rocket-hideout",
        state: include_bytes!("../data/at-rocket-hideout.bin"),
        covers: "spin tiles and a lift — the map the player does not steer",
        expect_map: None,
    },
    SoakState {
        name: "at-saffron",
        state: include_bytes!("../data/at-saffron.bin"),
        covers: "Silph Co's teleport pads and the gym's warp maze",
        expect_map: None,
    },
    SoakState {
        name: "at-cinnabar",
        state: include_bytes!("../data/at-cinnabar.bin"),
        covers: "surf, the mansion's switches, and the lab's fossil trades",
        expect_map: None,
    },
    SoakState {
        name: "post-safari",
        state: include_bytes!("../data/post-safari.bin"),
        covers: "the Safari Zone: a step counter, ROCK/BAIT, and no fighting",
        expect_map: None,
    },
    SoakState {
        name: "vr1f-strength",
        state: include_bytes!("../data/vr1f-strength.bin"),
        covers: "Victory Road's boulder switches — a map that needs a field move to leave",
        expect_map: None,
    },
    SoakState {
        name: "postgame-pc-box",
        state: include_bytes!("../data/postgame-pc-box.bin"),
        covers: "a PC with mons in the box: withdraw, deposit and release all reachable",
        expect_map: None,
    },
    SoakState {
        name: "postgame-fly-bike",
        state: include_bytes!("../data/postgame-fly-bike.bin"),
        covers: "a bicycle and Fly — the whole map open, and Cycling Road's forced scroll",
        expect_map: None,
    },
    SoakState {
        name: "postgame-aides",
        state: include_bytes!("../data/postgame-aides.bin"),
        covers: "the chain head: a full bag, a full party, and every field move learnt",
        expect_map: None,
    },

    // ── Checkpoints cut off the mainline by `playthrough::regen_soak_checkpoints` ────────────────
    //
    // ⚠️ **Everything above is hand-cut and everything below is not, and the difference is which
    // half of the map each half buys.** The states above exist because a fuzzer cannot get to them
    // on its own inside forty minutes — a bicycle, a full PC box, a ledge pocket a deployed run
    // really got stuck in — and each one cost somebody a fixture. The states below are the route
    // itself: `full_playthrough` already walks past every one of them, so taking a copy on the way
    // through costs one run of a test that has to be run anyway and no hand-cutting at all. See
    // `SOAK_CHECKPOINTS`, which is where the list of them lives and what regenerates them.
    SoakState {
        name: "soak-mt-moon",
        state: include_bytes!("../data/soak-mt-moon.bin"),
        covers: "a cave with no map of itself: the fossil pair, a Rocket that has to be beaten to pass, and \
                 ladders in three directions",
        expect_map: Some(Map::MtMoonB2F),
    },
    SoakState {
        name: "soak-ss-anne",
        state: include_bytes!("../data/soak-ss-anne.bin"),
        covers: "a ship of identical cabin doors, on a map that stops existing once the route leaves it",
        expect_map: Some(Map::SSAnne1F),
    },
    SoakState {
        name: "soak-rock-tunnel",
        state: include_bytes!("../data/soak-rock-tunnel.bin"),
        covers: "an unlit cave: the route crosses it with no Flash, so the walker cannot see the tile it is \
                 choosing",
        expect_map: Some(Map::RockTunnel1F),
    },
    SoakState {
        name: "soak-pokemon-tower",
        state: include_bytes!("../data/soak-pokemon-tower.bin"),
        covers: "the tower with the Silph Scope: Channelers that battle, and the ghosts that no longer refuse",
        expect_map: Some(Map::PokemonTower5F),
    },
    SoakState {
        name: "soak-route12-snorlax",
        state: include_bytes!("../data/soak-route12-snorlax.bin"),
        covers: "a road blocked by a sleeping Snorlax, with a fishing rod in the bag and water on both sides",
        expect_map: Some(Map::Route12),
    },
    SoakState {
        name: "soak-safari-zone",
        state: include_bytes!("../data/soak-safari-zone.bin"),
        covers: "inside the Safari Zone rather than after it: the step counter running, ROCK/BAIT, and no \
                 fighting",
        expect_map: Some(Map::SafariZoneCenter),
    },
    SoakState {
        name: "soak-silph-co",
        state: include_bytes!("../data/soak-silph-co.bin"),
        covers: "Silph Co mid-errand: teleport pads, a lift, and doors that need a Card Key the walker has not \
                 got yet",
        expect_map: Some(Map::SilphCo3F),
    },
    SoakState {
        name: "soak-saffron-gym",
        state: include_bytes!("../data/soak-saffron-gym.bin"),
        covers: "the warp maze: every door leads somewhere in the same room and none of them is the way out",
        expect_map: Some(Map::SaffronGym),
    },
    SoakState {
        name: "soak-pokemon-mansion",
        state: include_bytes!("../data/soak-pokemon-mansion.bin"),
        covers: "the switches, four floors of them, each opening a wall somewhere the walker cannot see",
        expect_map: Some(Map::PokemonMansionB1F),
    },
    SoakState {
        name: "soak-cinnabar-gym",
        state: include_bytes!("../data/soak-cinnabar-gym.bin"),
        covers: "the quiz doors: a yes/no question per gate, and a trainer battle for every wrong answer",
        expect_map: Some(Map::CinnabarGym),
    },
    SoakState {
        name: "soak-viridian-gym",
        state: include_bytes!("../data/soak-viridian-gym.bin"),
        covers: "spin tiles and a warp puzzle, with all seven other badges in hand",
        expect_map: Some(Map::ViridianGym),
    },
    SoakState {
        name: "soak-route23",
        state: include_bytes!("../data/soak-route23.bin"),
        covers: "the badge-checking gates on the way to Victory Road, and the water beside them",
        expect_map: Some(Map::Route23),
    },
];

/// Look a state up by name, so each `#[test]` names its fixture rather than an index into a list
/// somebody will reorder.
fn state(name: &str) -> &'static SoakState {
    STATES.iter().find(|s| s.name == name).expect("every soak test names a state in STATES")
}

/// Read a `u64` out of the environment, falling back to `default` when it is unset or unparseable.
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// Turn `state` loose under `RandomPolicy` and fail if the agent ever goes quiet for longer than the
/// deployed watchdog would allow.
fn soak(state: &SoakState) {
    // `GB_SOAK_LIMIT_SECS` tightens the net below the watchdog's 300 s. The default is the watchdog,
    // because that is the number that actually matters in production — but a *near* miss is a jam in
    // embryo, and the only way to get one to drop its artifact is to lower the bar until it trips.
    // `GB_SOAK_LIMIT_SECS=150` trips on anything twice as quiet as ordinary play and is the useful
    // setting for hunting; it found the pacing budget that way (182 s of silence in Viridian Forest
    // that turned out to be `PACING_BUDGET_TICKS` running to the end, not a jam at all).
    //
    // ⚠️ **Do not go below about 150.** A sweep at 100 kept flagging one shape that turned out to be
    // the game playing properly: a **paralysed Pokémon caught in a wrap chain** gets no menu for
    // several turns (Gen 1 skips your input while WRAP/BIND is running), and on Route 15's line of
    // trainers that is measured at **124 s** of legitimate silence. Seed 1's 62 s is the *quiet* end
    // of the distribution, not the top of it.
    let limit = Duration::from_secs(env_u64("GB_SOAK_LIMIT_SECS",
                                            crate::llm::config::DEFAULT_STUCK_TIMEOUT_SECS));
    let seed = env_u64("GB_SOAK_SEED", DEFAULT_SEED);
    let game_time = std::env::var("GB_SOAK_MINUTES").ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(SOAK_GAME_TIME, |m| Duration::from_secs(m * 60));
    println!("[soak] {} — seed {seed}, {game_time:?} of game time, stall limit {limit:?}\n\
              [soak] {}: {}", state.name, state.name, state.covers);

    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state.state).expect("a committed soak fixture loads");
    let mut cache = MapMetadataCache::default();
    // ⚠️ The *deployment's* options, not `FAST_FIXTURE_OPTIONS` — see the module docs. Written once:
    // nothing a random walker can do reloads them short of a soft reset, and a soak that re-applied
    // them every tick would be papering over exactly the kind of state change it exists to notice.
    PokemonApi::with_cache(&mut gb, &mut cache).debug_set_options(&DEPLOYMENT_OPTIONS);
    // ⚠️ **A checkpoint says where it was cut, and it is checked here rather than at the capture.**
    // The capture cannot be wrong about the map — it is what selects the moment — but the *file* can
    // be: a route that stops crossing PokemonTower5F leaves last year's bytes on disk, and a soak
    // that then fuzzes somewhere nobody chose still passes and still says it covered the tower. This
    // is the seam where the two halves have to agree, so it is where the disagreement is reported.
    if let Some(want) = state.expect_map {
        let on = PokemonApi::with_cache(&mut gb, &mut cache).game_state().map(|s| s.map.map);
        assert_eq!(on.ok(), Some(want),
                   "the `{}` checkpoint is not on {want} any more — re-cut it with \
                    `cargo test --release --features full-playthrough,regen-fixtures --bin gb -- \
                    pokemon::integration_tests::playthrough::regen_soak_checkpoints --exact`",
                   state.name);
    }
    // ⚠️ **`exploring`, not `seeded`** — the recency-weighted draw, whose argument is on
    // `RandomPolicy::exploring`. The short version is that this tier's own module docs record five
    // hours from Pallet Town spent almost entirely in Pallet Town: a uniform walker's distance from
    // where it started grows as the square root of its steps, so the second half of every soak was
    // re-crossing the first half. Weighting the draw away from the last two dozen actions taken
    // costs nothing, forbids nothing, stays exactly as seeded, and spends the same forty minutes
    // further out.
    let mut agent = PokemonAgent::new(Box::new(RandomPolicy::exploring(seed)));

    let target = MachineCycles::from_duration(game_time);
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
        // the buffer is unbounded and a long run would otherwise grow one event per decision for the
        // whole test.
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
            // ⚠️ **The screen is half the bug report.** Most of what `soak` finds is a menu or a
            // message the agent cannot leave, and the state name alone ("battle:wait") says which
            // driver was waiting but not what it was waiting *on*, which is the question every case
            // starts with. ⚠️ The artifact cannot always answer it either: a jam that lives in the
            // agent's own state does not reproduce from the save file, and then these lines are the
            // only evidence there is.
            let (screen, menu, buttons) = {
                let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
                (api.on_screen_text(false).map(|t| t.replace('\n', " ")), api.menu_state(),
                 format!("{:?}", api.read_joypad_state()))
            };
            let where_it_is = PokemonApi::with_cache(&mut gb, &mut cache).game_state().ok();
            // ⚠️ Named per state *and* per seed. The artifact is the whole value of a failure — it is
            // what gets promoted into `stalls` — and a hunt that runs every state under every seed
            // would otherwise have each failure overwrite the last one it found.
            let dir = std::path::Path::new("target/test-artifacts");
            std::fs::create_dir_all(dir).ok();
            let stem = dir.join(format!("soak-{}-seed{seed}", state.name));
            gb.save_state_to_file(&format!("{}.bin", stem.to_string_lossy())).ok();
            gb.save_screenshot_to_file(&format!("{}.png", stem.to_string_lossy())).ok();

            panic!(
                "the agent went {gap:?} of game time without reaching a decision point — \
                 a deployed LlmPolicy's watchdog would have fired here.\n\
                 \x20 from: {} ({})\n\
                 \x20 agent state: {}\n\
                 \x20 on screen: {}\n\
                 \x20 menu: {}\n\
                 \x20 buttons: {}\n\
                 \x20 where: {}\n\
                 \x20 after: {:?} of game time\n\
                 \x20 last {} events:\n{}\n\
                 \x20 reproduce: GB_SOAK_SEED={seed} cargo test --release --features soak-tests \
                    --bin gb -- soak::{} --nocapture\n\
                 \x20 artifacts: {}.{{bin,png}}",
                state.name, state.covers,
                agent.state_debug(),
                screen.as_deref().unwrap_or("(unreadable)"),
                menu.map_or_else(|| "(none)".to_string(),
                    |m| format!("{:?} item {} at ({},{})",
                                m.text_box_id, m.current_item, m.top_menu_item_x, m.top_menu_item_y)),
                buttons,
                where_it_is.as_ref().map_or_else(
                    || "unreadable".to_string(),
                    |s| format!("{} at {}", s.map.map, s.map.player_position)),
                emulated.to_duration(),
                tail.len(),
                tail.iter().map(|e| format!("    {e}")).collect::<Vec<_>>().join("\n"),
                state.name.replace('-', "_"),
                stem.to_string_lossy(),
            );
        }

        if emulated.to_duration() >= next_progress {
            next_progress += PROGRESS_EVERY;
            println!("[soak] {} {:?} — longest quiet stretch so far {worst:?} ({worst_state})",
                     state.name, emulated.to_duration());
        }
    }

    // The *emulated* total, not the constant it was asked for: a summary that prints its own target
    // back cannot tell you the loop exited early, which is exactly the failure it should surface.
    let ended = PokemonApi::with_cache(&mut gb, &mut cache).game_state().ok().map_or_else(
        || "unreadable".to_string(),
        |s| format!("{} at {}", s.map.map, s.map.player_position));
    println!("[soak] {} seed {seed}: {:?} of random play, ended in {ended}, longest quiet stretch \
              {worst:?} in state {worst_state:?} (the limit is {limit:?})",
             state.name, emulated.to_duration());
    assert!(worst < limit, "checked in the loop above");
}

/// A fresh save — the state `gb serve` starts a new run in, and the one both production jams were
/// found eight tiles from.
#[test]
fn random_play_from_a_fresh_save_never_goes_quiet() {
    soak(state("start-of-game"));
}

/// Viridian Forest: trainers, and the 8/256 grass that showed the pacing budget was three times too
/// generous.
#[test]
fn random_play_in_viridian_forest_never_goes_quiet() {
    soak(state("viridian-forest"));
}

/// Route 3's north-east pocket — **the deployed run's own save**, lifted off the pod, and the only
/// fixture here that did not come out of the leg chain.
///
/// The player is stood where a one-way ledge dropped them out of Route 4, against a map border whose
/// strip was misread twice over: the row came from the middle of Route 4, and cells were called
/// walkable if *any* of their four sub-tiles was. The crossing directly overhead was a ledge, it was
/// also the nearest one, and `actions()` only ever offers the nearest per map — so the only way into
/// Route 4 on the menu was one that could never be walked. The run picked it for a day.
///
/// ⚠️ **This is coverage, not the regression test.** A jam that aborts is not silence: the phantom
/// cost 60 s of `MAX_MOVEMENT_SILENCE` and then handed control back, which never approaches the
/// watchdog this tier watches, so a soak here would have passed throughout. What pins the bug is
/// `map_metadata`'s `test_a_ledge_across_a_border_is_not_a_connection` and its two neighbours. What
/// this buys is the region — connection strips, one-way ledges and a pocket that has to be routed
/// out of westwards — under a fuzzer, from a save that a real run really reached.
#[test]
fn random_play_in_route_3s_ledge_pocket_never_goes_quiet() {
    soak(state("route3-ledge-pocket"));
}

/// Vermilion: a gym behind a puzzle, a mart, a Pokémon Centre PC and the dock.
#[test]
fn random_play_around_vermilion_never_goes_quiet() {
    soak(state("at-vermilion"));
}

/// Lavender: the tower full of ghosts the player has no Silph Scope for, so every encounter is a
/// battle that cannot be fought normally.
#[test]
fn random_play_around_lavender_never_goes_quiet() {
    soak(state("at-lavender"));
}

/// Celadon: the department store, which is the deepest menu tree in the game.
#[test]
fn random_play_around_celadon_never_goes_quiet() {
    soak(state("at-celadon"));
}

/// The Rocket Hideout: spin tiles and a lift, i.e. movement the agent does not control.
#[test]
fn random_play_in_the_rocket_hideout_never_goes_quiet() {
    soak(state("at-rocket-hideout"));
}

/// Saffron: teleport pads in Silph Co and the gym's warp maze.
#[test]
fn random_play_around_saffron_never_goes_quiet() {
    soak(state("at-saffron"));
}

/// Cinnabar: surfing, the mansion, and the lab's trades.
#[test]
fn random_play_around_cinnabar_never_goes_quiet() {
    soak(state("at-cinnabar"));
}

/// Fuchsia and the Safari Zone: a step-limited mode with its own battle menu and no fighting.
#[test]
fn random_play_around_the_safari_zone_never_goes_quiet() {
    soak(state("post-safari"));
}

/// Victory Road: boulder switches, and floors that need Strength to leave.
#[test]
fn random_play_in_victory_road_never_goes_quiet() {
    soak(state("vr1f-strength"));
}

/// A PC with mons in the box — withdraw, deposit and release, none of which any other state here can
/// reach.
#[test]
fn random_play_with_a_full_pc_box_never_goes_quiet() {
    soak(state("postgame-pc-box"));
}

/// Bicycle and Fly: the whole map in reach, plus Cycling Road, where the game moves the player.
#[test]
fn random_play_with_a_bike_and_fly_never_goes_quiet() {
    soak(state("postgame-fly-bike"));
}

/// The postgame chain head: a full bag, a full party, every field move, and the largest action space
/// in the suite.
#[test]
fn random_play_in_the_postgame_never_goes_quiet() {
    soak(state("postgame-aides"));
}

/// Mt Moon: three floors of unlit cave, the fossil pair, and a Rocket in the way out.
#[test]
fn random_play_in_mt_moon_never_goes_quiet() {
    soak(state("soak-mt-moon"));
}

/// The S.S. Anne: two dozen identical cabin doors, and a ship that sails once the route is done with it.
#[test]
fn random_play_on_the_ss_anne_never_goes_quiet() {
    soak(state("soak-ss-anne"));
}

/// Rock Tunnel with no Flash — the walker chooses tiles it cannot see.
#[test]
fn random_play_in_rock_tunnel_never_goes_quiet() {
    soak(state("soak-rock-tunnel"));
}

/// Pokémon Tower *with* the Silph Scope, which is the other half of `at-lavender`: the ghosts fight back.
#[test]
fn random_play_in_pokemon_tower_never_goes_quiet() {
    soak(state("soak-pokemon-tower"));
}

/// Route 12: a road a sleeping Snorlax blocks, with a rod in the bag and water on both sides.
#[test]
fn random_play_at_the_snorlax_never_goes_quiet() {
    soak(state("soak-route12-snorlax"));
}

/// Inside the Safari Zone with the step counter running, rather than `post-safari`'s view from afterwards.
#[test]
fn random_play_inside_the_safari_zone_never_goes_quiet() {
    soak(state("soak-safari-zone"));
}

/// Silph Co mid-errand: teleport pads, a lift, and locked doors.
#[test]
fn random_play_inside_silph_co_never_goes_quiet() {
    soak(state("soak-silph-co"));
}

/// The Saffron Gym warp maze — every door is a warp and none of them is out.
#[test]
fn random_play_in_the_saffron_gym_never_goes_quiet() {
    soak(state("soak-saffron-gym"));
}

/// The Pokémon Mansion switches: walls that open somewhere the walker cannot see.
#[test]
fn random_play_in_the_pokemon_mansion_never_goes_quiet() {
    soak(state("soak-pokemon-mansion"));
}

/// The Cinnabar Gym quiz: a yes/no gate per door and a battle for each wrong answer.
#[test]
fn random_play_in_the_cinnabar_gym_never_goes_quiet() {
    soak(state("soak-cinnabar-gym"));
}

/// The Viridian Gym: spin tiles and warps, with seven badges already in hand.
#[test]
fn random_play_in_the_viridian_gym_never_goes_quiet() {
    soak(state("soak-viridian-gym"));
}

/// Route 23's badge-checking gates, the last thing between the route and Victory Road.
#[test]
fn random_play_at_the_victory_road_gates_never_goes_quiet() {
    soak(state("soak-route23"));
}

/// Every entry in [`SOAK_CHECKPOINTS`] is a soak state, cut on the map that list names.
///
/// The two lists cannot be one: the capture side needs `(name, Map)` in a module that is compiled
/// whatever features are on, and this side needs an `include_bytes!` per entry, which no const can
/// generate. So they are joined here instead — adding a checkpoint and forgetting to turn it loose
/// leaves a `.bin` in the repository that nothing reads, which is exactly the kind of thing that is
/// only ever noticed a year later.
#[test]
fn every_checkpoint_is_a_soak_state() {
    for (name, map) in SOAK_CHECKPOINTS {
        let found = STATES.iter().find(|s| s.name == *name)
            .unwrap_or_else(|| panic!("`{name}` is captured by regen_soak_checkpoints but no soak \
                                       state reads it — add one beside the others"));
        assert_eq!(found.expect_map, Some(*map),
                   "`{name}` is captured on {map} and its soak state expects {:?}",
                   found.expect_map);
    }
}

/// Every state in [`STATES`] is named by exactly one test, and every name resolves.
///
/// The list and the tests are separate on purpose — a test per state is what makes libtest run them
/// in parallel and report them individually — so this is the seam where an added state can be
/// forgotten. It costs nothing and it fails the moment the two drift apart.
#[test]
fn every_soak_state_is_named_by_a_test() {
    let source = include_str!("soak.rs");
    for s in STATES {
        assert!(source.matches(&format!("soak(state(\"{}\"))", s.name)).count() == 1,
                "no test drives the `{}` soak state — add one beside the others", s.name);
    }
    let names: std::collections::BTreeSet<_> = STATES.iter().map(|s| s.name).collect();
    assert_eq!(names.len(), STATES.len(), "two soak states share a name");
}
