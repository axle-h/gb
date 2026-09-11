//! Saffron: past the Route 7 guard, Eevee to Vaporeon, Silph Co and Giovanni, the Marsh Badge.

use super::*;

/// From post-Safari Fuchsia: a Fresh Water from Celadon's roof, then past the Route 7 guard.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_enter_saffron() {
    // Animations on: `TestFixture::with_original_battle_timing`.
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

/// The free Celadon Eevee, evolved to Vaporeon with a Water Stone.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_vaporeon() {
    use crate::pokemon::map::MapSprite as MS;
    use gb::geometry::Point8;
    let steps = vec![
        // The free Eevee in the Celadon Mansion roof house, by the back entrance; the front door is
        // the dead-end condos.
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
        PolicyStep::enter(Map::CeladonMart1F),
        PolicyStep::enter(Map::CeladonMart2F),
        PolicyStep::enter(Map::CeladonMart3F),
        PolicyStep::enter(Map::CeladonMart4F),
        PolicyStep::BuyFromMart { item: BagItem::new(ItemId::WaterStone, 1), map: Map::CeladonMart4F },
        PolicyStep::enter(Map::CeladonMart1F),
        PolicyStep::enter(Map::CeladonCity),
        // By species: where the gift Eevee lands depends on the party's size, which a `Slot` gets
        // wrong.
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

/// Silph Co 5F's teleport-pad maze to the Card Key, restocking Hyper Potions on the way.
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

/// The Silph Co endgame: up to 11F, the 7F rival, Giovanni, and the President.
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

/// Saffron Gym → Marsh Badge.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_marsh_badge() {
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

/// An elevator door you warped onto is taken with a step off and a step back on.
#[test]
fn an_elevator_door_you_warped_onto_is_stepped_onto_rather_than_leant_on() {
    use gb::geometry::Point8;
    const DOOR: Point8 = Point8 { x: 1, y: 3 };

    // No `PolicyStep`, because there is no map to name.
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
    // The step off and the step back on, not the held button a walked-onto entry gets.
    assert_eq!(door.route.len(), 2, "route was {:?}", door.route);
    fixture.agent.take_overworld_action(door);

    let end = fixture.run_until(|state| state.map.map != Map::SilphCoElevator);
    println!("ended on {} @ {}", end.map.map, end.map.player_position);
    assert_ne!(end.map.map, Map::SilphCoElevator,
        "the door has to fire, and holding the outward direction on it never will");
}

/// Every teleport pad in Saffron Gym is a row, named by its own square.
#[test]
fn every_teleport_pad_in_the_gym_is_a_row_including_the_one_underfoot() {
    use gb::geometry::Point8;
    use crate::pokemon::map_metadata::{CurrentMap, MapMetadataReader, PlayerFacingDirection};
    use crate::pokemon::tile::MetaTile;
    use std::sync::Arc;

    let mmu = gb::mmu::MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    let metadata = Arc::new(mmu.read_map_metadata(Map::SaffronGym).unwrap());
    // (1, 5) is a top-left pad landing on (11, 11), the only pad in Sabrina's centre room.
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
        water_encounter_rate: 0,
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
    // The pad whose landing is the square underfoot is the one row missing.
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

    // The pad underfoot re-fires with a step off and back on (`WarpTrigger::StepOn`), and what is
    // behind it is reachable.
    if standing_on != (Point8 { x: 1, y: 5 }) { return }
    let underfoot = map.actions().into_iter()
        .find(|a| a.id() == format!("SaffronGym:{},{}:Warp", standing_on.x, standing_on.y))
        .expect("the pad the player is standing on is still a way to go somewhere");
    assert_eq!(underfoot.route.len(), 2, "off and back on: {:?}", underfoot.route);
    // And the room it leads to comes back.
    assert!(map.route_to(Point8 { x: 9, y: 9 }).is_some(),
        "the centre room is behind the pad underfoot and nothing else");
    assert!(ids.iter().any(|id| id == "SaffronGym:9,17:Warp"), "and the way out: {ids:?}");
}

/// An intra-map warp is finished by arriving, because nothing else can say so.
#[test]
fn a_teleport_pad_reports_arriving_even_though_the_map_never_changed() {
    use gb::geometry::Point8;
    use crate::pokemon::tile::MetaTile;
    const PAD: Point8 = Point8 { x: 11, y: 11 };
    const LANDING: Point8 = Point8 { x: 1, y: 5 };

    struct TakeThePad;
    impl crate::pokemon::policy::Policy for TakeThePad {
        fn name(&self) -> &'static str { "take-the-pad" }
        fn pick_overworld_action(&mut self, state: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
            -> Option<crate::pokemon::actions::OverworldAction> {
            // Once only: a re-issued row would hide a missing completion.
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

    // Being told is the assertion, not arriving.
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
