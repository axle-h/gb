//! Pallet/Viridian → Pewter → Mt Moon → Cerulean → Vermilion.

use super::*;

#[test]
fn can_navigate_to_pewter_city() {
    // Explicit forward navigation (Viridian City → Viridian Forest → Pewter City), the same
    // single-hop `EnterMap` chain `complete_game_steps` uses.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/viridian-city-pokemart-shopping.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::enter(Map::ViridianCity),   // exit the Mart (the save state is inside it)
            PolicyStep::enter(Map::Route2),
            PolicyStep::enter(Map::ViridianForestSouthGate),
            PolicyStep::enter(Map::ViridianForest),
            PolicyStep::enter(Map::ViridianForestNorthGate),
            PolicyStep::enter(Map::Route2),
            PolicyStep::enter(Map::PewterCity),
        ]
    )
    .with_coverage();

    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::PewterCity, "agent should have navigated to Pewter City");

    // A defect here is the agent failing to do something it had already committed to — a route it
    // could not walk, a walk that never arrived, a menu row it offered and could not execute.
    let log = fixture.coverage.as_ref().expect("coverage was asked for");
    println!("[coverage] can_navigate_to_pewter_city: {}", log.summary());
    let hard: Vec<&str> = log
        .entries()
        .filter(|entry| matches!(entry.verdict, super::coverage::Verdict::Defect { .. }))
        .map(|entry| entry.id.as_str())
        .collect();
    assert!(hard.is_empty(), "the agent could not execute what it chose: {hard:?}\n{}", log.report());
    assert!(log.watchdog.is_empty(), "the watchdog fired on an ordinary leg: {:?}", log.watchdog);
    // And it saw something, so a log that silently observed nothing cannot pass this.
    assert!(log.len() > 5, "only {} ids across this whole leg: {}", log.len(), log.report());
}

/// Explicit Mt Moon traversal, discovered from the ROM warp graph + live sprite-resolved
/// reachability.
#[test]
fn can_navigate_mt_moon() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/mt-moon.bin"),
        Duration::from_mins(40),
        PolicyStep::mt_moon_traversal(),
    );

    fixture.pimp_pokemon();
    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::CeruleanCity, "agent should have navigated to Cerulean City");
}

/// Rebuild the root of the committed fixture chain, from a fresh save to Route 4 outside
/// Cerulean: `at-cerulean.bin`.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts at-cerulean.bin; needs GB_REGEN_FIXTURES=1"]
fn regen_at_cerulean_fixture() {
    let mut steps = PolicyStep::pallet_to_cerulean_steps();
    steps.extend(PolicyStep::mt_moon_traversal());
    // Stop exactly where `game_steps` stops, which is *inside* the Cerulean Pokémon Centre:
    // `cerulean_to_vermilion_steps` opens with `enter(CeruleanCity)`, meaning "walk out of the
    // building", and a fixture saved standing in the city instead makes that step look for a
    // transition *to* the map it is already on — measured, it walked back out to Route 4 and
    // stalled trying to reach Route 24 from there.
    steps.extend([
        PolicyStep::enter(Map::CeruleanPokecenter),
        PolicyStep::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
        PolicyStep::enter(Map::CeruleanCity),
    ]);
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(240),
        steps,
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("ended on {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    assert_eq!(s.map.map, Map::CeruleanCity, "should end back out in Cerulean City");
    // Assert the *heal*, not just the walk.
    for mon in s.pokemon.iter() {
        assert_eq!(mon.current_hp, mon.stats.hp, "{} should be healed", mon.species);
        for mv in mon.moves.iter().flatten() {
            assert_eq!(mv.pp, mv.name.metadata().pp, "{}'s {} should be at full PP", mon.species, mv.name);
        }
    }
    assert!(s.badges.contains(Badge::BoulderBadge), "should be holding the Boulder Badge");
    fixture.save_state_named("src/pokemon/data/at-cerulean.bin").unwrap();
}

/// From `at-cerulean.bin` (out of Mt Moon, Boulder Badge, no Cascade yet), the whole middle of
/// the early game: Nugget Bridge → Bill (SS Ticket) → back → Misty → trashed-house bridge → the
/// Route 25 Oddish → Route 5 → Vermilion.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_vermilion() {
    // Exactly the leg folded into `complete_game_steps` (Bill/SS-Ticket → trashed-house bridge →
    // Vermilion), so this test and the full playthrough stay in lockstep.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-cerulean.bin"),
        Duration::from_mins(40),
        PolicyStep::cerulean_to_vermilion_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("bag: {:?}", s.bag.iter().collect::<Vec<_>>());
    assert!(s.bag.contains(&ItemId::SSTicket), "should have obtained the SS Ticket from Bill");
    assert_eq!(s.map.map, Map::VermilionCity, "should reach Vermilion City via the trashed-house bridge");
    fixture.save_state_named("src/pokemon/data/at-vermilion.bin").unwrap();
}

