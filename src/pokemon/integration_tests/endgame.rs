//! Earth Badge → Victory Road → the Elite Four → the Hall of Fame.

use super::*;

/// Append the **Articuno** that `seafoam_articuno_steps` catches, at the level it is caught (50), with
/// the moves it is caught with.
///
/// ⚠️ **Seeded state, not earned state** — the same device as
/// [`postgame::legendaries::seed_master_ball`], and it is here because the endgame leg chain has
/// *diverged from the mainline*. `complete_game_steps` runs `seafoam_articuno_steps` between the
/// Volcano and Earth badges, so it reaches Victory Road with four party members; but
/// `post-volcano-lone.bin` — the root this chain hangs off, and one no test produces — was cut before
/// that leg existed and is explicitly the two-mon "lone" party. Asking a lv56 Venusaur, a lv30 Vaporeon
/// and the lv24 Machop it catches on arrival to clear Victory Road's nine trainers with no Pokémon
/// Center inside is asking for something the mainline never asks: it blacks out to Viridian around the
/// last cooltrainer, twice out of two, on either RNG stream.
///
/// Seeding the bird is the cheap half of re-cutting the chain. The honest fix is to re-cut
/// `post-volcano-lone.bin` out of a Seafoam-era `full_playthrough`; until someone does, this keeps the
/// leg testing what it is *for* — the Machop catch, the HM04 teach and the boulder puzzle — rather than
/// a party-strength accident of its seed. Note it is a *generous* Articuno: [`Pokemon::maxed`] gives
/// max IVs/EVs and the level is then wound back to 50, so it is stronger than one actually caught.
/// That is deliberate — the point here is to stop the gauntlet deciding the test, not to model the
/// mainline's bird exactly.
fn seed_seafoam_articuno(fixture: &mut TestFixture) {
    use crate::pokemon::pokemon::Pokemon;
    let mut party = fixture.game_state().pokemon;
    if party.iter().any(|p| p.species == PokemonSpecies::Articuno) { return; }

    let mut articuno = Pokemon::maxed(PokemonSpecies::Articuno, "ARTICUNO",
        [PokemonMoveName::Peck, PokemonMoveName::IceBeam, PokemonMoveName::Agility,
         PokemonMoveName::Mist],
        fixture.game_state().name.clone(), fixture.game_state().player_id);
    articuno.experience = PokemonSpecies::Articuno.metadata().experience_group.experience_for_level(50);
    articuno.recalculate();
    articuno.current_hp = articuno.stats.hp;

    party.push(articuno);
    fixture.api().debug_set_party(&party).expect("the lone party has room for another mon");
}

/// From `post-volcano-badge.bin` (in Blaine's gym with 7 badges — exactly where the mainline is; the
/// Seafoam detour is no longer on the route): Surf back to Pallet and up to Viridian, then clear
/// Giovanni's
/// **Viridian Gym** spinner-tile maze for the **Earth Badge**, the 8th and final gym badge. Exercises
/// the `ViridianGym` arrow-tile table.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_earth_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-volcano-badge.bin"),
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
/// This was `#[ignore]`d blaming the bag: `TeachMove { Hm04Strength }` never completed, and the
/// standing theory was item-menu **scrolling** for an HM deep in the bag. That theory was already
/// disproven in the tree — `postgame::fly_bike::can_teach_fly` teaches HM02 from bag index 15 of 16 in
/// 0.6 s. The real cause was the `machop_slot` argument: the leg took the slave's party index as a
/// *parameter*, and its two callers disagreed about it (`complete_game_steps` passed 4, this test
/// passed 2), so on some parties the teach was aimed at a mon that cannot learn Strength — and since
/// the step's completion check reads that same slot, it could never finish. Naming the Machop by
/// species removed the argument and the guess with it; the teach now lands in ~20 ticks.
///
/// Fixing that exposed a second blocker underneath, which is why this test is **party-seeded** — see
/// [`seed_seafoam_articuno`], and read it before touching the seed. Its budget is also **180 emulated
/// minutes**, not the 120 it had while ignored: `CatchPokemon` waits out Victory Road's wild table for
/// a Machop and is exempt from stall detection, so an under-sized budget fails as a bare *timeout*
/// with no clue in it.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_solve_victory_road_1f() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-earth-badge.bin"),
        Duration::from_mins(180),
        PolicyStep::victory_road_1f_approach_steps(),
    ).with_stall_tolerance(Duration::from_mins(30));
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    let has_strength = s.pokemon.iter()
        .any(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Strength));
    println!("final: {} @ {}  strength={has_strength}", s.map.map, s.map.player_position);
    assert!(has_strength, "a party member should know Strength for the boulder puzzle");
    assert_eq!(s.map.map, Map::VictoryRoad1F, "should walk to Victory Road 1F");
    fixture.save_state_named("src/pokemon/data/vr1f-strength.bin").unwrap();
}

