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

/// Fetch the free **Eevee** (Celadon Mansion roof house), buy a **Water Stone** (Dept Store 4F), evolve
/// Eevee → **Vaporeon** and teach it **Surf**. The lone Grass starter cannot learn Surf, and Surf is
/// what reaches Cinnabar — so this leg is what makes the back half of the game possible at all. Ends
/// back in Saffron for Silph Co.
///
/// This was `#[ignore]`d for a long time on the theory that the fixture was stale and a regeneration
/// would fix it. It would not have. `eevee_vaporeon_surf_steps` used to address the party by *slot* —
/// evolve slot 1, teach Surf to slot 1 — assuming the gift Eevee appends right after the lone starter.
/// `at-saffron.bin` still carries the early Route-1 Pidgey at slot 1, so the Water Stone evolved
/// nothing and `TeachMove` spun forever on a Pidgey, which cannot learn Surf. But the Pidgey enters the
/// chain at `post-cascade.bin`, a committed **root** no test produces, so re-deriving `at-saffron.bin`
/// would have re-derived the Pidgey with it. The fix was [`PartyRef::Species`]: the leg now names the
/// Eevee and the Vaporeon it becomes, and does not care what else is in the party or where.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_vaporeon() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-saffron.bin"),
        Duration::from_mins(60),
        PolicyStep::eevee_vaporeon_surf_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    let vaporeon = s.pokemon.iter()
        .find(|p| p.species == PokemonSpecies::Vaporeon)
        .expect("Eevee should have evolved into Vaporeon with the Water Stone");
    println!("ended on {} @ {} — Vaporeon lv{} {:?}", s.map.map, s.map.player_position,
        vaporeon.level, vaporeon.moves);
    assert!(s.can_use_surf, "Vaporeon should know Surf after TeachMove");
    assert_eq!(s.map.map, Map::SaffronCity, "should end back in Saffron for Silph Co");
    fixture.save_state_named("src/pokemon/data/vaporeon-ready.bin").unwrap();
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
/// Seeded from `vaporeon-ready.bin`, not `at-saffron.bin`: `complete_game_steps` fetches Vaporeon
/// *before* Silph so its Surf can answer the 7F rival, and this leg used to start one step upstream of
/// that — which is why `silph-card-key.bin` reached [`can_beat_silph_giovanni`] with no Vaporeon in it
/// and left that test unwinnable. The leg chain now matches the mainline ordering.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_silph_card_key() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vaporeon-ready.bin"),
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
    // `at-saffron-post-silph.bin`, not `post-silph-giovanni.bin`: the latter is snapshotted the moment
    // Giovanni's after-battle script completes, with the player still on Silph 11F, so the gym leg has
    // nowhere to start from. This one is the same run threaded back out to a liberated Saffron.
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-saffron-post-silph.bin"),
        Duration::from_mins(30),
        PolicyStep::marsh_badge_steps(),
    );
    let s = fixture.run_until(|s| s.badges.contains(Badge::MarshBadge));
    println!("final: map={} @ {} party0 lv{}", s.map.map, s.map.player_position,
        s.pokemon.get(0).map(|p| p.level).unwrap_or(0));
    fixture.save_state_named("src/pokemon/data/post-marsh-badge.bin").unwrap();
}
