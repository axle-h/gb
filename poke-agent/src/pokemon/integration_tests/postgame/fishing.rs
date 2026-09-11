use super::super::*;

use crate::pokemon::postgame::fishing::{FishGoal, Rod};

const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_old_rod() {
    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(20), PolicyStep::old_rod_steps());

    assert!(!fixture.game_state().bag.iter().any(|i| i.id == ItemId::OldRod), "entry fixture already has a rod");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::OldRod));
    // Outdoors: the leg walks back out so the next leg's `Fly` is not refused.
    assert_eq!(state.map.map, Map::VermilionCity);
    println!("old rod in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-old-rod.bin").unwrap();
}

const OLD_ROD: &[u8] = include_bytes!("../../data/postgame-old-rod.bin");

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_fish_a_wild_battle_out_of_the_water() {
    const CASTS: u32 = 2;
    let mut fixture = TestFixture::new(OLD_ROD, Duration::from_mins(20),
        PolicyStep::fish_at_pallet_steps(Rod::Old, FishGoal::Casts(CASTS)));

    // `run_until`'s predicate is `Fn`, so the running tally lives in `Cell`s.
    let battles = std::cell::Cell::new(0u32);
    let in_battle = std::cell::Cell::new(false);
    fixture.run_until(|s| {
        // Count battle entries, not ticks.
        let now = s.battle.is_some();
        if now && !in_battle.get() {
            battles.set(battles.get() + 1);
            let enemy = &s.battle.as_ref().unwrap().enemy;
            println!("  bite #{}: {:?} lv{}", battles.get(), enemy.species, enemy.level);
        }
        in_battle.set(now);
        battles.get() >= CASTS && !now
    });

    assert_eq!(battles.get(), CASTS, "the Old Rod bites on every cast, so every cast should be a battle");
    let state = fixture.game_state();
    assert_eq!(state.map.map, Map::PalletTown, "the session should end back on the beach");
    assert!(state.pokedex_seen.contains(&PokemonSpecies::Magikarp));
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_a_magikarp_on_the_old_rod() {
    let goal = FishGoal::Catch { species: PokemonSpecies::Magikarp, max_casts: 20 };
    let mut fixture = TestFixture::new(OLD_ROD, Duration::from_mins(30),
        PolicyStep::fish_at_pallet_steps(Rod::Old, goal));

    let before = fixture.game_state();
    assert!(!before.pokedex_owned.contains(&PokemonSpecies::Magikarp));
    assert_eq!(before.pokemon.len(), 4, "the catch needs a free party slot");

    // Wait on the party, not the dex bit.
    let state = fixture.run_until(|s| s.pokemon.len() == 5);
    println!("caught a Magikarp — party is now {:?}",
        state.pokemon.iter().map(|p| p.species).collect::<Vec<_>>());
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Magikarp));
    assert_eq!(state.pokemon[4].species, PokemonSpecies::Magikarp);

    // Let the battle finish unwinding before snapshotting.
    fixture.run_until(|s| s.battle.is_none() && s.map.map == Map::PalletTown);
    fixture.save_state_named("src/pokemon/data/postgame-magikarp.bin").unwrap();
}

const MAGIKARP: &[u8] = include_bytes!("../../data/postgame-magikarp.bin");

/// The Good Rod, and proof that it opens a different table.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_good_rod_and_catch_a_goldeen() {
    let goal = FishGoal::Catch { species: PokemonSpecies::Goldeen, max_casts: 60 };
    let mut steps = PolicyStep::good_rod_steps();
    steps.extend(PolicyStep::fish_at_pallet_steps(Rod::Good, goal));
    let mut fixture = TestFixture::new(MAGIKARP, Duration::from_mins(60), steps);

    assert!(!fixture.game_state().bag.iter().any(|i| i.id == ItemId::GoodRod));

    let state = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::GoodRod));
    println!("good rod in the bag on {}", state.map.map);

    let state = fixture.run_until(|s| s.pokemon.len() == 6);
    println!("party is now {:?}", state.pokemon.iter().map(|p| p.species).collect::<Vec<_>>());
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Goldeen));
    assert_eq!(state.pokemon[5].species, PokemonSpecies::Goldeen);

    fixture.run_until(|s| s.battle.is_none() && s.map.map == Map::PalletTown);
    fixture.save_state_named("src/pokemon/data/postgame-good-rod.bin").unwrap();
}

/// `can_get_the_good_rod_and_catch_a_goldeen`'s output: Pallet Town, both rods, the party ending
/// Magikarp and Goldeen.
const GOOD_ROD: &[u8] = include_bytes!("../../data/postgame-good-rod.bin");

