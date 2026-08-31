//! Tests for workstream `fishing` — see `docs/postgame-coverage-plan.md` §6-C and
//! [`crate::pokemon::postgame::fishing`].
//!
//! The chain is `postgame-fly-bike.bin` → `-old-rod` → `-magikarp` → `-good-rod` →
//! `postgame-fishing.bin`. It is rooted on **B's** output rather than `postgame-phase0.bin` because
//! all three rods live in three different corners of Kanto and Fly turns each of those trips into one
//! step — see the §11 entry.

#[allow(unused_imports)]
use super::super::*;

use crate::pokemon::postgame::fishing::{FishGoal, Rod};

/// Workstream B's output (§9): Fuchsia City, Fly on Articuno, the Bicycle in the bag (16/20), party
/// Venusaur / Articuno / Vaporeon / Slowpoke, ¥41,209, dex 7 owned / 112 seen.
const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// **Task C1** — the Old Rod, from the fishing guru in the Vermilion house.
///
/// Fly to Vermilion, walk in, talk. The guru's "do you like to fish?" is a `YesNoChoice` that opens on
/// YES, so the agent's generic A-mash answers it — the same shape as B1's Fan Club chairman, and again
/// no driver is needed.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_old_rod() {
    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(20), PolicyStep::old_rod_steps());

    assert!(!fixture.game_state().bag.iter().any(|i| i.id == ItemId::OldRod), "entry fixture already has a rod");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::OldRod));
    // Outdoors, not in the guru's house: the leg's last step walks back out so the next leg's `Fly`
    // is not refused for being indoors. See `postgame::fishing::rod_pickup`.
    assert_eq!(state.map.map, Map::VermilionCity);
    println!("old rod in the bag — bag now {} entries", state.bag.len());

    fixture.save_state_named("src/pokemon/data/postgame-old-rod.bin").unwrap();
}

/// C1's output: inside `VermilionOldRodHouse` with the Old Rod in the bag.
const OLD_ROD: &[u8] = include_bytes!("../../data/postgame-old-rod.bin");

/// **Task C2** — the fishing driver: cast from a water tile and land in a wild battle.
///
/// Two casts at Pallet Town with the Old Rod. Pallet is chosen because it is a Fly destination *and*
/// has a Super Rod fishing group, so C2, C3 and C5 all share one map — and its beach is the shore the
/// Route 21 surf crossing already uses, i.e. water the agent is known to be able to stand next to.
///
/// The Old Rod is the right rod to prove the *driver* with, because it removes the RNG:
/// `ItemUseOldRod` sets the bite flag unconditionally (`ld a, $1 ; set bite`,
/// `engine/items/item_effects.asm:1826-1831`), so **every** cast is a lv5 Magikarp battle. Two casts
/// therefore means two battles, and the count is the assertion — a driver that opened the bag and
/// backed out again, or that cast at a tile the ROM does not consider water, would produce zero.
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
        // Count battle *entries*, not ticks. `pokedex_seen` cannot be the observable here: this save
        // has seen 112 of 151 species and Magikarp is long since among them.
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


/// **Task C3** — catch what bites: a Magikarp out of Pallet Town's water.
///
/// `Catch` throws at the target and flees everything else, with **no weakening pass** — Magikarp is
/// catch rate 255, so a Poké Ball is better than a coin flip, and the only realistic way to lose the
/// encounter is a stray critical hit from a lv73 Articuno. (Same reasoning as
/// `legendaries::pre_catch_action`, from the opposite end of the catch-rate range.) The 20-cast bound
/// is far more than the ~2 the arithmetic wants; it is there so a broken rod fails in a minute
/// instead of running to the budget.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_a_magikarp_on_the_old_rod() {
    let goal = FishGoal::Catch { species: PokemonSpecies::Magikarp, max_casts: 20 };
    let mut fixture = TestFixture::new(OLD_ROD, Duration::from_mins(30),
        PolicyStep::fish_at_pallet_steps(Rod::Old, goal));

    let before = fixture.game_state();
    assert!(!before.pokedex_owned.contains(&PokemonSpecies::Magikarp));
    assert_eq!(before.pokemon.len(), 4, "the catch needs a free party slot");

    // Wait on the **party**, not the dex bit. `ItemUseBall` sets `wPokedexOwned` inside the catch
    // routine, several seconds of text and a nickname screen before the mon is actually added, so a
    // `run_until` on the dex returns with the party still four long.
    let state = fixture.run_until(|s| s.pokemon.len() == 5);
    println!("caught a Magikarp — party is now {:?}",
        state.pokemon.iter().map(|p| p.species).collect::<Vec<_>>());
    assert!(state.pokedex_owned.contains(&PokemonSpecies::Magikarp));
    assert_eq!(state.pokemon[4].species, PokemonSpecies::Magikarp);

    // And let the battle finish unwinding before snapshotting.
    fixture.run_until(|s| s.battle.is_none() && s.map.map == Map::PalletTown);
    fixture.save_state_named("src/pokemon/data/postgame-magikarp.bin").unwrap();
}

