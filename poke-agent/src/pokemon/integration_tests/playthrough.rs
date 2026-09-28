//! The whole game, start to finish, in one run.

use super::*;

/// Where on the eight-badge route [`super::soak`] starts, and the map each state is cut on.
#[cfg(feature = "slow-tests")]
pub(super) const SOAK_CHECKPOINTS: &[(&str, Map)] = &[
    ("soak-mt-moon", Map::MtMoonB2F),
    ("soak-ss-anne", Map::SSAnne1F),
    ("soak-rock-tunnel", Map::RockTunnel1F),
    ("soak-pokemon-tower", Map::PokemonTower5F),
    ("soak-route12-snorlax", Map::Route12),
    ("soak-safari-zone", Map::SafariZoneCenter),
    ("soak-silph-co", Map::SilphCo3F),
    ("soak-saffron-gym", Map::SaffronGym),
    ("soak-pokemon-mansion", Map::PokemonMansionB1F),
    ("soak-cinnabar-gym", Map::CinnabarGym),
    ("soak-viridian-gym", Map::ViridianGym),
    ("soak-route23", Map::Route23),
];

/// How long the run must stand on a checkpoint's map before the state is taken: one second of game
/// time.
#[cfg(feature = "slow-tests")]
const CHECKPOINT_SETTLE_TICKS: u32 = 50;

/// Re-cut every [`SOAK_CHECKPOINTS`] state by playing the eight-badge route once.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts every soak checkpoint; needs GB_REGEN_FIXTURES=1"]
fn regen_soak_checkpoints() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::eight_badge_steps(),
    );

    let mut written: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut visited: Vec<Map> = Vec::new();
    let mut standing_on: Option<(Map, u32)> = None;

    while !fixture.agent.policy_exhausted() {
        fixture.step();
        let Ok(state) = fixture.try_game_state() else { continue };
        let map = state.map.map;

        standing_on = match standing_on {
            Some((was, ticks)) if was == map => Some((map, ticks + 1)),
            _ => {
                if visited.last() != Some(&map) { visited.push(map); }
                Some((map, 0))
            }
        };
        let Some((_, ticks)) = standing_on else { continue };
        if ticks != CHECKPOINT_SETTLE_TICKS { continue }

        for (name, want) in SOAK_CHECKPOINTS {
            if *want != map || written.contains(name) { continue }
            let path = format!("src/pokemon/data/{name}.bin");
            fixture.gb.save_state_to_file(&path).expect("write a soak checkpoint");
            written.insert(name);
            println!("[checkpoint] {name} — {map} at {} ({:?} of game time, party {:?})",
                     state.map.player_position, fixture.total_cycles.to_duration(),
                     state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
        }
    }

    println!("\n[checkpoint] maps visited, in order:");
    for map in &visited { println!("    {map}"); }

    let missed: Vec<&str> = SOAK_CHECKPOINTS.iter()
        .map(|(name, _)| *name).filter(|name| !written.contains(name)).collect();
    assert!(missed.is_empty(),
            "the route never stood on the map(s) these checkpoints name: {missed:?} — pick \
             replacements from the visited list above, or drop them from SOAK_CHECKPOINTS");
    println!("[checkpoint] re-cut {} soak fixtures", written.len());
}

