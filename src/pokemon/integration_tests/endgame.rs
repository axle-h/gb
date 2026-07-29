//! Earth Badge → Victory Road → the Elite Four → the Hall of Fame.

use super::*;

/// From `post-volcano-lone.bin` (in Blaine's gym after the Volcano Badge, the full-playthrough party —
/// Venusaur + Vaporeon, 7 badges): Surf back to Pallet and up to Viridian, then clear Giovanni's
/// **Viridian Gym** spinner-tile maze for the **Earth Badge**, the 8th and final gym badge. Exercises
/// the `ViridianGym` arrow-tile table.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_earth_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-volcano-lone.bin"),
        Duration::from_mins(40),
        PolicyStep::earth_badge_steps(),
    );
    let s = fixture.run_until(|s| s.badges.contains(Badge::EarthBadge));
    println!("on {} @ {} — badges = {:?}", s.map.map, s.map.player_position, s.badges);
    fixture.save_state_named("src/pokemon/data/post-earth-badge.bin").unwrap();
}

/// Victory Road 1F: reach the cave, catch a wild **Machop** with the Master Ball as a Strength
/// HM-slave, teach it HM04, then push a boulder onto the (17,13) switch to open the (1,1) ladder and
/// climb to VR2F. This is the half that is folded into `complete_game_steps`.
///
/// **Known failure.** The Machop *is* caught and lands at slot 2 as the step list expects, but the
/// following `TeachMove { item: Hm04Strength, target_slot: 2 }` never completes — it re-enters the bag
/// menu forever, and the run then walks into "This requires STRENGTH to move!" at every boulder until
/// the cycle budget fires. `vermilion::can_teach_cut` passes teaching HM01 from bag index 7, so the
/// suspicion is the item-menu **scrolling** for an HM deeper in the bag (HM04 sits at index 11 of 16
/// here); `saffron::can_get_vaporeon` wedges the same way on HM03 at index 12. Note the committed
/// `vr1f-strength.bin` proves the leg worked at some point, so this is a regression, not a dead end.
#[test]
#[ignore = "TeachMove(HM04, slot 2) never completes — suspect deep-bag item-menu scrolling; see the doc comment"]
fn can_solve_victory_road_1f() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-earth-badge.bin"),
        Duration::from_mins(120),
        PolicyStep::victory_road_1f_steps(2),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    let has_strength = s.pokemon.iter()
        .any(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Strength));
    println!("final: {} @ {}  strength={has_strength}", s.map.map, s.map.player_position);
    assert!(has_strength, "should have caught + taught a Strength HM-slave");
    assert_eq!(s.map.map, Map::VictoryRoad2F, "should solve VR1F and climb to VR2F");
    fixture.save_state_named("src/pokemon/data/vr1f-strength.bin").unwrap();
}

/// The interconnected VR2F/VR3F Strength puzzle, through to the Indigo Plateau lobby: switch1 → 3F →
/// hole-drop reveals the hidden 2F boulder → fall → switch2 → return trip → exit. Every boulder is a
/// real Strength push.
///
/// Self-contained from `vr1f-strength.bin` (Machop HM-slave already caught and taught) because
/// chaining it onto a *fresh* run is PP-marginal — Victory Road's ~9 mandatory trainers plus the
/// Route-22 rival drain the lead past its damaging PP in some RNG lines and there is no Pokémon Center
/// inside. That is why `victory_road_2f_3f_steps` is not in `complete_game_steps`, and why this proof
/// lives here instead.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_solve_victory_road_2f_3f() {
    let mut steps = vec![
        PolicyStep::UseStrength { slot: 2 },
        PolicyStep::SolveBoulders { switch: Point8 { x: 17, y: 13 } },
        PolicyStep::enter(Map::VictoryRoad2F),
    ];
    steps.extend(PolicyStep::victory_road_2f_3f_steps(2));
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr1f-strength.bin"),
        Duration::from_mins(60),
        steps,
    );
    let s = fixture.run_until(|s| s.map.map == Map::IndigoPlateauLobby);
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    // Snapshot the Indigo Plateau lobby for fast Elite Four iteration.
    fixture.save_state_named("src/pokemon/data/at-indigo.bin").unwrap();
}

