//! S.S. Anne → HM01 Cut → the Vermilion Gym → Thunder Badge → back to Cerulean.

use super::*;

/// Board the S.S. Anne, defeat all 16 cabin/bow trainers (leveling the party), beat the rival, get
/// HM01 Cut from the captain, and disembark back to Vermilion. Each floor is a heal → board → sweep
/// → disembark cycle (no Pokémon Center on the ship). The longest single leg in the chain, and the
/// one that bounds the wall clock of `--features slow-tests`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_clear_ss_anne() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-vermilion.bin"),
        Duration::from_mins(90),
        PolicyStep::ss_anne_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended {} @ {} party_lv={:?} bag={:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| p.level).collect::<Vec<_>>(), s.bag.iter().collect::<Vec<_>>());
    assert!(s.bag.contains(&ItemId::Hm01Cut), "should have HM01 Cut after clearing the S.S. Anne");
    assert_eq!(s.map.map, Map::VermilionCity, "should have disembarked back to Vermilion City");
    // Snapshot post-S.S.-Anne (HM01 in bag, party ~lv32) for the next leg (teach Cut → Lt. Surge).
    fixture.save_state_named("src/pokemon/data/post-ss-anne.bin").unwrap();
}

/// Teach HM01 Cut via the bag (START → ITEM → HM01 → USE → choose Pokémon), from the post-S.S.-Anne
/// save.
///
/// ⚠️ **To the Oddish, not the starter, and that is the point of the test now.**
/// `data/pokemon/base_stats/wartortle.asm` has no CUT, so a `Slot(0)` teach is *correctly* refused by
/// `learnset::can_learn` and skipped — which looks exactly like the driver failing. The route catches
/// an Oddish on Route 25 to hold Cut; this proves the teach lands on a mon that is not the lead and
/// that `can_use_cut` (which asks the whole party) then goes true.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_teach_cut() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_mins(5),
        vec![PolicyStep::TeachMove { item: ItemId::Hm01Cut,
                                    target: PartyRef::Species(PokemonSpecies::Oddish) }],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    for p in s.pokemon.iter() { println!("{} moves: {:?}", p.species, p.moves); }
    assert!(s.can_use_cut, "the Cut carrier should know Cut (can_use_cut true) after TeachMove");
    // Snapshot with Cut taught (at Vermilion) for the next leg (cut the gym tree → trash cans).
    fixture.save_state_named("src/pokemon/data/post-teach-cut.bin").unwrap();
}

/// The two field mechanics that gate Lt. Surge, isolated from the fight itself.
///
/// **Cut**: the `CuttingTree` state drives START→POKéMON→mon→CUT with plain button mashing (cursor to
/// each target index, then A). The agent's `MetaTileMap` is decoded from static ROM, so it still shows
/// the felled tree — it records what it cut and treats it as `Empty` for routing (`observe_state`).
///
/// **The trash cans**: the agent reads which cans hold the two switches (`GameState::trash_cans`, from
/// RAM), walks to each and presses A, unlocking the door to Surge. Junior trainers that engage en route
/// are fought and beaten normally.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_solve_gym_trash_cans() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-teach-cut.bin"),
        Duration::from_mins(15),
        vec![
            PolicyStep::CutTree { map: Map::VermilionCity },
            PolicyStep::enter(Map::VermilionGym),
            PolicyStep::SolveTrashCans,
        ],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    assert_eq!(s.map.map, Map::VermilionGym, "should cut the gym tree and enter the Vermilion Gym");
    let tc = s.trash_cans.clone().expect("trash-can puzzle state in the gym");
    println!("player@{} first_opened={} second_opened={}", s.map.player_position, tc.first_opened, tc.second_opened);
    assert!(tc.second_opened, "both trash-can switches should be flipped (door to Lt. Surge unlocked)");
    fixture.save_state_named("src/pokemon/data/gym-trash-solved.bin").unwrap();
}

/// The integrated Thunder-Badge leg exactly as folded into `complete_game_steps`: from post-S.S.-Anne
/// (HM01 Cut in the bag, in Vermilion City) run `thunder_badge_steps()` — teach Cut, cut the gym tree,
/// solve the trash-can puzzle, beat Lt. Surge — and confirm the badge. Keeps the helper and the full
/// playthrough in lockstep, and subsumes the two mechanic tests above.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_thunder_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-ss-anne.bin"),
        Duration::from_mins(20),
        PolicyStep::thunder_badge_steps(),
    );
    // `DefeatGymLeader` never pops on its own, and the trailing `Interact` retries keep talking to
    // Surge after the win — so stop on the badge, not on an empty queue.
    let s = fixture.run_until(|s| s.badges.contains(Badge::ThunderBadge));
    println!("badges={:?} on {}", s.badges, s.map.map);
    fixture.save_state_named("src/pokemon/data/post-thunder-badge.bin").unwrap();
}

/// From the post-Thunder-Badge state (inside the Vermilion Gym), exit the gym, re-cut the enclosure
/// tree (it regrew when the map reloaded), heal, **catch and grind the Route 11 Drowzee**, teach the
/// starter Dig, and trek back to Cerulean City via the Underground Path (Route 6 → Route 5) —
/// Saffron's Route 6 gate is guard-blocked, so the tunnel is the only way north. Snapshots
/// `back-in-cerulean.bin` for the Rock Tunnel leg.
///
/// ⚠️ **Four hours of game time rather than thirty minutes, because this leg now contains a grind.**
/// The Drowzee arrives at lv9-13 and is taken to **26**, where it becomes a Hypno; a grind's cost is
/// measured in encounters rather than in steps, and the trainee is handed off to a tank on turn one
/// of every one of them, so it earns the halved participation share.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_return_to_cerulean() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-thunder-badge.bin"),
        Duration::from_mins(240),
        PolicyStep::back_to_cerulean_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::CeruleanCity, "should trek back to Cerulean City");
    fixture.save_state_named("src/pokemon/data/back-in-cerulean.bin").unwrap();
}