/// Resume [`full_playthrough`] from a stalled run's dropped save with its remaining steps queued.
/// Set `RESUME_QUEUE_LEN` to the number of steps it had left.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_resume_playthrough() {
    let Ok(bytes) = std::fs::read("target/test-artifacts/test_stall_state.bin") else {
        println!("no stall artifact — run full_playthrough first");
        return;
    };
    let all = PolicyStep::complete_game_steps();
    let remaining: usize = std::env::var("RESUME_QUEUE_LEN").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(all.len());
    let from = all.len().saturating_sub(remaining);
    println!("resuming at step {from} of {} ({remaining} queued)", all.len());
    for (i, step) in all.iter().skip(from).take(6).enumerate() {
        println!("  [{}] {step:?}", from + i);
    }

    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(800),
        all[from..].to_vec());
    let s = fixture.game_state();
    println!("resume state: {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    // A stall is usually about an item that was not delivered or an exit the pathfinder cannot see.
    println!("   bag[{}]: {:?}", s.bag.len(), s.bag.iter().map(|i| i.id).collect::<Vec<_>>());
    println!("   tile under player: {:?}", s.map.tile_at_checked(s.map.player_position));
    for sprite in &s.map.sprites {
        println!("   sprite {:?} hidden={} @ {}", sprite.name, sprite.hidden, sprite.position);
    }
    for action in s.map.actions() {
        println!("   action {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("resume ended: {} @ {} badges={:?}", s.map.map, s.map.player_position, s.badges);
}

/// The scripted route from a fresh save through all eight badges.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "full playthrough; run with --features slow-tests")]
fn full_playthrough() {
    // A gym fought by an HM carrier once the starter faints stays on one step through its trainers,
    // its black-outs and the regrown trees on the way back: longer than the default.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::eight_badge_steps(),
    ).with_stall_tolerance(Duration::from_mins(30));

    {
        let state = fixture.game_state();
        assert_eq!(state.map.map, Map::RedsHouse2F, "save state should be in RedsHouse2F");
        assert_eq!(state.pokemon.len(), 0, "player should have no pokemon before Oak's script");
    }

    let started = std::time::Instant::now();
    fixture.step_until_exhausted();
    let elapsed = started.elapsed();
    let state = fixture.game_state();

    println!("\nplayed {:?} of game time in {elapsed:?} of wall clock ({:.0}x realtime)",
             fixture.total_cycles.to_duration(), elapsed.as_secs_f64().max(0.001).recip()
                 * fixture.total_cycles.to_duration().as_secs_f64());

    for pokemon in state.pokemon.iter() {
        println!("{}: {} lv.{}", pokemon.species, pokemon.nickname, pokemon.level);
    }
    println!("badges: {:?}", state.badges);
    println!("map: {:?}", state.map.map);
    println!("money: {}  bag: {:?}", state.money, state.bag.iter().collect::<Vec<_>>());

    assert!(state.badges.contains(Badge::BoulderBadge), "should have the Boulder Badge");
    assert!(state.badges.contains(Badge::CascadeBadge), "should have the Cascade Badge");
    assert!(state.badges.contains(Badge::ThunderBadge), "should have the Thunder Badge");
    assert!(state.badges.contains(Badge::RainbowBadge), "should have the Rainbow Badge");
    // Post-Rainbow: Silph Scope, Poké Flute, Snorlax, Soul Badge.
    assert!(state.bag.contains(&ItemId::SilphScope), "should have the Silph Scope");
    assert!(state.bag.contains(&ItemId::PokeFlute), "should have the Poké Flute");
    assert!(state.badges.contains(Badge::SoulBadge), "should have the Soul Badge");
    // Post-Soul: Safari HMs, Vaporeon, Silph, Cinnabar Mansion, Volcano, Viridian.
    assert!(state.bag.contains(&ItemId::Hm03Surf), "should have HM03 Surf");
    assert!(state.badges.contains(Badge::MarshBadge), "should have the Marsh Badge");
    assert!(state.badges.contains(Badge::VolcanoBadge), "should have the Volcano Badge");
    assert!(state.badges.contains(Badge::EarthBadge), "should have the Earth Badge (all 8 gym badges)");

    // One fighter and two HM slaves, and nothing else.
    assert_eq!(state.pokemon.len(), 3, "party should be the starter + the two HM slaves");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Blastoise),
        "the starter should have reached Blastoise");
    assert!(state.pokemon.iter().any(|p| matches!(p.species,
        PokemonSpecies::Oddish | PokemonSpecies::Gloom)), "should have caught the Route 25 Cut carrier");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Machop),
        "should have caught the Victory Road Strength slave");

    // Every HM is checked on the party, since a step aimed at a carrier that cannot learn one waits
    // for ever: Cut on the Oddish, the rest on Blastoise.
    for want in [PokemonMoveName::Cut, PokemonMoveName::Surf, PokemonMoveName::Strength] {
        assert!(state.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| m.name == want)),
            "a party member should know {want}");
    }
    assert_eq!(state.map.map, Map::VictoryRoad2F, "should have solved VR1F and climbed to Victory Road 2F");

    fixture.save_state_named("src/pokemon/data/post-victory-road-1f.bin").unwrap();
}

/// The scripted route from a fresh save to the Boulder Badge on `options`, and the game time it took.
#[cfg(feature = "slow-tests")]
fn play_to_the_boulder_badge(options: crate::pokemon::options::GameOptions) -> Duration {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(240),
        PolicyStep::complete_game_steps(),
    ).with_options(options);
    let started = std::time::Instant::now();
    let state = fixture.run_until(|state| state.badges.contains(Badge::BoulderBadge));
    let played = fixture.total_cycles.to_duration();
    println!("\nBoulder Badge after {played:?} of game time, {:?} of wall clock; party {:?}",
             started.elapsed(), state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    played
}

/// [`full_playthrough`]'s opening with battle animations on, exactly what `--policy deterministic`
/// serves, from the fresh save to Brock.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "the route to Brock; run with --features slow-tests")]
fn full_playthrough_animated() {
    #[cfg(feature = "slow-tests")]
    play_to_the_boulder_badge(crate::pokemon::options::SERVED_OPTIONS);
}