/// The Super Rod, and the map-specific table it opens.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_super_rod_and_catch_a_tentacool() {
    let goal = FishGoal::Catch { species: PokemonSpecies::Tentacool, max_casts: 60 };
    let mut steps = PolicyStep::super_rod_steps();
    // Bank the Magikarp and the Goldeen back to front, so the first deposit does not renumber the
    // second.
    steps.push(PolicyStep::Fly { to: Map::ViridianCity });
    steps.push(PolicyStep::enter(Map::ViridianPokecenter));
    steps.push(PolicyStep::deposit_pokemon(5, Map::ViridianPokecenter));
    steps.push(PolicyStep::deposit_pokemon(4, Map::ViridianPokecenter));
    steps.push(PolicyStep::enter(Map::ViridianCity));
    steps.extend(PolicyStep::fish_at_pallet_steps(Rod::Super, goal));
    let mut fixture = TestFixture::new(GOOD_ROD, Duration::from_mins(90), steps);

    assert_eq!(fixture.game_state().pokemon.len(), 6);

    let state = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::SuperRod));
    println!("super rod in the bag on {}", state.map.map);

    // Wait on the box count, not the party count.
    let state = fixture.run_until(|s| s.boxed_pokemon.len() == 2);
    println!("banked the first two fish — box 1 now holds {:?}, party {:?}",
        state.boxed_pokemon.iter().map(|p| p.species).collect::<Vec<_>>(),
        state.pokemon.iter().map(|p| p.species).collect::<Vec<_>>());

    let state = fixture.run_until(|s| s.pokemon.iter().any(|p| p.species == PokemonSpecies::Tentacool));
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Tentacool));
    println!("caught a Tentacool — dex now {} owned / {} seen",
        state.pokedex_owned.species().len(), state.pokedex_seen.species().len());
    for rod in [ItemId::OldRod, ItemId::GoodRod, ItemId::SuperRod] {
        assert!(state.bag.iter().any(|i| i.id == rod), "{rod:?} should still be in the bag");
    }

    fixture.run_until(|s| s.battle.is_none() && s.map.map == Map::PalletTown);
    fixture.save_state_named("src/pokemon/data/postgame-fishing.bin").unwrap();
}

/// The action menu offers a cast with water in reach and a rod in the bag, and picking it fishes.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn the_action_menu_offers_a_cast_when_a_rod_is_in_the_bag() {
    const FISHING: &[u8] = include_bytes!("../../data/postgame-fishing.bin");
    const CASTS: u32 = 12;

    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::world_graph::WorldGraph;

    /// Take the fishing row whenever it is offered, up to `CASTS` times, and flee whatever bites.
    struct FishTheRow { casts: u32 }
    impl crate::pokemon::policy::Policy for FishTheRow {
        fn name(&self) -> &'static str { "fish-the-row" }

        fn pick_overworld_action(&mut self, state: &GameState, _: &WorldGraph) -> Option<OverworldAction> {
            if self.casts >= CASTS { return None }
            let action = state.map.actions().into_iter()
                .find(|a| matches!(a.tile, MetaTile::Fish { .. }))?;
            self.casts += 1;
            Some(action)
        }

        fn pick_battle_action(&mut self, _: &GameState) -> Option<BattleAction> {
            Some(BattleAction::Run)
        }
    }

    let mut fixture = TestFixture::with_policy(FISHING, Duration::from_mins(30),
        Box::new(FishTheRow { casts: 0 }));

    // The row is there before anything is driven, and names the best rod.
    let offered = fixture.game_state().map.actions().into_iter()
        .find(|a| matches!(a.tile, MetaTile::Fish { .. }))
        .expect("Pallet's beach with three rods in the bag should offer a cast");
    assert_eq!(offered.tile, MetaTile::Fish { rod: Rod::Super },
        "the row should carry the best rod in the bag");
    assert_eq!(offered.to_string(), "Fish with the Super Rod");

    // ── The LLM path ──
    let id = crate::llm::tools::overworld_id(&fixture.game_state(), &offered);
    let menu = crate::llm::tools::overworld_menu(&fixture.game_state(), None);
    let row = menu.iter().find(|item| item.id == id)
        .expect("the fishing row should survive into the menu the model is sent");
    println!("the model is offered: `{id}` — {}", row.description);
    assert!(row.description.contains("fish"), "the row should say what it is for: {}", row.description);
    assert_eq!(crate::llm::tools::resolve_overworld(&fixture.game_state(), &id).as_ref(), Some(&offered),
        "the id the model would quote back should re-resolve to the same action");

    // Casting is all this policy does, so any wild battle came out of the water.
    let bites = std::cell::Cell::new(0u32);
    let in_battle = std::cell::Cell::new(false);
    let seen = std::cell::RefCell::new(Vec::new());
    fixture.run_until(|s| {
        let now = s.battle.is_some();
        if now && !in_battle.get() {
            bites.set(bites.get() + 1);
            let enemy = &s.battle.as_ref().unwrap().enemy;
            seen.borrow_mut().push((enemy.species, enemy.level));
        }
        in_battle.set(now);
        bites.get() >= 2 && !now
    });

    println!("fished up {:?}", seen.borrow());
    assert!(bites.get() >= 2, "the row should keep producing wild battles");
    assert_eq!(fixture.game_state().map.map, Map::PalletTown, "fishing does not move the player");

    // Every cast says what it did.
    let mut outcomes = 0;
    for event in fixture.agent.drain_events() {
        match event {
            AgentEvent::OverworldActionCompleted { destination: MetaTile::Fish { .. } } => outcomes += 1,
            AgentEvent::OverworldActionAborted {
                destination: MetaTile::Fish { .. },
                reason: crate::pokemon::agent::OverworldActionAbortedReason::Battle, .. } => outcomes += 1,
            _ => {}
        }
    }
    assert!(outcomes > 0, "a cast must report an outcome; every one of them was silent");
    println!("{outcomes} of the casts reported an outcome");
}
