//! Saffron: entry past the Route-7 guard → Eevee/Vaporeon → Silph Co (Card Key → Giovanni →
//! liberation) → Marsh Badge.
//!
//! The legs run in the order `complete_game_steps` composes them, and each is seeded from the
//! snapshot the previous one writes. Vaporeon is fetched **before** Silph deliberately: its Surf is
//! the answer to the 7F rival's Alakazam and to Blaine's Fire team, and it ferries the party across
//! Route 21 later.

use super::*;

/// From Fuchsia (post-Safari), trek to Celadon, buy a Fresh Water from the roof vending machine
/// (`UseVendingMachine`), and pass the Route-7 guard into Saffron. Reverses the Soul-Badge gates
/// (Route 15/12 gates west→east / south→north; the Lavender→Route 8 and Route-7-gate crossings use
/// `EnterMap { to_position }`).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_enter_saffron() {
    // ⚠️ Pinned to the pre-**J** battle timing — see `TestFixture::with_original_battle_timing`.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-safari.bin"),
        Duration::from_mins(60),
        PolicyStep::saffron_entry_steps(),
    ).with_original_battle_timing();
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {} has_water={}", s.map.map, s.map.player_position, s.bag.contains(&ItemId::FreshWater));
    assert_eq!(s.map.map, Map::SaffronCity, "should enter Saffron City");
    fixture.save_state_named("src/pokemon/data/at-saffron.bin").unwrap();
}

/// The free Celadon **Eevee**, evolved to **Vaporeon** with a Water Stone.
///
/// ⚠️ **The mainline stopped doing this and the test is kept for one mechanism it is the only cover
/// for.** The route's starter is a Squirtle and Blastoise learns Surf itself, so the Eevee leg was
/// deleted; `PolicyStep::EvolveWithStone` is used nowhere else in the suite, and neither is a gift
/// Pokémon picked up off the floor of a building. So the steps live here, cut down to what they
/// prove: no Route 7 gate crossings (they were the fragile half — asking for the far landing put the
/// player two tiles from a door it could not reach and the step oscillated there for a whole
/// budget), no HM teach (HM03 is not in the bag this early), and no fixture written for anything
/// downstream.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_vaporeon() {
    use crate::pokemon::map::MapSprite as MS;
    use crate::geometry::Point8;
    let steps = vec![
        // Free Eevee from the Celadon Mansion roof house (BACK entrance (24,3)→1F(4,0); the front
        // door is the dead-end condos). Climb the stairwell to the roof.
        PolicyStep::EnterMap { to_map: Map::CeladonMansion1F, to_position: Some(Point8 { x: 4, y: 0 }) },
        PolicyStep::enter(Map::CeladonMansion2F),
        PolicyStep::enter(Map::CeladonMansion3F),
        PolicyStep::enter(Map::CeladonMansionRoof),
        PolicyStep::enter(Map::CeladonMansionRoofHouse),
        PolicyStep::CollectItem(MS::CELADONMANSION_ROOF_HOUSE_EEVEE_POKEBALL),
        PolicyStep::enter(Map::CeladonMansionRoof),
        PolicyStep::enter(Map::CeladonMansion3F),
        PolicyStep::enter(Map::CeladonMansion2F),
        PolicyStep::EnterMap { to_map: Map::CeladonMansion1F, to_position: Some(Point8 { x: 4, y: 0 }) },
        PolicyStep::enter(Map::CeladonCity),
        // Dept Store 4F: buy a Water Stone.
        PolicyStep::enter(Map::CeladonMart1F),
        PolicyStep::enter(Map::CeladonMart2F),
        PolicyStep::enter(Map::CeladonMart3F),
        PolicyStep::enter(Map::CeladonMart4F),
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::WaterStone, 1), map: Map::CeladonMart4F },
        PolicyStep::enter(Map::CeladonMart1F),
        PolicyStep::enter(Map::CeladonCity),
        // ⚠️ By **species**: where the gift Eevee lands depends on how many members the party already
        // has, which is exactly what a `Slot` target gets wrong.
        PolicyStep::EvolveWithStone { stone: ItemId::WaterStone,
                                      target: PartyRef::Species(PokemonSpecies::Eevee) },
    ];
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-celadon.bin"),
        Duration::from_mins(60),
        steps,
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    let vaporeon = s.pokemon.iter()
        .find(|p| p.species == PokemonSpecies::Vaporeon)
        .expect("Eevee should have evolved into Vaporeon with the Water Stone");
    println!("Vaporeon lv{} {:?}", vaporeon.level, vaporeon.moves);
}