/// The Elite Four gauntlet, from the Indigo Plateau lobby to the credits: stock up, heal, then
/// Lorelei → Bruno → Agatha → Lance → the rival, and on through Oak's post-Champion script into the
/// **Hall of Fame**.
///
/// The two leads are looked up **by species**, not hardcoded: `MovePokemonToFront` rotates the party,
/// so where Articuno sits after Venusaur is pulled to the front depends on where both started — and a
/// grinded fixture arrives in a different order from an ungrinded one.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_beat_elite_four() {
    const FIXTURE: &[u8] = include_bytes!("../data/at-indigo-articuno.bin");

    let (venusaur, articuno_after) = {
        let mut probe = TestFixture::new(FIXTURE, Duration::from_mins(1), vec![]);
        let party = probe.game_state().pokemon;
        let idx = |species| party.iter().position(|p| p.species == species);
        let v = idx(PokemonSpecies::Venusaur).expect("fixture should carry a Venusaur") as u8;
        let a = idx(PokemonSpecies::Articuno).expect("fixture should carry an Articuno") as u8;
        // After `move_to_front(v)`, anything that sat before v shifts one later.
        (v, if a < v { a + 1 } else { a })
    };
    println!("leads: Venusaur slot {venusaur}, Articuno slot {articuno_after} after the rotation");

    // 180 min covers the five rooms plus Oak's post-Champion speech and the walk to the Hall of Fame.
    let mut fixture = TestFixture::new(
        FIXTURE,
        Duration::from_mins(180),
        PolicyStep::elite_four_steps(venusaur, articuno_after),
    );

    // The rival's battle starts from a map script rather than from a step, and once it is won the
    // agent hands itself to `drive_post_champion_cutscene`, which stops polling the policy — so the
    // last steps stay queued and "done" is never an empty queue.
    fixture.run_until(|s| s.map.map == Map::ChampionsRoom);
    const SCRIPT_OAK_ARRIVES: u8 = 4;
    while fixture.gb.core().mmu().read_pointer(&pokered_symbols::wChampionsRoomCurScript) < SCRIPT_OAK_ARRIVES {
        fixture.step();
    }
    // Bank the moment of victory: everything past here is Oak's script chain, and iterating on that
    // from a snapshot takes seconds instead of re-fighting five rooms.
    fixture.save_state_named("src/pokemon/data/post-champion.bin").unwrap();

    let s = fixture.run_until(|s| s.map.map == Map::HallOfFame);
    println!("HALL OF FAME — final team:");
    for p in s.pokemon.iter() { println!("  {:?} lv{} {}/{}hp", p.species, p.level, p.current_hp, p.stats.hp); }
    fixture.save_state_named("src/pokemon/data/post-hall-of-fame.bin").unwrap();
}

/// The post-Champion cutscene on its own, from `post-champion.bin` (rival beaten, Oak about to walk
/// in) to the credits — no policy steps at all, because `drive_post_champion_cutscene` drives it.
///
/// Oak's congratulation, his aside about the rival, his "come with me", his exit and the player
/// following him are five map-script stages, each gated on a text box, and the agent used to wedge at
/// stage 6. The fix was an **early return** rather than a press cadence: the agent's ordinary per-tick
/// machinery makes stray `release_all_buttons` calls as it changes state, and because `toggle_button`
/// flips relative to the *current* joypad, a release landing between two toggles turns the alternation
/// into two presses in a row — and A held across a tick boundary is exactly what pokered's
/// `HoldTextDisplayOpen` spins on. Seconds of game time, so it is the cheap guard on that fix.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_enter_hall_of_fame() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-champion.bin"),
        Duration::from_mins(30),
        vec![],
    );
    let s = fixture.run_until(|s| s.map.map == Map::HallOfFame);
    println!("credits rolling at {} @ {}", s.map.map, s.map.player_position);
}