/// The S.S.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn the_action_menu_alone_gets_the_ss_ticket_from_bill() {
    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::map::MapSprite;
    use crate::pokemon::policy::{DeterministicPolicy, FieldMove, Policy};
    use crate::pokemon::tile::{HiddenObject, MetaTile};
    use crate::pokemon::world_graph::WorldGraph;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Arc;

    // 0 = not sampled yet, 1 = the row was absent, 2 = the row was already there.
    const UNSAMPLED: u8 = 0;
    const ABSENT: u8 = 1;
    const PRESENT: u8 = 2;

    /// Everything but the separator is the scripted route; the separator is the menu row.
    struct MenuPressesTheSeparator {
        inner: DeterministicPolicy,
        /// The negative half, and it has to be sampled before the conversation.
        row_before_talking: Arc<AtomicU8>,
        pressed: bool,
    }

    impl MenuPressesTheSeparator {
        fn separator(state: &GameState) -> Option<OverworldAction> {
            state.map.actions().into_iter()
                .find(|action| matches!(action.tile,
                    MetaTile::Switch { object: HiddenObject::CellSeparator, .. }))
        }
    }

    impl Policy for MenuPressesTheSeparator {
        fn name(&self) -> &'static str { "menu-separator" }

        fn pick_overworld_action(&mut self, state: &GameState, graph: &WorldGraph) -> Option<OverworldAction> {
            if state.map.map == Map::BillsHouse {
                let offered = Self::separator(state).is_some();
                let _ = self.row_before_talking.compare_exchange(
                    UNSAMPLED,
                    match offered { true => PRESENT, false => ABSENT },
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
                if !self.pressed && offered {
                    self.pressed = true;
                    return Self::separator(state);
                }
            }
            self.inner.pick_overworld_action(state, graph)
        }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            self.inner.pick_battle_action(state)
        }

        fn pick_field_move(&mut self, state: &GameState) -> Option<FieldMove> {
            self.inner.pick_field_move(state)
        }

        fn is_exhausted(&self) -> bool { self.pressed && self.inner.is_exhausted() }
    }

    // The leg's own route to Bill and its eight talks afterwards — but no `UsePc` between them.
    let mut steps = vec![
        PolicyStep::enter(Map::CeruleanCity),
        PolicyStep::enter(Map::Route24),
        PolicyStep::enter(Map::Route25),
        PolicyStep::enter(Map::BillsHouse),
        PolicyStep::Interact(MapSprite::BILLSHOUSE_BILL_POKEMON),
    ];
    steps.extend(std::iter::repeat_n(PolicyStep::Interact(MapSprite::BILLSHOUSE_BILL1), 8));

    let row_before_talking = Arc::new(AtomicU8::new(UNSAMPLED));
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/at-cerulean.bin"),
        Duration::from_mins(30),
        Box::new(MenuPressesTheSeparator {
            inner: DeterministicPolicy::new(42, steps),
            row_before_talking: Arc::clone(&row_before_talking),
            pressed: false,
        }),
    );
    let state = fixture.run_until(|state| state.bag.contains(&ItemId::SSTicket));
    assert!(state.bag.contains(&ItemId::SSTicket), "the ticket came out of the action menu");
    // Without this the test passes with the gate deleted, which is the whole reason it is here:
    // an ungated row is offered on arrival, pressed into a storage menu that does nothing, and
    // the run merely takes longer to reach the same ticket.
    assert_eq!(
        row_before_talking.load(Ordering::Relaxed),
        ABSENT,
        "the separator must not be on the menu before Bill has asked for it — outside that window \
         the same tile is a storage PC, which is the row `overworld_menu` withholds",
    );
}
