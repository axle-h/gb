//! The whole game, start to finish, in one run.

use super::*;

/// The full end-to-end playthrough — the single source of truth for how far the agent can play. From a
/// fresh `RedsHouse2F` save it plays legitimately (button input only, starting from the **lone starter**)
/// and earns **all 8 gym badges**: Boulder → Cascade → Thunder → Rainbow → (Silph Scope → Poké Flute →
/// Snorlax) → Soul → (Safari Surf/Strength → Eevee→Vaporeon+Surf) → Silph Co (Card Key → rival → Giovanni
/// → liberation) → Marsh → surf to Cinnabar → Pokémon Mansion Secret Key → Volcano → back to the Viridian
/// Gym for **Earth** (Giovanni), with the **Seafoam Islands** detour for Articuno slotted in between
/// Volcano and Earth.
///
/// Nothing is caught before then: the run is carried by the starter and the free Celadon **Eevee**,
/// evolved to **Vaporeon** (its Surf counters the Silph rival's Alakazam / Blaine's Fire / Giovanni's
/// Ground, and ferries the party across Route 21), because a weak extra mon breaks the black-out
/// recovery the early dungeons rely on. Seafoam then adds a **Slowpoke** HM-slave (Strength + Dig) and
/// **Articuno**.
///
/// It emulates every frame, so even in `--release` it takes ~20 min of wall clock — hence its own
/// feature gate, separate from the leg chain:
/// `cargo test --release --features full-playthrough full_playthrough`. The per-leg tests (each seeded
/// from a saved fixture) cover the same ground quickly and in parallel.
#[test]
#[cfg_attr(not(feature = "full-playthrough"), ignore = "full playthrough; run with --features full-playthrough")]
fn full_playthrough() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::complete_game_steps(),
    );

    {
        let state = fixture.game_state();
        assert_eq!(state.map.map, Map::RedsHouse2F, "save state should be in RedsHouse2F");
        assert_eq!(state.pokemon.len(), 0, "player should have no pokemon before Oak's script");
    }

    fixture.step_until_exhausted();

    let state = fixture.game_state();
    for pokemon in state.pokemon.iter() {
        println!("{}: {} lv.{}", pokemon.species, pokemon.nickname, pokemon.level);
    }
    println!("badges: {:?}", state.badges);
    println!("map: {:?}", state.map.map);
    println!("money: {}  bag: {:?}", state.money, state.bag.iter().collect::<Vec<_>>());

    assert!(state.badges.contains(Badge::BoulderBadge), "should have the Boulder Badge");
    assert!(state.badges.contains(Badge::CascadeBadge), "should have the Cascade Badge");
    assert!(state.badges.contains(Badge::ThunderBadge), "should have the Thunder Badge");
    assert!(state.badges.contains(Badge::RainbowBadge), "should have the Rainbow Badge");
    // Post-Rainbow: Silph Scope (Rocket Hideout) → Poké Flute → Snorlax → Soul Badge (Koga).
    assert!(state.bag.contains(&ItemId::SilphScope), "should have the Silph Scope");
    assert!(state.bag.contains(&ItemId::PokeFlute), "should have the Poké Flute");
    assert!(state.badges.contains(Badge::SoulBadge), "should have the Soul Badge");
    // Post-Soul: Safari HMs → Vaporeon → Silph (Marsh) → Cinnabar Mansion → Volcano → Viridian (Earth).
    assert!(state.bag.contains(&ItemId::Hm03Surf), "should have HM03 Surf");
    assert!(state.badges.contains(Badge::MarshBadge), "should have the Marsh Badge");
    assert!(state.badges.contains(Badge::VolcanoBadge), "should have the Volcano Badge");
    assert!(state.badges.contains(Badge::EarthBadge), "should have the Earth Badge (all 8 gym badges)");

    // The starter and the one free Eevee (evolved to Vaporeon for Surf) carry the whole run — nothing
    // is caught before Seafoam, because a weak extra mon blocks the black-out recovery that clears the
    // early attrition dungeons. Seafoam then adds the two the Elite Four needs: a Slowpoke HM-slave
    // (Strength for Victory Road, Dig for the way out of the islands) and Articuno.
    assert!(state.pokemon.len() >= 4, "party should have the starter + Vaporeon + the Slowpoke slave + Articuno");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Vaporeon), "should have a Vaporeon");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Articuno),
        "should have caught Articuno in the Seafoam Islands");

    // Post-Earth: Victory Road 1F — caught a Machop HM-slave, taught it Strength, solved the boulder
    // puzzle (a real push onto the (17,13) switch) and climbed to VR2F. The full VR2F/VR3F puzzle and
    // the Elite Four are validated separately (`endgame::can_solve_victory_road_2f_3f`,
    // `endgame::can_beat_elite_four`) — chaining them here is PP-marginal for this team.
    assert!(state.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Strength)),
        "a party member should know Strength (the Seafoam Slowpoke, and the Victory Road Machop)");
    assert_eq!(state.map.map, Map::VictoryRoad2F, "should have solved VR1F and climbed to Victory Road 2F");

    fixture.save_state_named("src/pokemon/data/post-victory-road-1f.bin").unwrap();
}