/// Enter Silph Co, ride the elevator to 5F, thread the teleport-pad maze to the Card Key pocket, and
/// grab the Card Key (restocking Hyper Potions in Saffron on the way).
///
/// Two things had to work. **The elevator** (1F → step into the (20,0) door → ride to any floor)
/// needed five fixes: (1) `read_warp_events` crashed `game_state()` the moment the player entered any
/// elevator, because the elevator's exits point at the header-less UNUSED_MAP_ED placeholder;
/// (2)/(3)/(4) three hard-coded `Map::RocketHideoutElevator` checks (policy ×2, agent ×1)
/// skipped/aborted the elevator for every non-Rocket elevator; (5) the floor menu scrolls (11 floors)
/// so the cursor is driven by *absolute* index, and the pick's A-press is re-pulsed until the ride
/// starts. **The maze**: the Card Key sits in a walled 5F pocket (row 16) reachable only by *arriving*
/// on the 5F (9,15) pad and stepping down. (9,15)↔9F(17,15) are a teleport pair, so the route is
/// `enter(9F)` (walk to the reachable (9,15) pad → 9F(17,15)) then `enter(5F)` (step back onto (17,15)
/// → arrive standing on 5F(9,15), now adjacent to the pocket) — expressed directly as `enter()` steps,
/// no new maze-routing machinery needed.
///
/// Seeded from `at-saffron.bin`, which is where the mainline is: there is no Eevee leg any more, so
/// the chain runs straight from Saffron into Silph Co. (It used to come from `vaporeon-ready.bin`,
/// because the route fetched Vaporeon first for its Surf against the 7F rival's Alakazam; Blastoise
/// takes that fight on bulk.)
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_silph_card_key() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-saffron.bin"),
        Duration::from_mins(30),
        PolicyStep::silph_co_card_key_steps(),
    );
    let s = fixture.run_leg(|s| s.bag.contains(&ItemId::CardKey));
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    fixture.save_state_named("src/pokemon/data/silph-card-key.bin").unwrap();
}

/// The Silph Co endgame: pads and the elevator up to 11F, the 7F rival, Giovanni, and the President.
///
/// Giovanni's scripted battle only fires when the player **stands on** 11F (6,13)/(7,12) — talking to
/// him does nothing, and it is his *after-battle* script that liberates Saffron. The route to his front
/// at (6,10) passes through (6,13), so walking at him is what starts the fight; `InteractIfReachable`
/// is queued repeatedly because a plain `Interact` pops the moment it issues the walk and cannot resume
/// after the Rocket in the path interrupts it. Ends back in Saffron City, healed.
///
/// This was `#[ignore]`d because the navigation worked — the run reached 11F — and then the 7F rival
/// fight was unwinnable: `silph-card-key.bin` had been cut under an older leg ordering that put Silph
/// before Vaporeon, so it arrived with Venusaur alone against an Alakazam and a Charizard. Nothing was
/// wrong with the leg. Re-pointing [`can_get_silph_card_key`] at `vaporeon-ready.bin` put the chain
/// back in `complete_game_steps`' own order and the Vaporeon back in the party.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_beat_silph_giovanni() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/silph-card-key.bin"),
        Duration::from_mins(120),
        PolicyStep::silph_giovanni_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {}", s.map.map, s.map.player_position);
    for (i, p) in s.pokemon.iter().enumerate() {
        println!("  {i}: {:?} lv{} hp {}/{}", p.species, p.level, p.current_hp, p.stats.hp);
    }
    assert!(s.bag.contains(&ItemId::MasterBall),
        "the Silph President should hand over the Master Ball once Giovanni is beaten");
    assert_eq!(s.map.map, Map::SaffronCity, "should exit Silph Co back into a liberated Saffron");
    fixture.save_state_named("src/pokemon/data/post-silph-giovanni.bin").unwrap();
}

