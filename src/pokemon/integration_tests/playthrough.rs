//! The whole game, start to finish, in one run.

use super::*;

/// Resume [`full_playthrough`] from the save state a stalled run drops in `target/test-artifacts/`,
/// with the steps it had left still queued — so a stall 270 steps in can be re-tested in seconds
/// instead of re-running the whole 20 minutes up to it.
///
/// A save state carries the emulator's RNG registers, so resuming continues the *same* stream the run
/// was on. That is what makes this valid for chasing a route bug and **invalid as a substitute for the
/// real run**: it proves a fix works from that point, not that the run still reaches that point.
/// Always finish with a clean `full_playthrough`.
///
/// `RESUME_QUEUE_LEN` is the `queue_len=` from the last `[policy]` line of the stalled run.
/// ```text
/// RESUME_QUEUE_LEN=233 cargo test --release --features full-playthrough --bin gb -- \
///   probe_resume_playthrough --exact --ignored --nocapture
/// ```
#[test]
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
    // The bag and the reachable set are the two things a stall is usually *about*: an item a gift or a
    // purchase silently failed to deliver, or an exit the pathfinder cannot see from where it stands.
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

/// The full end-to-end playthrough — the single source of truth for how far the agent can play. From a
/// fresh `RedsHouse2F` save it plays legitimately (button input only, starting from the **lone starter**)
/// and earns **all 8 gym badges**: Boulder → Cascade → Thunder → Rainbow → (Silph Scope → Poké Flute →
/// Snorlax) → Soul → (Safari Surf/Strength → Eevee→Vaporeon+Surf) → Silph Co (Card Key → rival → Giovanni
/// → liberation) → Marsh → surf to Cinnabar → Pokémon Mansion Secret Key → Volcano → back to the Viridian
/// Gym for **Earth** (Giovanni), with the **Seafoam Islands** detour for Articuno slotted in between
/// Volcano and Earth.
///
/// Nothing is caught before then: the run is carried by the starter and the free Celadon **Eevee**,
/// evolved to **Vaporeon** (its Surf counters the Silph rival's Alakazam / Blaine's Fire / Giovanni's
/// Ground, and ferries the party across Route 21), because a weak extra mon breaks the black-out
/// recovery the early dungeons rely on. Seafoam then adds a **Slowpoke** HM-slave (Strength + Dig) and
/// **Articuno**.
///
/// It emulates every frame, so even in `--release` it takes ~20 min of wall clock — hence its own
/// feature gate, separate from the leg chain:
/// `cargo test --release --features full-playthrough full_playthrough`. The per-leg tests (each seeded
/// from a saved fixture) cover the same ground quickly and in parallel.
///
/// ⚠️ **Run this after every major work item and always before pushing** (see CLAUDE.md). It is the
/// only test that proves the legs *compose*; the leg tier proves each leg from a committed fixture and
/// is systematically blind to three things — a leg that only passes because `run_leg` kept stepping
/// after its queue emptied, a fixture that hands a leg a party or a bag the run could not actually
/// have earned, and any change to frame timing re-rolling the RNG stream every route is tuned against.
///
/// ⚠️ **Its end point is Victory Road 2F, not the Hall of Fame** — `victory_road_2f_3f_steps` and
/// `elite_four_steps` are deliberately excluded as PP-marginal for this team (see their notes) and are
/// proved separately by `endgame::can_solve_victory_road_2f_3f` and `endgame::can_beat_elite_four`.
/// Saying otherwise is how this test came to be believed green while it was not: it sat broken for a
/// long time behind doc comments — this one included — claiming it played to the Hall of Fame.
#[test]
#[cfg_attr(not(feature = "full-playthrough"), ignore = "full playthrough; run with --features full-playthrough")]
fn full_playthrough() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::complete_game_steps(),
    );

    {
        let state = fixture.game_state();
        assert_eq!(state.map.map, Map::RedsHouse2F, "save state should be in RedsHouse2F");
        assert_eq!(state.pokemon.len(), 0, "player should have no pokemon before Oak's script");
    }

    // ⚠️ **`step_until_exhausted`, never `run_leg`.** The queue emptying is the *only* thing this test
    // is allowed to accept as "the run finished". `run_leg` would keep stepping afterwards until the
    // assertions happened to come true, which turns "the step list plays the game" into "the step list
    // plus whatever the agent does on its own eventually gets there" — and that is precisely the hole
    // the Poké Flute fell through for a long time (see `TestFixture::run_leg`). Everything below has to
    // be true the instant the last step pops.
    fixture.step_until_exhausted();

    let state = fixture.game_state();
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
    // Post-Rainbow: Silph Scope (Rocket Hideout) → Poké Flute → Snorlax → Soul Badge (Koga).
    assert!(state.bag.contains(&ItemId::SilphScope), "should have the Silph Scope");
    assert!(state.bag.contains(&ItemId::PokeFlute), "should have the Poké Flute");
    assert!(state.badges.contains(Badge::SoulBadge), "should have the Soul Badge");
    // Post-Soul: Safari HMs → Vaporeon → Silph (Marsh) → Cinnabar Mansion → Volcano → Viridian (Earth).
    assert!(state.bag.contains(&ItemId::Hm03Surf), "should have HM03 Surf");
    assert!(state.badges.contains(Badge::MarshBadge), "should have the Marsh Badge");
    assert!(state.badges.contains(Badge::VolcanoBadge), "should have the Volcano Badge");
    assert!(state.badges.contains(Badge::EarthBadge), "should have the Earth Badge (all 8 gym badges)");

    // The starter and the one free Eevee (evolved to Vaporeon for Surf) carry the whole run — nothing
    // is caught before Seafoam, because a weak extra mon blocks the black-out recovery that clears the
    // early attrition dungeons. Seafoam then adds the two the Elite Four needs: a Slowpoke HM-slave
    // (Strength for Victory Road, Dig for the way out of the islands) and Articuno.
    assert!(state.pokemon.len() >= 4, "party should have the starter + Vaporeon + the Slowpoke slave + Articuno");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Vaporeon), "should have a Vaporeon");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Articuno),
        "should have caught Articuno in the Seafoam Islands");

    // Post-Earth: Victory Road 1F — caught a Machop HM-slave, taught it Strength, solved the boulder
    // puzzle (a real push onto the (17,13) switch) and climbed to VR2F. The full VR2F/VR3F puzzle and
    // the Elite Four are validated separately (`endgame::can_solve_victory_road_2f_3f`,
    // `endgame::can_beat_elite_four`) — chaining them here is PP-marginal for this team.
    assert!(state.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Strength)),
        "a party member should know Strength (the Seafoam Slowpoke, and the Victory Road Machop)");
    assert_eq!(state.map.map, Map::VictoryRoad2F, "should have solved VR1F and climbed to Victory Road 2F");

    fixture.save_state_named("src/pokemon/data/post-victory-road-1f.bin").unwrap();
}