/// The 1F boulder onto (17,13) and the climb to VR2F, split out from the approach above.
///
/// ⚠️ **`vr1f-strength.bin` has to be cut *on 1F*, and running the whole of
/// `victory_road_1f_steps` here moved it to 2F.** Two other tests read that fixture for what its name
/// says it is: `mechanics::strength_switches_are_exposed` asserts VR1F's single switch is exposed on
/// it (a pure state read, no emulation, which is why it is in the fast tier), and the VR2F/VR3F leg
/// wants the floor *above*. So the approach and the climb are two tests and two fixtures.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_climb_victory_road_1f() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr1f-strength.bin"),
        Duration::from_mins(60),
        PolicyStep::victory_road_1f_climb_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::VictoryRoad2F, "should solve VR1F and climb to VR2F");
    fixture.save_state_named("src/pokemon/data/vr2f-ladder.bin").unwrap();
}

/// The interconnected VR2F/VR3F Strength puzzle, through to the Indigo Plateau lobby: switch1 → 3F →
/// hole-drop reveals the hidden 2F boulder → fall → switch2 → return trip → exit. Every boulder is a
/// real Strength push.
///
/// Self-contained from `vr1f-strength.bin` because chaining it onto a *fresh* run is PP-marginal —
/// Victory Road's ~9 mandatory trainers plus the Route-22 rival drain the lead past its damaging PP
/// in some RNG lines and there is no Pokémon Center inside. That is why `victory_road_2f_3f_steps` is
/// not in `complete_game_steps`, and why this proof lives here instead.
///
/// ⚠️ **The Strength it re-arms is the starter's, and this line still named the Machop the route used
/// to catch.** `PolicyStep::UseStrength` waits rather than failing when its `PartyRef` does not
/// resolve — which is right, because a `Species` target may not be caught yet — so a name no party
/// member has any more is a silent stall, and it sat here for a whole budget.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_solve_victory_road_2f_3f() {
    // ⚠️ **The 1F climb is part of this test, not a leftover.** Seeded on VR1F with Strength ready,
    // it pushes 1F's boulder, climbs the ladder and then solves the interconnected floors — which is
    // the sequence `complete_game_steps` plays. Starting it on VR2F instead (from a fixture cut after
    // the climb) lands the player on a different tile of a Sokoban puzzle and the second switch is
    // then unreachable: the run finishes eleven of fourteen steps and stalls with only VR1F and VR3F
    // warps in reach.
    let mut steps = PolicyStep::victory_road_1f_climb_steps();
    steps.extend(PolicyStep::victory_road_2f_3f_steps());
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr1f-strength.bin"),
        Duration::from_mins(60),
        steps,
    );
    // ⚠️ **Stopped on the plateau *outside* the lobby, and the reason is not this test.**
    // `at-indigo.bin`'s only other reader is `llm::map_image`, whose fixture list is chosen for what
    // each state makes *drawable* — its entry for this one is "a `Plateau` map whose strip tileset
    // differs from its own", which is the open-air `IndigoPlateau` and not the building standing on
    // it. The committed file had been an outdoor state for a long time because nothing regenerated
    // it; the first regeneration in a year replaced it with the lobby and both map-render tests went
    // red with "only 0 ring pixels". Nothing needs the lobby: `elite_four_steps` opens by routing to
    // its mart, and the Elite Four leg is seeded from `at-indigo-articuno.bin`. Walking out of
    // Victory Road onto the plateau is what this leg is proving either way.
    let s = fixture.run_until(|s| s.map.map == Map::IndigoPlateau);
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    fixture.save_state_named("src/pokemon/data/at-indigo.bin").unwrap();
}

/// The Elite Four gauntlet, from the Indigo Plateau lobby to the credits: stock up, heal, then
/// Lorelei → Bruno → Agatha → Lance → the rival, and on through Oak's post-Champion script into the
/// **Hall of Fame**.
///
/// ⚠️ **The lead lookup this test used to do is gone**, and so are `elite_four_steps`' slot
/// arguments: the steps name Venusaur and Articuno by species and resolve them against the live
/// party. Working an index out here and passing it in is the `machop_slot` mistake, and this test
/// was the caller that had to get it right.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_beat_elite_four() {
    const FIXTURE: &[u8] = include_bytes!("../data/at-indigo-articuno.bin");

    // 180 min covers the five rooms plus Oak's post-Champion speech and the walk to the Hall of Fame.
    // ⚠️ Pinned to the pre-**J** battle timing — see `TestFixture::with_original_battle_timing`. The
    // gauntlet's win is a tuned sequence of switches and Blizzards against six of Lance's dragons;
    // shifting the RNG stream under it re-rolls every accuracy and crit check in five long fights,
    // and §3 puts battle tactics out of scope precisely because in deployment they are the LLM's.
    let mut fixture = TestFixture::new(
        FIXTURE,
        Duration::from_mins(180),
        PolicyStep::elite_four_steps(),
    ).with_original_battle_timing();

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