/// Saffron Gym → **Marsh Badge**. The gym is a 3×3 grid of rooms joined only by teleport pads
/// (self-referential intra-map warps); the agent solves the maze for free because `bfs_from_player`
/// routes *through* those pads the same way it routes through arrow/spinner tiles, so a plain
/// `DefeatGymLeader` reaches Sabrina. Requires Saffron to have been liberated by the leg above.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_marsh_badge() {
    // ⚠️ **`post-silph-giovanni.bin`, and it used to be a hand-cut `at-saffron-post-silph.bin`.**
    // That root existed because the Silph leg was snapshotted the moment Giovanni's after-battle
    // script completed, with the player still on 11F and this leg nowhere to start from; the Silph
    // leg now walks itself back out and asserts it is standing in a liberated Saffron, so the chain
    // joins up and there is one less root carrying a party nothing produces. (It carried the old
    // Venusaur/Vaporeon/Pidgey team, which is how a fully regenerated chain still handed the Seafoam
    // leg a party with no Blastoise in it to use Strength.)
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-silph-giovanni.bin"),
        Duration::from_mins(30),
        PolicyStep::marsh_badge_steps(),
    );
    let s = fixture.run_until(|s| s.badges.contains(Badge::MarshBadge));
    println!("final: map={} @ {} party0 lv{}", s.map.map, s.map.player_position,
        s.pokemon.get(0).map(|p| p.level).unwrap_or(0));
    fixture.save_state_named("src/pokemon/data/post-marsh-badge.bin").unwrap();
}

/// ⭐ **Every teleport pad in Saffron Gym is a row, named by its own square.**
///
/// The gym is nine rooms joined only by intra-map warps, and `bfs_from_player` treats a pad the way
/// it treats a spinner: stepping onto it hands control to the game, so the edge it records runs from
/// the square *beside* the pad to the pad's **landing**, and the pad itself never gets a `dist`
/// entry. That is right for crossing the maze and wrong for naming a pad as a destination — and
/// `actions()` used to require exactly that `dist` entry, so a pad was offered only while it
/// happened to be some other pad's landing. The coverage walk of 2026-09-09 was offered a handful
/// that way, chose one, and the moment it moved the row stopped existing: **37 defects, every one
/// of them "there is no route to the warp to SaffronGym", and 239 of the walk's 454 turns spent in
/// this room.**
///
/// So a pad is priced by the square you step onto it *from* (`actions()`'s `pad_approach`), which is
/// what `reconstruct` would have produced had it been an ordinary terminal.
///
/// ⚠️ **And the pad underfoot is the one that matters most.** `bfs_from_player` used to skip a
/// settled neighbour before it looked at what the neighbour *was*, and the search's own root is
/// settled at price 0 — so standing on a pad threw away the only edge out of the room. This state
/// stands on the gym's centre-room pad; without that fix Sabrina, the Gym Guide and the door out
/// are all "no route" from here, which is what the walk reported.
/// ⭐ **A warp you *warped* onto is not one you can lean on, and every elevator in the game lands
/// you on exactly such a square.**
///
/// `home/overworld.asm`'s `.noDirectionChange` reaches `ExtraWarpCheck` and `CheckWarpsCollision`
/// only past `bit BIT_STANDING_ON_WARP, [hl]` — a flag `CheckWarpsNoCollision` sets when a completed
/// **step** lands on a warp entry, and which is therefore clear for a player the cartridge put there
/// itself. The Silph Co elevator's two entries at (1, 3) and (2, 3) are the squares you arrive on,
/// its raw tile `$14` is not in LOBBY's door table, and its warp destination in the ROM is the
/// placeholder `UNUSED_MAP_ED` that `SilphCoElevatorStoreWarpEntriesScript` overwrites at map load
/// with wherever you came from.
///
/// The coverage walk of 2026-09-10 held Down there for 60 s of game time and reported that it "did
/// not arrive" while standing exactly there. Measured on this state: 60 ticks of Down move nothing
/// and `wMovementFlags` reads `$00` throughout; Up and then Down warps out on the first step.
///
/// ⚠️ **Two places had to learn it — the third time that has been true of a warp rule**, and the
/// count is the point rather than the coincidence: `MetaTileMap::actions` builds the
/// `[opposite(dir), dir]` pair, and `OverworldMovement` tests for a border warp *before* it consults
/// the route and would otherwise press the outward direction itself. This test fails if either
/// condition is removed.
///
/// The fixture is the save state the walk dropped at the moment the verdict turned, which is the
/// only moment it exists.
// Default tier for the same reason as its Seafoam sibling: the state is two ticks from the answer.
#[test]
fn an_elevator_door_you_warped_onto_is_stepped_onto_rather_than_leant_on() {
    use crate::geometry::Point8;
    const DOOR: Point8 = Point8 { x: 1, y: 3 };

    // ⚠️ **No `PolicyStep`, because there is no map to name.** `SilphCoElevator_Object`'s two
    // `warp_event`s are written `UNUSED_MAP_ED, 1` and
    // `SilphCoElevatorStoreWarpEntriesScript` overwrites the destination in `wWarpEntries` at map
    // load with wherever the player came from — so the ROM table the map model reads honestly names
    // a map that does not exist, and only the cartridge knows where the door goes. The row is taken
    // the way the coverage walk takes one: straight off `actions()`.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/silph-elevator-warped-in.bin"), Duration::from_mins(2), vec![]);
    let start = fixture.game_state();
    println!("from {} @ {} standing_on_warp={}",
        start.map.map, start.map.player_position, start.map.standing_on_warp);
    assert_eq!(start.map.map, Map::SilphCoElevator);
    assert_eq!(start.map.player_position, DOOR, "the state is dropped standing on the entry itself");
    assert!(!start.map.standing_on_warp,
        "and it got there by warping, which is the whole of it: `wMovementFlags` bit 2 is clear");

    let door = start.map.actions().into_iter()
        .find(|action| action.destination == DOOR)
        .expect("the door underfoot is a row");
    // The step off and the step back on, rather than the one held button a walked-onto entry gets.
    assert_eq!(door.route.len(), 2, "route was {:?}", door.route);
    fixture.agent.take_overworld_action(door);

    let end = fixture.run_until(|state| state.map.map != Map::SilphCoElevator);
    println!("ended on {} @ {}", end.map.map, end.map.player_position);
    assert_ne!(end.map.map, Map::SilphCoElevator,
        "the door has to fire, and holding the outward direction on it never will");
}

