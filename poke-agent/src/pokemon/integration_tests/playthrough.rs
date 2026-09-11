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
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::eight_badge_steps(),
    );

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
