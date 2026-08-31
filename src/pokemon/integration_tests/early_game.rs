//! Pallet/Viridian → Pewter → Mt Moon → Cerulean → Vermilion.
//!
//! The two navigation tests here are in the fast tier: between them they cover forward `EnterMap`
//! chaining across connections and the warp-graph traversal of a fragmented dungeon, which is the
//! machinery every later leg is built on. If they pass, a failure further down the chain is about
//! that leg rather than about routing.

use super::*;

#[test]
fn can_navigate_to_pewter_city() {
    // Explicit forward navigation (Viridian City → Viridian Forest → Pewter City), the same
    // single-hop `EnterMap` chain `complete_game_steps` uses. The abstract `goto` form this test
    // used previously needed the deleted pre-built world graph.
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
    );

    fixture.step_until_exhausted();

    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::PewterCity, "agent should have navigated to Pewter City");
}

/// Explicit Mt Moon traversal, discovered from the ROM warp graph + live sprite-resolved
/// reachability. Mt Moon's floors are fragmented into disjoint walkable components joined only by
/// warps; the sole route to the Route 4 east exit crosses B2F between the (21,17) and (5,7) warps,
/// which is plugged by the two fossil item-sprites. Collecting one fossil (which also triggers the
/// mandatory Super Nerd battle and makes him grab the other fossil) opens the 1-wide passage.
///
///   1F(5,5)→B1F(5,5) [comp A] → walk → B1F(21,17)→B2F(21,17)
///     → collect Helix Fossil (beat Super Nerd, corridor opens)
///     → walk → B2F(5,7)→B1F(23,3) [comp D] → walk → B1F(27,3)→Route4 → Cerulean
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

/// **Rebuild the root of the committed fixture chain**, from a fresh save to Route 4 outside
/// Cerulean: `at-cerulean.bin`.
///
/// ⚠️ **Every fixture in this repo descends from one state no test produced, and that is fine right
/// up until the mainline party changes.** Swapping the starter changes all of them at once — a leg
/// that teaches an HM to the starter, grinds a caught mon or leads with one resolves against a party
/// the old root does not have, and simply waits for ever. So the root has a producer now.
///
/// Only under `regen-fixtures`, because it plays about a fifth of the game (~90 s) and asserts
/// nothing the default tier's `can_navigate_to_pewter_city` and `can_navigate_mt_moon` do not.
/// Regenerate the chain in order from here — see the `test-suite` skill.
#[test]
#[cfg(feature = "regen-fixtures")]
fn regen_at_cerulean_fixture() {
    let mut steps = PolicyStep::pallet_to_cerulean_steps();
    steps.extend(PolicyStep::mt_moon_traversal());
    // ⚠️ **Stop exactly where `game_steps` stops**, which is *inside* the Cerulean Pokémon Centre:
    // `cerulean_to_vermilion_steps` opens with `enter(CeruleanCity)`, meaning "walk out of the
    // building", and a fixture saved standing in the city instead makes that step look for a
    // transition *to* the map it is already on — measured, it walked back out to Route 4 and stalled
    // trying to reach Route 24 from there.
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
    // ⚠️ **Assert the *heal*, not just the walk.** `Interact` pops the moment the conversation lands,
    // which is before the nurse has finished, so a fixture cut there carries the party the run walked
    // in with. The first version of this root was saved with **Water Gun on 6 of 25 PP**, and the leg
    // seeded from it lost the Cerulean rival ambush and blacked out to the Mt Moon Centre — from where
    // a single-hop `EnterMap { Route24 }` cannot resolve and the whole chain stalled at 4 steps.
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
/// the early game: Nugget Bridge → Bill (SS Ticket) → back → **Misty** → trashed-house bridge →
/// the Route 25 Oddish → Route 5 → Vermilion.
///
/// Route 5 is unreachable from the Cerulean Pokécenter terrace directly (one-way south ledges split
/// the city; verified ROM-faithful). The real path is the **trashed-house bridge**, which only opens
/// after meeting Bill (the `CERULEANCITY_GUARD2` guard at raw (27,12) clears): enter the trashed
/// house from the main terrace, take its back door to land at Cerulean (27,9) — which IS in the
/// Route-5-reaching terrace — then walk onto Route 5. So: Nugget Bridge → Bill (SS Ticket) → return →
/// trashed-house bridge → Route 5 → Underground Path → Route 6 → Vermilion.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_reach_vermilion() {
    // Exactly the leg folded into `complete_game_steps` (Bill/SS-Ticket → trashed-house bridge →
    // Vermilion), so this test and the full playthrough stay in lockstep. It subsumes the SS Ticket
    // hand-off, which is why there is no separate test for it.
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

/// **The S.S. Ticket, taken entirely through the action menu — the one thing an `LlmPolicy` can
/// see.** `can_reach_vermilion` above proves the same errand for the *scripted* policy, and proved
/// nothing about this: it drives the cell separator with `PolicyStep::UsePc`, which resolves
/// `MetaTile::Pc` straight out of `actions()` and never goes near `llm::tools::overworld_menu`.
///
/// ⚠️ **That gap was a hard progression blocker for four months and cost a deployed run its
/// life.** `overworld_menu` withheld every `MetaTile::Pc` row — correctly, for the storage PCs it
/// was written about — and Bill's is not storage, it is one press and the only route to the ticket.
/// No ticket means `EVENT_GOT_SS_TICKET` never fires, so `BillsHouse.asm` never hides
/// `CERULEANCITY_GUARD2`, who stands on the only approach to the Trashed House door at raw (27,11)
/// — the only crossing between Cerulean's two terraces. The run of 2026-08-27 walked Cerulean and
/// Routes 24/25 for four and a half hours of cartridge time and filed six issue reports about it.
///
/// So the policy here takes the row and nothing else: `MetaTile::Switch(CellSeparator)`, exactly
/// what `overworld_menu` renders and `resolve_overworld` re-mints from an id. ⚠️ **Hand-rolled
/// rather than `DeterministicPolicy`**, which would drive the PC through `UsePc` before the agent
/// ever saw the row, and pass with the whole thing reverted.
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
        /// ⚠️ **The negative half, and it has to be sampled before the conversation.** Entering the
        /// map is a step of its own and completes first, so the first poll on `BillsHouse` is
        /// necessarily before Bill has been spoken to — which is the moment the row must *not* be
        /// there, or the gate is doing nothing and the test would pass ungated.
        /// Shared with the test, because `TestFixture` owns the policy once it is boxed.
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
    // ⚠️ Without this the test passes with the gate deleted, which is the whole reason it is here:
    // an ungated row is offered on arrival, pressed into a storage menu that does nothing, and the
    // run merely takes longer to reach the same ticket.
    assert_eq!(
        row_before_talking.load(Ordering::Relaxed),
        ABSENT,
        "the separator must not be on the menu before Bill has asked for it — outside that window \
         the same tile is a storage PC, which is the row `overworld_menu` withholds",
    );
}