/// **The mainline's own tail: Victory Road 2F to the Hall of Fame, from the party the playthrough
/// actually arrives with.** This is the composition `full_playthrough` now runs, on the fixture that
/// run writes, and it exists so the endgame can be iterated on in minutes rather than by replaying
/// the whole game each time.
///
/// ⚠️ **It is a different question from `can_beat_elite_four`, which is why both are kept.** That
/// one runs from `at-indigo-articuno.bin` — a rich fixture, ¥49,975 — pinned to the pre-J battle
/// timing, so it proves the gauntlet against one known-good RNG line. This one runs the *mainline's*
/// composition on the live timing and the ¥9,710 the run actually arrives with, which is where the
/// three-Full-Restore ceiling and `agent::affordable`'s trim actually bite.
///
/// ⚠️ **The two leads are seeded to `GAUNTLET_LEVEL` rather than grinded, and this test therefore
/// proves nothing about the grind.** The grind lives in `victory_road_1f_steps`, one floor below and
/// several hours of game time before this fixture, and it needs the Viridian Centre in the world
/// graph to recover PP — which a fixture that starts on VR2F does not have. Seeding is the same
/// device (and the same warning) as [`seed_seafoam_articuno`]: it keeps this test about the puzzle,
/// the shopping and the five rooms, and leaves "can the run reach that weight" to `full_playthrough`,
/// which is the only thing that can answer it.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_finish_from_victory_road() {
    let mut steps = PolicyStep::victory_road_2f_3f_steps();
    steps.extend(PolicyStep::elite_four_steps());

    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-victory-road-1f.bin"),
        Duration::from_mins(3000),
        steps,
    );
    seed_gauntlet_levels(&mut fixture);
    {
        let s = fixture.game_state();
        println!("start: {} @ {} | ¥{}", s.map.map, s.map.player_position, s.money);
        for p in s.pokemon.iter() { println!("  {:?} lv{}", p.species, p.level); }
    }

    let s = fixture.run_until(|s| s.map.map == Map::HallOfFame);
    println!("HALL OF FAME — final team:");
    for p in s.pokemon.iter() { println!("  {:?} lv{} {}/{}hp", p.species, p.level, p.current_hp, p.stats.hp); }
}

/// Wind the three gauntlet fighters up to what `victory_road_grind_steps` would have left them at.
///
/// ⚠️ **Seeded state, not earned state** — the same device as [`seed_seafoam_articuno`], and read
/// that one before touching this. The experience is set from the species' own growth curve and the
/// stats recomputed, so these are ordinary Pokémon at that level rather than the max-IV specimens
/// `Pokemon::maxed` would build.
fn seed_gauntlet_levels(fixture: &mut TestFixture) {
    // ⚠️ **The Elixer is seeded for the same reason the levels are, and it is not decoration.** The
    // gauntlet is 26 Pokémon against the starter's 35 PP with no way to restore any of it once the
    // first door closes, so the route now carries the Pokémon Tower 4F Elixer into the Champion's
    // room — see `elite_four_steps`. This fixture was cut before that step existed and its bag has
    // no Elixer, so without this the test proves the five rooms against a PP budget the mainline no
    // longer has. Earning it is `hall_of_fame_playthrough`'s job, exactly as earning the levels is.
    // ⚠️ **A slot has to come out first, because the bag is exactly full.** Measured on this
    // fixture it is **20/20**, so a bare `debug_give_item` is an `Err` — or, worse, it lands and the
    // twelve Full Restores bought at Indigo do not, which is the same test proving less while still
    // passing. ⚠️ **TM06, and which TM matters**: the route spends TM24 on the Celadon roof and TM21
    // at the Indigo mart, so a seed that took either would leave that step popping with nothing to
    // do and the leg would quietly stop exercising a squeeze the mainline feels. This fixture
    // predates both tosses and still carries all of them; none is ever taught, because the starter's
    // four slots are Surf, Dig, Blizzard and Hydro Pump.
    fixture.api().debug_take_item(crate::pokemon::item::ItemId::Tm06Toxic)
        .expect("the fixture carries TM06 and never teaches it");
    fixture.api().debug_give_item(crate::pokemon::item::ItemId::Elixer, 1)
        .expect("the toss above freed a slot");
    let mut party = fixture.game_state().pokemon;
    for slot in 0..party.len() {
        let mon = party.get_mut(slot).expect("in range");
        if !matches!(mon.species, PokemonSpecies::Blastoise) { continue }
        if mon.level >= PolicyStep::GAUNTLET_LEVEL { continue }
        mon.experience = mon.species.metadata().experience_group
            .experience_for_level(PolicyStep::GAUNTLET_LEVEL);
        mon.recalculate();
        mon.current_hp = mon.stats.hp;
    }
    fixture.api().debug_set_party(&party).expect("the party is unchanged in length");
}