#[test]
fn every_teleport_pad_in_the_gym_is_a_row_including_the_one_underfoot() {
    use crate::geometry::Point8;
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use crate::pokemon::tile::MetaTile;
    use std::sync::Arc;

    let mmu = crate::mmu::MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    let metadata = Arc::new(mmu.read_map_metadata(Map::SaffronGym).unwrap());
    // (1, 5) is a pad in the top-left room; its landing is (11, 11), the centre room's only pad,
    // and the centre room is where Sabrina stands. (1, 10) is ordinary floor in the middle-left
    // room, and is where the walk of 2026-09-09 was standing when it read back that eleven of these
    // rows did not exist.
    let standing_on: Point8 = match std::env::var("GB_PROBE_AT").ok().as_deref() {
        Some("floor") => Point8 { x: 1, y: 10 },
        _ => Point8 { x: 1, y: 5 },
    };
    let map = MetaTileMap::new(&CurrentMap {
        player_position: standing_on,
        player_direction: PlayerFacingDirection::Down,
        sprites: Vec::new(),
        metadata: Arc::clone(&metadata),
        closed_doors: Vec::new(),
        grass_encounter_rate: 0,
        card_key_locked: false,
        header_loaded: true,
        surfing: false,
        sprites_loaded: true,
        script_cancelled_warps: Vec::new(),
        standing_on_warp: true,
    });

    let pads: Vec<Point8> = map.meta_tiles.iter().enumerate()
        .filter(|(_, t)| matches!(t, MetaTile::Warp { to_map, .. } if *to_map == Map::SaffronGym))
        .map(|(i, _)| Point8 { x: (i % map.width) as u8, y: (i / map.width) as u8 })
        .collect();
    assert_eq!(pads.len(), 30, "the gym's 32 warps are 30 pads and the two halves of its door");

    let ids: Vec<String> = map.actions().iter().map(|a| a.id()).collect();
    // ⚠️ One row is deliberately missing: the pad whose landing is the square the player is standing
    // on. `actions()` withholds it because there is nothing to go to — and because the agent's
    // arrival test is `player_position == to_position`, so offering it would report a walk that
    // never happened. Every other pad is here.
    let landing_of = |p: Point8| match map.tile_at(p) {
        MetaTile::Warp { to_position, .. } => to_position,
        other => panic!("{p} is {other:?}, not a pad"),
    };
    let already_here: Vec<Point8> = pads.iter().copied()
        .filter(|&p| landing_of(p) == standing_on).collect();
    assert!(already_here.len() <= 1, "at most one pad lands where the player stands: {already_here:?}");

    for pad in &pads {
        let want = format!("SaffronGym:{},{}:Warp", pad.x, pad.y);
        assert_eq!(ids.contains(&want), !already_here.contains(pad),
            "{want} — offered when it should not be, or missing when it should be: {ids:?}");
    }

    // The pad underfoot re-fires with a step off and a step back on (`WarpTrigger::StepOn`), and
    // everything behind it is reachable again.
    if standing_on != (Point8 { x: 1, y: 5 }) { return }
    let underfoot = map.actions().into_iter()
        .find(|a| a.id() == format!("SaffronGym:{},{}:Warp", standing_on.x, standing_on.y))
        .expect("the pad the player is standing on is still a way to go somewhere");
    assert_eq!(underfoot.route.len(), 2, "off and back on: {:?}", underfoot.route);
    // …and the room it leads to comes back with it. The centre room is walled off from every other
    // room and (11, 11) is the only pad in it, so the one edge this test is really about is the one
    // that runs from a square beside the player's own into it. Sabrina stands at (9, 8); the map is
    // built with no sprites, so what is asserted is her floor.
    assert!(map.route_to(Point8 { x: 9, y: 9 }).is_some(),
        "the centre room is behind the pad underfoot and nothing else");
    assert!(ids.iter().any(|id| id == "SaffronGym:9,17:Warp"), "and the way out: {ids:?}");
}

