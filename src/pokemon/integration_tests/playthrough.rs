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
#[cfg(feature = "diagnostics")]
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
/// fresh `RedsHouse2F` save it plays legitimately (button input only, starting from a **Squirtle**) and
/// earns **all 8 gym badges**: Boulder → (Nugget Bridge → Bill → ) Cascade → Thunder → Rainbow →
/// (Silph Scope → Poké Flute → Snorlax) → Soul → (Safari Surf/Strength) → Silph Co (Card Key → rival →
/// Giovanni → liberation) → Marsh → surf to Cinnabar → Pokémon Mansion Secret Key → Volcano → back to
/// the Viridian Gym for **Earth** (Giovanni), with the **Seafoam Islands** detour for Articuno slotted
/// in between Volcano and Earth.
///
/// It catches exactly two things on the way and neither of them fights: an **Oddish** on Route 25 to
/// carry Cut, and a **Machop** on Victory Road to carry Strength. Blastoise does everything else, with
/// Surf, Blizzard and Dig — Surf is the one HM on it, because Surf is a 95-power STAB attack that
/// happens to be an HM.
///
/// It emulates every frame, so even in `--release` it takes ~7 min of wall clock — hence its own
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
/// ⚠️ **Its end point is Victory Road 2F, and that is a cost decision rather than a limitation.**
/// The route `gb serve --policy deterministic` plays goes all the way to the Hall of Fame — see
/// [`PolicyStep::complete_game_steps`] and `hall_of_fame_playthrough`, which is the same run carried
/// through the gauntlet grind, both Victory Road puzzles and the Elite Four. That takes **~26 min**
/// against this one's seven, most of it the grind's ~840 wild battles, so it lives behind its own
/// `hall-of-fame` feature and this stays the gate you run before pushing.
///
/// ⚠️ **This doc comment has lied before, so keep it honest.** The test sat broken for a long time
/// while this very paragraph claimed it reached the Hall of Fame. It does not, on purpose, and the
/// assertion at the bottom is what says where it does stop — not this.
#[test]
#[cfg_attr(not(feature = "full-playthrough"), ignore = "full playthrough; run with --features full-playthrough")]
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

    // ⚠️ **`step_until_exhausted`, never `run_leg`.** The queue emptying is the *only* thing this
    // test is allowed to accept as "the run finished". `run_leg` would keep stepping afterwards until
    // the assertions happened to come true, which turns "the step list plays the game" into "the step
    // list plus whatever the agent does on its own eventually gets there" — and that is precisely the
    // hole the Poké Flute fell through for a long time (see `TestFixture::run_leg`). Everything below
    // has to be true the instant the last step pops.
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

    // ⚠️ **One fighter and two HM slaves, and the slaves are the *only* other members.** Measured at
    // the end of a clean run: Blastoise 59, Gloom 21 (the Oddish evolves on the way), Machop 24, all
    // eight badges and **no black-outs at all**. Anything else in the party is a regression — the
    // whole point of `gauntlet_grind_steps` is that one mon taken further is cheaper than three.
    assert_eq!(state.pokemon.len(), 3, "party should be the starter + the two HM slaves");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Blastoise),
        "the starter should have reached Blastoise");
    assert!(state.pokemon.iter().any(|p| matches!(p.species,
        PokemonSpecies::Oddish | PokemonSpecies::Gloom)), "should have caught the Route 25 Cut carrier");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Machop),
        "should have caught the Victory Road Strength slave");

    // ⚠️ **Every HM this route needs, checked on the *party* rather than the bag**, because a carrier
    // that cannot learn one is exactly the failure the starter swap introduced: Cut lives on the
    // Oddish and Surf, Strength and Dig on Blastoise, and a step aimed at the wrong one waits for
    // ever rather than failing.
    for want in [PokemonMoveName::Cut, PokemonMoveName::Surf, PokemonMoveName::Strength] {
        assert!(state.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| m.name == want)),
            "a party member should know {want}");
    }
    assert_eq!(state.map.map, Map::VictoryRoad2F, "should have solved VR1F and climbed to Victory Road 2F");

    fixture.save_state_named("src/pokemon/data/post-victory-road-1f.bin").unwrap();
}

/// **The whole game, to the Hall of Fame** — [`PolicyStep::complete_game_steps`], which is what
/// `gb serve --policy deterministic` plays.
///
/// ⚠️ **Its own `hall-of-fame` feature because it is ~26 minutes against `full_playthrough`'s seven,
/// and a gate that long is a gate nobody runs.** Most of the difference is `gauntlet_grind_steps`:
/// ~840 wild battles in the Pokémon Mansion to bring one Blastoise to lv85. Run this when the endgame
/// changes — the grind, either Victory Road puzzle, or the Elite Four — and `full_playthrough` the
/// rest of the time.
///
/// ⚠️ **It has been fifty minutes twice, and both times the cause was the grind rather than the
/// route.** Once because a trainee was switched in rather than *leading*, which halves the payout and
/// costs the turn; and once because it was grinding three Pokémon to lv75 (1.4 M experience) instead
/// of one to lv85 (425 k). The whole argument is on `PolicyStep::gauntlet_grind_steps`.
///
/// ⚠️ **`run_until`, not `step_until_exhausted`, and this is the one test allowed that.** The final
/// step is the rival in the Champion's room, and beating him hands the agent to
/// `drive_post_champion_cutscene`, which stops polling the policy — so the queue never empties and
/// waiting on it would hang. The exception is kept honest by asserting afterwards that all but the
/// **last two** steps popped, so "reached the Hall of Fame" cannot be satisfied by the agent
/// wandering into the credits on its own.
///
/// ⚠️ **Two rather than one, and the difference is a frame-timing race rather than a step that did
/// not happen.** The bound was `<= 1` — the rival's own `BattleTrainer` — and that only held while
/// the agent happened to get one overworld poll between arriving in the Champion's room and the
/// rival challenging. He challenges *on entry*, as a script, so there is nothing to guarantee that
/// poll, and the `enter(ChampionsRoom)` in front of him pops on the tick after the map changes or
/// not at all. A run that took the room faster stopped getting it: the log shows Gary beaten, the
/// Champion's script played and the Hall of Fame reached with both steps still queued. It is the
/// same mechanism the paragraph above describes for the last step, one step earlier. Out of 516 the
/// guard is unchanged in what it is for.
#[test]
#[cfg_attr(not(feature = "hall-of-fame"), ignore = "~26 min — run with --features hall-of-fame")]
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
    // ⚠️ **One fighter over the target, and it replaced "three fighters or you lose".** That rule
    // was true of three mons at *seventy-five*, with a lv26, a lv30 and a lv24 behind them; height
    // turned out to be the answer rather than depth of bench. See
    // `PolicyStep::gauntlet_grind_steps`.
    for species in [PokemonSpecies::Blastoise] {
        let mon = state.pokemon.iter().find(|p| p.species == species)
            .unwrap_or_else(|| panic!("the party should carry a {species:?}"));
        assert!(mon.level >= PolicyStep::GAUNTLET_LEVEL,
            "{species:?} is lv{} — the gauntlet grind did not run", mon.level);
    }
}