/// What the served options save on the route to Brock against the cartridge's own.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "measurement — run with --ignored --nocapture"]
fn probe_served_options_to_brock() {
    use crate::pokemon::options::{BattleStyle, GameOptions, SERVED_OPTIONS, TextSpeed};
    let cartridge = GameOptions { battle_style: BattleStyle::Shift, text_speed: TextSpeed::Medium, ..SERVED_OPTIONS };
    let slow = play_to_the_boulder_badge(cartridge);
    let fast = play_to_the_boulder_badge(SERVED_OPTIONS);
    println!("MEDIUM/SHIFT {slow:?}, FAST/SET {fast:?}: {:.0}% less game time",
             100.0 * (1.0 - fast.as_secs_f64() / slow.as_secs_f64()));
}

/// [`PolicyStep::complete_game_steps`] to the Hall of Fame, as `--policy deterministic` plays it.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "~26 min — run with --features slow-tests")]
fn hall_of_fame_playthrough() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(6000),
        PolicyStep::complete_game_steps(),
    );

    let state = fixture.run_until(|state| state.map.map == Map::HallOfFame);
    let left = fixture.agent.policy_steps_remaining().expect("the scripted policy counts its queue");
    assert!(
        left <= 2,
        "{left} steps never ran — the run reached the Hall of Fame without playing the route",
    );

    for pokemon in state.pokemon.iter() {
        println!("{}: {} lv.{}", pokemon.species, pokemon.nickname, pokemon.level);
    }
    assert!(state.badges.contains(Badge::EarthBadge), "all eight badges");
    // One fighter over the target.
    for species in [PokemonSpecies::Blastoise] {
        let mon = state.pokemon.iter().find(|p| p.species == species)
            .unwrap_or_else(|| panic!("the party should carry a {species:?}"));
        assert!(mon.level >= PolicyStep::GAUNTLET_LEVEL,
            "{species:?} is lv{} — the gauntlet grind did not run", mon.level);
    }
}

/// A new game on the recreation, in Red's room with the preset names, with battle animations on
/// or off and the rest of the options as a served run plays them, at `pacing`.
pub(crate) fn native_new_game(seed: u64, battle_animation: bool, pacing: pokered::Pacing) -> pokered::Game {
    use pokered::command::{Command, Decision};
    use pokered::mode::{Mode, Status};
    use pokered::rng::GameRng;
    use pokered::world::{BattleStyle, Options, TextSpeed};
    use pokered::{Game, Input, Pacing};

    let mut game = Game::power_on(None, GameRng::seeded(seed), Pacing::Instant);
    for _ in 0..60_000 {
        if matches!(game.modes(), [Mode::Overworld(_)]) {
            let mut world = game.world().clone();
            world.options = Options { text_speed: TextSpeed::Fast, battle_animation, battle_style: BattleStyle::Set };
            let mut game = Game::new(world, GameRng::seeded(seed), pacing);
            game.push(Mode::Overworld(pokered::modes::overworld::Overworld::new()));
            return game;
        }
        let command = match game.status() {
            Status::Waiting(Decision::TitleScreen | Decision::Text) => Some(Command::Advance),
            Status::Waiting(Decision::MainMenu) => Some(Command::ChooseOption(0)),
            Status::Waiting(Decision::IntroNameMenu) => Some(Command::ChooseOption(1)),
            _ => None,
        };
        game.frame(command.map_or(Input::None, Input::Command));
    }
    panic!("the intro never reached Red's room: {:?}", game.status());
}

/// Where [`native_full_playthrough`] leaves the game it stalled in, for [`probe_native_stall`].
const NATIVE_STALL: &str = "target/test-artifacts/native_stall.pkrd";

/// [`full_playthrough`]'s route on the recreation, from a new game, through the native agent.
#[test]
fn native_full_playthrough() {
    play_native_full_playthrough(1);
}

/// The route on other seeds, which black out along the way and have to find their way back.
#[test]
#[cfg(feature = "slow-tests")]
fn native_full_playthrough_recovers_from_black_outs_on_other_seeds() {
    for seed in 2..=9 {
        play_native_full_playthrough(seed);
    }
}