/// ⭐ **A door the cartridge draws shut is a wall, and Vermilion Gym's are the only ones a finished
/// save can never show you.**
///
/// `VermilionGymSetDoorTile` writes block `$24` over `lb bc, 2, 2` while `EVENT_2ND_LOCK_OPENED` is
/// clear and `$5` once the trash-can puzzle sets it. The static ROM blocks carry the *open* layout,
/// so `MetaTileMap` routed straight through the closed doorway and `actions()` offered a row to
/// Lt. Surge behind it — and the walk then held a direction against a wall until
/// `MAX_MOVEMENT_SILENCE` gave up sixty seconds later.
///
/// ⚠️ **No sweep could find it until a start existed that had not won the game**, because the doors
/// are open in every postgame fixture. `coverage::Start::before_the_credits`'s `ssanne` walked in on
/// three badges on 2026-09-10 and scored `VermilionGym:LtSurge` a defect on the second sweep of the
/// day, which is [coverage-plan](../../../docs/coverage-plan.md) §2.1's one remaining failure. It is
/// on the way to the third badge, so a paying run meets it.
///
/// The cure is `map_uses_runtime_blocks`, which builds the map from `wOverworldMap` — what the
/// cartridge actually drew — rather than from a second hand-transcribed table of block ids and event
/// flags. This test fails if the map goes back on the cached ROM path.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn lt_surge_is_not_a_row_while_his_doors_are_shut() {
    // ⚠️ **`post-teach-cut`, and the tree first.** Vermilion City's cuttable tree at (15, 19) is
    // the only way from the city's north half to the gym, so an earlier state cannot walk there at
    // all and the test would fail for the wrong reason — measured: without the `CutTree` the gym's
    // warp is not in `actions()` either, and for a reason that has nothing to do with the doors.
    // This state is the chain's own "in Vermilion, Cut in hand, gym not yet done", which is exactly
    // the situation a run meets on three badges.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-teach-cut.bin"), Duration::from_mins(10),
        vec![
            PolicyStep::CutTree { map: Map::VermilionCity },
            PolicyStep::enter(Map::VermilionGym),
        ],
    );
    fixture.step_until_exhausted();
    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::VermilionGym);

    let rows: Vec<String> = state.map.actions().iter().map(|a| format!("{:?}", a.tile)).collect();
    println!("in the gym on {} badges: {}", state.badges.bits().count_ones(), rows.join(", "));
    // He is on the map — the row is withheld because the doorway is a wall, not because the sprite
    // is missing, and a test that could not tell those apart would pass on an empty sprite table.
    assert!(
        state.map.sprites.iter().any(|sprite| sprite.name.contains("Surge") && !sprite.hidden),
        "Lt. Surge has to be on the map for this to mean anything: {:?}",
        state.map.sprites.iter().map(|s| s.name).collect::<Vec<_>>(),
    );
    assert!(
        !rows.iter().any(|row| row.contains("Surge")),
        "the doors are shut, so there is no route to Lt. Surge and no row for him: {rows:?}",
    );
    // And the trash cans that open them *are* rows, so the floor is not simply unreachable.
    assert!(
        state.map.actions().len() > 1,
        "the front room has to be reachable: {rows:?}",
    );
}

/// **Cut a coverage start that is standing *on* the ship**: `on-the-ss-anne.bin`.
///
/// ⭐ **Eleven maps depend on one walk out of Vermilion turning south, and it usually does not.**
/// `EVENT_SS_ANNE_LEFT` is set the moment the captain hands over HM01 — before the third badge — and
/// `VermilionCityLeftSSAnneCallbackScript` then shuts the dock for good, so `SSAnne1F`, `1FRooms`,
/// `2F`, `2FRooms`, `3F`, `B1F`, `B1FRooms`, `Bow`, `CaptainsRoom`, `Kitchen` and `VermilionDock`
/// are unreachable from every finished save. The `ssanne` start exists for exactly that and stands
/// in Vermilion City, one warp away — and the frontier's least-taken exit out of Vermilion goes
/// *north*, so its first attempt on 2026-09-10 spent a whole 6-hour budget without ever boarding,
/// and only its second found the ship. A start that is already aboard makes eleven maps a certainty
/// rather than a coin flip, and the walk keeps its budget for the ship instead of spending it on the
/// way there.
///
/// ⚠️ **Two warps and nothing else**, which is the whole point: a coverage start contributes *where
/// the player is standing* and nothing else (`Start`'s own note — the party and the bag come from
/// `Cheats`). So this does not sail, fight or collect anything; it walks aboard and stops.
///
/// Only under `regen-fixtures`. Cut from `at-vermilion.bin`, the same pre-credits save the `ssanne`
/// start uses, so both come from the playthrough's own state one leg before it boards.
#[test]
#[cfg(feature = "regen-fixtures")]
fn regen_on_the_ss_anne_fixture() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-vermilion.bin"),
        Duration::from_mins(20),
        vec![PolicyStep::enter(Map::VermilionDock), PolicyStep::enter(Map::SSAnne1F)],
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::SSAnne1F, "the start has to be standing on the ship");
    fixture.save_state_named("src/pokemon/data/on-the-ss-anne.bin").unwrap();
}
