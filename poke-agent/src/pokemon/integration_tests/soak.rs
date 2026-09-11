//! The stall hunt: random play from two dozen points in the game, watching for the agent to stop
//! asking the policy anything.

use super::*;
use crate::pokemon::options::{BattleStyle, GameOptions, TextSpeed};
use crate::pokemon::policy::RandomPolicy;
use super::playthrough::SOAK_CHECKPOINTS;

/// How much game time one soak covers, per starting state.
const SOAK_GAME_TIME: Duration = Duration::from_secs(40 * 60);

/// How often to print, in game time.
const PROGRESS_EVERY: Duration = Duration::from_secs(10 * 60);

/// Events kept for the failure report.
const EVENT_TAIL: usize = 12;

/// The seed the fuzzer plays by default.
/// ```shell
/// for seed in $(seq 1 20); do
///   GB_SOAK_SEED=$seed cargo test --release --features slow-tests --lib -- soak --nocapture
/// done
/// ```
const DEFAULT_SEED: u64 = 1;

/// The options `poke-agent-web` plays on, pokered's `InitOptions`: animations on and SHIFT style, which
/// no other tier sees.
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
    /// What this state puts within reach that the others do not.
    covers: &'static str,
    /// Where the capture is supposed to have been taken, asserted on the way in.
    expect_map: Option<Map>,
}

/// Where the fuzzer starts, chosen for what is reachable from each, not for progression.
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

    // ── Checkpoints cut off the mainline by `playthrough::regen_soak_checkpoints` ──
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

/// Look a state up by name, so each test names its fixture rather than an index.
fn state(name: &str) -> &'static SoakState {
    STATES.iter().find(|s| s.name == name).expect("every soak test names a state in STATES")
}

/// A `u64` from the environment, or `default` when unset or unparseable.
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// Turn `state` loose under `RandomPolicy` and fail if the agent goes quiet for longer than the
/// watchdog allows.
fn soak(state: &SoakState) {
    // `GB_SOAK_LIMIT_SECS` tightens the net below the watchdog's.
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
    // Nobody listens to a jam hunt, so the APU does not mix or resample.
    gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
    let mut cache = MapMetadataCache::default();
    // The deployment's options, not `FAST_FIXTURE_OPTIONS`.
    PokemonApi::with_cache(&mut gb, &mut cache).debug_set_options(&DEPLOYMENT_OPTIONS);
    if let Some(want) = state.expect_map {
        let on = PokemonApi::with_cache(&mut gb, &mut cache).game_state().map(|s| s.map.map);
        assert_eq!(on.ok(), Some(want),
                   "the `{}` checkpoint is not on {want} any more — re-cut it with \
                    `cargo test --release --features slow-tests --lib -- \
                    pokemon::integration_tests::playthrough::regen_soak_checkpoints --exact`",
                   state.name);
    }
    // `exploring`, the recency-weighted draw argued on `RandomPolicy::exploring`.
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
        // `update` reports transient read errors during map transitions; a wedged agent shows up as
        // silence either way.
        agent.update(&mut api, ran).ok();

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
            let (screen, menu, buttons) = {
                let api = PokemonApi::with_cache(&mut gb, &mut cache);
                (api.on_screen_text(false).map(|t| t.replace('\n', " ")), api.menu_state(),
                 format!("{:?}", api.read_joypad_state()))
            };
            let where_it_is = PokemonApi::with_cache(&mut gb, &mut cache).game_state().ok();
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
                 \x20 reproduce: GB_SOAK_SEED={seed} cargo test --release --features slow-tests \
                    --lib -- soak::{} --nocapture\n\
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

    let ended = PokemonApi::with_cache(&mut gb, &mut cache).game_state().ok().map_or_else(
        || "unreadable".to_string(),
        |s| format!("{} at {}", s.map.map, s.map.player_position));
    println!("[soak] {} seed {seed}: {:?} of random play, ended in {ended}, longest quiet stretch \
              {worst:?} in state {worst_state:?} (the limit is {limit:?})",
             state.name, emulated.to_duration());
    assert!(worst < limit, "checked in the loop above");
}

/// A fresh save, the state `poke-agent-web` starts a new run in.
#[test]
fn random_play_from_a_fresh_save_never_goes_quiet() {
    soak(state("start-of-game"));
}

/// Viridian Forest: trainers, and 8/256 grass.
#[test]
fn random_play_in_viridian_forest_never_goes_quiet() {
    soak(state("viridian-forest"));
}

#[test]
fn random_play_in_route_3s_ledge_pocket_never_goes_quiet() {
    soak(state("route3-ledge-pocket"));
}

/// Vermilion: a gym behind a puzzle, a mart, a Pokémon Centre PC and the dock.
#[test]
fn random_play_around_vermilion_never_goes_quiet() {
    soak(state("at-vermilion"));
}

/// Lavender: the tower's ghosts with no Silph Scope, so no encounter can be fought normally.
#[test]
fn random_play_around_lavender_never_goes_quiet() {
    soak(state("at-lavender"));
}

/// Celadon: the department store, the deepest menu tree in the game.
#[test]
fn random_play_around_celadon_never_goes_quiet() {
    soak(state("at-celadon"));
}

/// The Rocket Hideout: spin tiles and a lift, movement the agent does not control.
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

/// A PC with mons in the box: withdraw, deposit and release.
#[test]
fn random_play_with_a_full_pc_box_never_goes_quiet() {
    soak(state("postgame-pc-box"));
}

/// Bicycle and Fly: the whole map in reach, plus Cycling Road, where the game moves the player.
#[test]
fn random_play_with_a_bike_and_fly_never_goes_quiet() {
    soak(state("postgame-fly-bike"));
}

/// The postgame chain head: a full bag and party, every field move, the largest action space.
#[test]
fn random_play_in_the_postgame_never_goes_quiet() {
    soak(state("postgame-aides"));
}

/// Mt Moon: three floors of unlit cave, the fossil pair, and a Rocket in the way out.
#[test]
fn random_play_in_mt_moon_never_goes_quiet() {
    soak(state("soak-mt-moon"));
}

/// The S.S. Anne: identical cabin doors, on a map that stops existing once the route leaves it.
#[test]
fn random_play_on_the_ss_anne_never_goes_quiet() {
    soak(state("soak-ss-anne"));
}

/// Rock Tunnel with no Flash: the walker chooses tiles it cannot see.
#[test]
fn random_play_in_rock_tunnel_never_goes_quiet() {
    soak(state("soak-rock-tunnel"));
}

/// Pokémon Tower with the Silph Scope, the other half of `at-lavender`: the ghosts fight back.
#[test]
fn random_play_in_pokemon_tower_never_goes_quiet() {
    soak(state("soak-pokemon-tower"));
}

/// Route 12: a road a sleeping Snorlax blocks, with a rod in the bag and water on both sides.
#[test]
fn random_play_at_the_snorlax_never_goes_quiet() {
    soak(state("soak-route12-snorlax"));
}

/// Inside the Safari Zone with the step counter running.
#[test]
fn random_play_inside_the_safari_zone_never_goes_quiet() {
    soak(state("soak-safari-zone"));
}

/// Silph Co mid-errand: teleport pads, a lift, and locked doors.
#[test]
fn random_play_inside_silph_co_never_goes_quiet() {
    soak(state("soak-silph-co"));
}

/// The Saffron Gym warp maze: every door is a warp and none is the way out.
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