/// C3's output: Pallet Town's beach, Old Rod in the bag, a Magikarp in party slot 5.
const MAGIKARP: &[u8] = include_bytes!("../../data/postgame-magikarp.bin");

/// **Task C4** — the Good Rod, then proof that it opens a different table.
///
/// The rod is one Fly, one door and one talk from the Fuchsia guru. The proof is **Goldeen**: the
/// Good Rod picks uniformly between Goldeen and Poliwag (`data/wild/good_rod.asm`) and the Old Rod's
/// table is the single species Magikarp, so a Goldeen caught at Pallet cannot have come from any
/// other rod. Half of casts get no bite and half of the bites are Poliwag, so ~4 casts per Goldeen
/// and the bound is generous.
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

/// C4's output: Pallet Town, both rods in the bag, party of six ending Magikarp / Goldeen.
const GOOD_ROD: &[u8] = include_bytes!("../../data/postgame-good-rod.bin");

/// **Task C5** — the Super Rod, and the map-specific table it opens.
///
/// Route 12 is not a Fly destination, so this leg flies to Lavender and walks south to the guru's
/// house. The catch target is **Tentacool**, which at Pallet can only have come from the Super Rod:
/// `SuperRodData` maps `PALLET_TOWN` to `.Group1` (Tentacool / Poliwag, `data/wild/super_rod.asm:4`,
/// `:39-42`), while the Old Rod is Magikarp-only and the Good Rod is Goldeen/Poliwag everywhere.
///
/// **The party is emptied out first**, and that is not tidiness. With six in the party a caught
/// Pokémon goes to the box, and the nickname screen on *that* path wedges the agent — the bug
/// workstream D hit twice on Mewtwo (§11). So the two fish caught so far are banked at the Viridian
/// PC with workstream A's `deposit_pokemon`, which is also the first time a fishing session and box
/// storage meet.
///
/// Commit target: `postgame-fishing.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_super_rod_and_catch_a_tentacool() {
    let goal = FishGoal::Catch { species: PokemonSpecies::Tentacool, max_casts: 60 };
    let mut steps = PolicyStep::super_rod_steps();
    // Bank the Magikarp and the Goldeen, back to front so the first deposit does not renumber the
    // second. The `enter` steps are explicit because the world graph is built as the agent walks and
    // starts empty in every test, so `route_toward(ViridianPokecenter)` has nothing to route over.
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

    // Wait on the **box count**, not the party count. `wPartyCount` dips to its post-deposit value
    // partway through the box menus, while the mon list on screen still shows the old party, so a
    // `run_until(party == 4)` returns in the middle of the first deposit rather than after the second.
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

/// **The fishing row in the action menu** — water within reach plus a rod in the bag, taken by a
/// policy that knows nothing about fishing beyond picking the row.
///
/// That last part is the point. `PolicyStep::Fish` drives a cast from a *scripted* queue, which is no
/// use to the model: `LlmPolicy` chooses from `MetaTileMap::actions`, so until there was a row there
/// the whole mechanic was unreachable for a run that was not following a script. The row carries the
/// best rod in the bag (`Rod::best_in_bag` — the earlier two are strictly worse, so there is nothing
/// to choose between), and the hand-off happens in the agent rather than in any policy, so a random,
/// scripted or model-driven run all fish the same way.
///
/// `postgame-fishing.bin` stands on the Pallet beach with all three rods, which is why the assertion
/// below is `Rod::Super` rather than "a rod".
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

    // The row is there before anything is driven, and it names the best rod rather than the first.
    let offered = fixture.game_state().map.actions().into_iter()
        .find(|a| matches!(a.tile, MetaTile::Fish { .. }))
        .expect("Pallet's beach with three rods in the bag should offer a cast");
    assert_eq!(offered.tile, MetaTile::Fish { rod: Rod::Super },
        "the row should carry the best rod in the bag");
    assert_eq!(offered.to_string(), "Fish with the Super Rod");

    // ── The LLM path, which is the only reason this row exists ──
    //
    // `LlmPolicy` never sees an `OverworldAction`: it is sent `overworld_menu`, answers with an id,
    // and `resolve_overworld` turns that back into an action by **string equality** on a freshly
    // recomputed list. So the three things that can silently break a row for the model and for
    // nothing else are that the menu drops it (`overworld_menu` withholds `MetaTile::Pc`), that the
    // id does not round-trip, and that it has no description — a row the model cannot tell the
    // purpose of is one it does not pick.
    let id = crate::llm::tools::overworld_id(&fixture.game_state(), &offered);
    let menu = crate::llm::tools::overworld_menu(&fixture.game_state(), None);
    let row = menu.iter().find(|item| item.id == id)
        .expect("the fishing row should survive into the menu the model is sent");
    println!("the model is offered: `{id}` — {}", row.description);
    assert!(row.description.contains("fish"), "the row should say what it is for: {}", row.description);
    assert_eq!(crate::llm::tools::resolve_overworld(&fixture.game_state(), &id).as_ref(), Some(&offered),
        "the id the model would quote back should re-resolve to the same action");

    // Casting is the only thing this policy does, so any wild battle at all came out of the water.
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
}
