//! Tests for workstream `aides` — see `docs/postgame-coverage-plan.md` §6-H and
//! [`crate::pokemon::postgame::aides`].
//!
//! Rooted on **G-trades' output**, which is where the dex count comes from: the three aides are gated
//! at **10 / 30 / 50 owned**, and this chain sits at 19.

#[allow(unused_imports)]
use super::super::*;

/// G-trades' output (§9): Cinnabar Island, party Venusaur / Articuno / Vaporeon / Tangela, eight mons
/// in box 1, **dex 19 owned**, bag 20/20.
const TANGELA: &[u8] = include_bytes!("../../data/postgame-tangela.bin");

/// **Tasks H1 + H2** — **HM05 Flash** from the Route 2 Gate aide, taught, and used to light a cave.
///
/// H1's gate is dex **10 owned** and this chain has 19, so the aide hands it over on sight — G-gifts
/// and G-trades are what unblocked this row. The three things that actually needed care are in
/// [`PolicyStep::flash_steps`]; the one worth repeating is that **only Slowpoke and Mr. Mime** can
/// learn Flash out of everything this save has owned, and Slowpoke is in the box.
///
/// H2's observable is the plan's own — "a dark cave renders lit" — read where the ROM keeps it:
/// entering Rock Tunnel 1F sets `wMapPalOffset = 6` and the Flash field move is the only thing that
/// clears it, so `GameState::map_is_dark` flipping is exactly the assertion.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_hm05_and_light_rock_tunnel() {
    /// Slowpoke, banked by G-trades — box slot 4, and one of two Flash learners this save has.
    const SLOWPOKE_BOX_SLOT: u8 = 4;
    /// It lands at the end of a four-mon party.
    const FLASH_SLOT: u8 = 4;
    /// The bag is 20/20; an Escape Rope is the least useful thing in it (Slowpoke knows Dig).
    const SHED: ItemId = ItemId::EscapeRope;

    let mut fixture = TestFixture::new(TANGELA, Duration::from_mins(90),
        PolicyStep::flash_steps(Some(SLOWPOKE_BOX_SLOT), SHED, FLASH_SLOT));

    let before = fixture.game_state();
    assert!(before.pokedex_owned.species().len() >= 10, "H1's aide wants 10 species owned");
    assert!(!before.bag.iter().any(|i| i.id == ItemId::Hm05Flash), "entry fixture already has HM05");
    assert_eq!(before.boxed_pokemon[SLOWPOKE_BOX_SLOT as usize].species, PokemonSpecies::Slowpoke,
        "box slot {SLOWPOKE_BOX_SLOT} should be the Flash learner");

    let with_hm = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::Hm05Flash));
    println!("HM05 in the bag at {:?}", with_hm.map.map);

    let taught = fixture.run_until(|s| s.pokemon.get(FLASH_SLOT as usize).is_some_and(|p|
        p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Flash)));
    assert_eq!(taught.pokemon[FLASH_SLOT as usize].species, PokemonSpecies::Slowpoke);
    println!("Flash taught to slot {FLASH_SLOT}");

    // Entering Rock Tunnel is what makes the map dark in the first place; assert that, or "lit"
    // proves nothing.
    let dark = fixture.run_until(|s| s.map.map == Map::RockTunnel1F && s.map_is_dark);
    assert!(dark.map_is_dark, "Rock Tunnel 1F should be dark on arrival (wMapPalOffset = 6)");

    let state = fixture.run_leg(|s| s.map.map == Map::RockTunnel1F && !s.map_is_dark);
    assert!(!state.map_is_dark, "Flash should have cleared wMapPalOffset");
    println!("Rock Tunnel lit · dex owned {}", state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-flash.bin").unwrap();
}