/// ⭐ **An intra-map warp is finished by arriving, because nothing else can say so.**
///
/// Every completion the agent had for a `Warp` row was the map changing, and a teleport pad does not
/// change the map. So no intra-map warp row in the game had ever been reported as completed: the
/// coverage walk took thirty of them and scored every one a defect, and a model would have read
/// thirty walks that went quiet. `player_position == to_position` is exact rather than approximate,
/// because a pad's landing is reached by that pad and by nothing else.
///
/// This starts in the gym's centre room, whose only pad is (11, 11) → (1, 5).
#[test]
fn a_teleport_pad_reports_arriving_even_though_the_map_never_changed() {
    use crate::geometry::Point8;
    use crate::pokemon::tile::MetaTile;
    const PAD: Point8 = Point8 { x: 11, y: 11 };
    const LANDING: Point8 = Point8 { x: 1, y: 5 };

    struct TakeThePad;
    impl crate::pokemon::policy::Policy for TakeThePad {
        fn name(&self) -> &'static str { "take-the-pad" }
        fn pick_overworld_action(&mut self, state: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction> {
            // Once only: a re-issued row would hide a completion that never came.
            (state.map.player_position != LANDING).then(|| state.map.actions().into_iter()
                .find(|a| a.destination == PAD))?
        }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<crate::pokemon::battle::BattleAction> { None }
        fn pick_field_move(&mut self, _: &GameState) -> Option<crate::pokemon::policy::FieldMove> { None }
    }

    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/post-marsh-badge.bin"), Duration::from_mins(3), Box::new(TakeThePad));
    let start = fixture.game_state();
    assert_eq!(start.map.map, Map::SaffronGym);
    assert_eq!(start.map.tile_at(PAD),
        MetaTile::Warp { to_map: Map::SaffronGym, to_position: LANDING });
    println!("from {} ", start.map.player_position);

    // ⚠️ Arriving is not the assertion — being *told* is. The walk moves the player either way; what
    // this test exists for is the `OverworldActionCompleted` that used to never come and turned,
    // sixty seconds later, into "the walk was given up without getting there".
    let mut reported = false;
    for _ in 0..3_000 {
        for event in fixture.agent.drain_events() {
            if let crate::pokemon::agent::AgentEvent::OverworldActionCompleted {
                destination: MetaTile::Warp { to_map: Map::SaffronGym, to_position } } = event
                && to_position == LANDING
            {
                reported = true;
            }
        }
        if reported { break }
        fixture.step();
    }
    let end = fixture.game_state();
    println!("landed on {} reported={reported}", end.map.player_position);
    assert_eq!(end.map.player_position, LANDING, "the pad puts the player in the top-left room");
    assert!(reported, "the pad has to report that it arrived");
}