fn play_native_full_playthrough(seed: u64) {
    use crate::pokemon::native_agent::NativeAgent;

    let game = native_new_game(seed, false, pokered::Pacing::Instant);
    assert_eq!(game.world().location.map, Map::RedsHouse2F);
    let policy = DeterministicPolicy::new(seed, PolicyStep::eight_badge_steps());
    let mut agent = NativeAgent::new(game, Box::new(policy)).expect("a native agent");

    // An hour of game time with the queue standing still is a stall.
    const STALL_FRAMES: u64 = 60 * 60 * 60;
    let mut left = agent.policy().steps_remaining();
    let (mut frame, mut moved_at) = (0u64, 0u64);
    while !agent.policy().is_exhausted() {
        frame += 1;
        let ticked = agent.tick();
        if agent.policy().steps_remaining() != left {
            left = agent.policy().steps_remaining();
            moved_at = frame;
        }
        let stalled = frame - moved_at > STALL_FRAMES;
        if ticked.is_err() || stalled {
            std::fs::create_dir_all("target/test-artifacts").ok();
            std::fs::write(NATIVE_STALL, agent.game().save()).ok();
            panic!("seed {seed}: {} with {left:?} steps left, on {:?}, {:?}",
                   ticked.err().unwrap_or_else(|| "stalled for an hour of game time".into()),
                   agent.game().world().location, agent.game().status());
        }
    }

    let state = agent.game_state().expect("a game state");
    println!("\nplayed {frame} frames");
    for pokemon in state.pokemon.iter() {
        println!("{}: {} lv.{}", pokemon.species, pokemon.nickname, pokemon.level);
    }
    for badge in [Badge::BoulderBadge, Badge::CascadeBadge, Badge::ThunderBadge, Badge::RainbowBadge,
                  Badge::SoulBadge, Badge::MarshBadge, Badge::VolcanoBadge, Badge::EarthBadge] {
        assert!(state.badges.contains(badge), "should have the {badge:?}");
    }
    assert!(state.bag.contains(&ItemId::SilphScope), "should have the Silph Scope");
    assert!(state.bag.contains(&ItemId::PokeFlute), "should have the Poké Flute");
    assert_eq!(state.pokemon.len(), 3, "party should be the starter + the two HM slaves");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Blastoise),
        "the starter should have reached Blastoise");
    for want in [PokemonMoveName::Cut, PokemonMoveName::Surf, PokemonMoveName::Strength] {
        assert!(state.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| m.name == want)),
            "a party member should know {want}");
    }
    assert_eq!(state.map.map, Map::VictoryRoad2F, "should have solved VR1F and climbed to Victory Road 2F");
}

/// What [`native_full_playthrough`]'s saved stall is doing: its mode stack over `PROBE_FRAMES`
/// frames, then the rows the agent would be offered.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "probe — run with --ignored --nocapture after a native stall"]
fn probe_native_stall() {
    let path = std::env::var("NATIVE_STALL").unwrap_or_else(|_| NATIVE_STALL.to_string());
    let Ok(bytes) = std::fs::read(&path) else {
        println!("no native stall artifact — run native_full_playthrough first");
        return;
    };
    let mut game = pokered::Game::load(&bytes, pokered::Pacing::Instant).expect("a native save");
    let frames: u32 = std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    for _ in 0..frames {
        let location = &game.world().location;
        println!("{:?} on {:?} at ({}, {}) facing {:?}", game.status(), location.map, location.x, location.y, location.facing);
        for mode in game.modes() {
            println!("    {mode:#?}");
        }
        game.frame(pokered::Input::None);
    }
    if let Ok(steps) = std::env::var("PROBE_STEPS") {
        for step in steps.split(',') {
            let direction = match step {
                "Up" => pokered::systems::overworld::location::Direction::Up,
                "Down" => pokered::systems::overworld::location::Direction::Down,
                "Left" => pokered::systems::overworld::location::Direction::Left,
                _ => pokered::systems::overworld::location::Direction::Right,
            };
            let reply = game.frame(pokered::Input::Command(pokered::command::Command::Step(direction))).reply;
            for _ in 0..60 {
                game.frame(pokered::Input::None);
            }
            let location = &game.world().location;
            println!("step {step}: {reply:?}, now at ({}, {}) on {:?}", location.x, location.y, location.map);
        }
    }
    let state = crate::pokemon::native::NativeGame::new(game).and_then(|native| native.game_state()).expect("a game state");
    println!("party {:?}", state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    println!("bag {:?}", state.bag.iter().map(|item| (item.id, item.quantity)).collect::<Vec<_>>());
    for sprite in &state.map.sprites {
        println!("sprite {sprite:?}");
    }
    if std::env::var("PROBE_MAP").is_ok() {
        println!("{}", state.map);
        println!("can_surf {} surfing {}", state.map.can_surf, state.map.surfing);
        if let Ok(target) = std::env::var("PROBE_FACE") {
            let (x, y) = target.split_once(',').expect("x,y");
            let at = poke_core::geometry::Point8 { x: x.parse().unwrap(), y: y.parse().unwrap() };
            println!("route to face {at}: {:?}", state.map.route_to_face(at));
        }
    }
    for action in state.map.actions() {
        println!("row {} {:?}", action.tile, action.route);
    }
}