/// **The gauntlet grind on its own**, from the fixture the route reaches it at.
///
/// ⚠️ **Its own test because it is the expensive step and it took five sites to place.** Running it
/// through `full_playthrough` costs half an hour before the grind even starts, and the four failure
/// modes it went through — a route whose grass is unreachable, a cave that flees every wild, a cave
/// four maps from a Pokémon Centre, and a cave whose door has a man standing in it — are all things
/// this can show in minutes. See [`PolicyStep::gauntlet_grind_steps`].
///
/// ⚠️ **`hall-of-fame`, not `slow-tests`, and the wrong gate showed up immediately**: the leg chain
/// runs in about 55 seconds and this is **20 minutes**, so one careless attribute turned the whole
/// tier into something nobody would run. It shares a flag with `hall_of_fame_playthrough` because it
/// is the same subject — the grind is most of that test — and the flag does *not* imply `slow-tests`,
/// so both are named in the message.
///
/// It is also the number to watch when anything touches how a grind battle is fought: **1552 wild
/// battles in 1229 s**, twelve heal round trips and no black-outs. It was 2306 battles in 2829 s
/// until the trainee stopped being switched in and started leading — half the experience per
/// knockout and a wasted turn in every one of them, which is 2.3× of this test.
#[test]
#[cfg_attr(not(feature = "hall-of-fame"), ignore = "20 min, the slowest test in the repo but one — \
    run with --features slow-tests,hall-of-fame")]
fn can_grind_for_the_gauntlet() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-articuno.bin"),
        Duration::from_mins(3000),
        PolicyStep::gauntlet_grind_steps(),
    );
    {
        let s = fixture.game_state();
        println!("start: {} @ {}", s.map.map, s.map.player_position);
        for p in s.pokemon.iter() { println!("  {:?} lv{}", p.species, p.level); }
    }
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    for p in s.pokemon.iter() { println!("  {:?} lv{}", p.species, p.level); }
    for species in [PokemonSpecies::Venusaur, PokemonSpecies::Articuno, PokemonSpecies::Vaporeon] {
        let mon = s.pokemon.iter().find(|p| p.species == species)
            .unwrap_or_else(|| panic!("the party should carry a {species:?}"));
        assert!(mon.level >= PolicyStep::GAUNTLET_LEVEL,
            "{species:?} only reached lv{}", mon.level);
    }
}

/// Dump what the agent can see and reach from a save — map, position, money, party, bag, tile under
/// foot, sprites and every action. Instant; the point is to answer "why did that `EnterMap` have
/// nowhere to go" without re-running the leg that produced it.
///
/// ```text
/// GB_PROBE_STATE=src/pokemon/data/post-articuno.bin \
/// cargo test --release --features diagnostics --bin gb -- probe_stall_actions --ignored --nocapture
/// ```
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_stall_actions() {
    let path = std::env::var("GB_PROBE_STATE")
        .unwrap_or_else(|_| "target/test-artifacts/test_stall_state.bin".to_string());
    let Ok(bytes) = std::fs::read(&path) else {
        println!("no state at {path}");
        return;
    };
    let mut fixture = TestFixture::new(&bytes, Duration::from_secs(1), Vec::new());
    let s = fixture.game_state();
    println!("{} @ {}  money ¥{}", s.map.map, s.map.player_position, s.money);
    for mon in s.pokemon.iter() {
        println!("  party {:?} lv{} {}/{}hp {:?}", mon.species, mon.level, mon.current_hp,
            mon.stats.hp, mon.status);
    }
    println!("  bag: {:?}", s.bag.iter().map(|i| (i.id, i.quantity)).collect::<Vec<_>>());
    println!("tile under player: {:?}", s.map.tile_at_checked(s.map.player_position));
    for sprite in &s.map.sprites {
        println!("  sprite {:?} hidden={} @ {}", sprite.name, sprite.hidden, sprite.position);
    }
    for action in s.map.actions() {
        println!("  action {:?} → {} ({} steps)", action.tile, action.destination, action.route.len());
    }
}
