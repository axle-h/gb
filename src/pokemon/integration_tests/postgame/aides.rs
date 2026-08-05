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

/// E's output (§9): Fuchsia City, party 6, box 1 holding 18, **dex 31 owned / 116 seen** — which is
/// what takes H3 past the Itemfinder aide's gate of 30. Bag is 20/20, so H3 has to shed first.
const SAFARI: &[u8] = include_bytes!("../../data/postgame-safari.bin");

/// **Task H3** — the **Itemfinder** from the `Route11Gate2F` aide, at a dex gate of 30 owned.
///
/// This is the row **E** unblocked: the gate wants 30 species and the chain sat at 19 until the
/// Safari sweep took it to 31. Nothing here is conditional beyond arriving with a free bag slot —
/// which is the one thing that would fail *silently*, since `OaksAideScript` refuses a full bag with
/// a single text box and an ordinary goodbye. See [`PolicyStep::itemfinder_steps`].
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_itemfinder() {
    /// Two vitamins: the least useful things in a 20/20 bag whose party is already at level 71+.
    /// Two rather than one so [`can_collect_a_hidden_item`] inherits a free slot.
    const SHED: &[ItemId] = &[ItemId::Calcium, ItemId::Carbos];

    let mut fixture = TestFixture::new(SAFARI, Duration::from_mins(60),
        PolicyStep::itemfinder_steps(SHED));

    let before = fixture.game_state();
    assert!(before.pokedex_owned.species().len() >= 30, "H3's aide wants 30 species owned");
    assert!(!before.bag.iter().any(|i| i.id == ItemId::Itemfinder), "entry fixture already has it");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::Itemfinder));
    for shed in SHED {
        assert!(!state.bag.iter().any(|i| i.id == *shed), "{shed:?} should have gone to the PC");
    }
    assert_eq!(state.map.map, Map::Route11, "the leg ends outdoors so the next Fly is allowed");
    // Not a bag *occupancy* count — `GameState::bag` drops every id `ItemId` cannot name (§10), so it
    // under-reports against the 20-slot ceiling. `probe_coverage` reads `wNumBagItems` for that.
    println!("Itemfinder in the bag · dex owned {} · named bag items {}",
        state.pokedex_owned.species().len(), state.bag.iter().count());

    fixture.save_state_named("src/pokemon/data/postgame-itemfinder.bin").unwrap();
}

/// H3's output: Route 11 at (50,8), just outside the gate's west door, Itemfinder in the bag and one
/// free slot left over for what H4 picks up.
const ITEMFINDER: &[u8] = include_bytes!("../../data/postgame-itemfinder.bin");

/// **Task H4** — collect a hidden item: Route 11's **Escape Rope**, lying on a tile that looks like
/// nothing at all.
///
/// Two things §6-H4 assumed turned out not to hold, both recorded in §11. The mechanic needed **no
/// new driver** — a hidden item is collected by facing its tile and pressing A, which is
/// `FieldMove::CheckTrashCan` — and the **Itemfinder is not a prerequisite**: it only reports whether
/// something is nearby, so this leg would pass without H3. H3 still comes first because it is what
/// leaves the bag with a free slot, and a full bag here is the one failure that does not look like
/// one (see [`crate::pokemon::postgame::aides::pick`]).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_collect_a_hidden_item() {
    let mut fixture = TestFixture::new(ITEMFINDER, Duration::from_mins(30),
        PolicyStep::hidden_item_steps(Map::Route11, ItemId::EscapeRope));

    let before = fixture.game_state();
    assert!(before.bag.iter().any(|i| i.id == ItemId::Itemfinder), "H3's output should carry it");
    assert!(!before.bag.iter().any(|i| i.id == ItemId::EscapeRope), "already holding one");
    // The tile is *discovered*, not passed in: `HiddenItemCoords` says raw (48,5), and Route 11 is a
    // `WEST | EAST` map whose west connection strip shifts every raw x by one. Getting that wrong is
    // silent — the agent walks somewhere plausible, presses A and nothing happens.
    assert_eq!(before.map.hidden_items, vec![(Point8 { x: 49, y: 5 }, ItemId::EscapeRope)],
        "Route 11's hidden item, offset for the connection strip");

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::EscapeRope));
    assert_eq!(state.map.map, Map::Route11, "the leg ends outdoors so the next Fly is allowed");
    println!("hidden Escape Rope collected at {} · named bag items {}",
        state.map.player_position, state.bag.iter().count());

    fixture.save_state_named("src/pokemon/data/postgame-hidden-item.bin").unwrap();
}

/// H4's output, and H5's entry: Route 11, dex 31 owned, bag full again.
const HIDDEN_ITEM: &[u8] = include_bytes!("../../data/postgame-hidden-item.bin");

/// The share floor every H5 leg sweeps at: chase a map's fat slots, take anything rarer that happens
/// to turn up, and move on. See [`PolicyStep::dex_sweep_steps`]'s reasoning.
const MIN_SHARE: u8 = 20;

/// **Task H5a** — the sweep's outfitting, then the two grounds either side of Route 11.
///
/// Three species (Ekans, Drowzee, Diglett) for almost no travel, which makes this the leg to prove
/// the mechanism on before the ones that cross Kanto. It also does the shopping the rest of H5 needs:
/// a hundred Poké Balls and, less obviously, **an empty box** — the party is full, so every catch goes
/// to the PC, and a full box turns the sweep into an infinite retry.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sweep_the_vermilion_grounds() {
    /// One bag slot for the balls. The Rare Candy is the least useful thing in a 20/20 bag whose
    /// party is level 71.
    const SHED: &[ItemId] = &[ItemId::RareCandy];
    /// Box 1 already holds 18 of E's Safari catches; box 2 is empty.
    const BOX: u8 = 1;

    let mut steps = PolicyStep::dex_sweep_outfit_steps(SHED, 99, BOX);
    steps.extend(PolicyStep::dex_sweep_vermilion_steps(MIN_SHARE));
    let mut fixture = TestFixture::new(HIDDEN_ITEM, Duration::from_mins(300), steps);

    let before = fixture.game_state();
    let owned_before = before.pokedex_owned.species().len();
    assert_eq!(owned_before, 31, "H4's output");

    let stocked = fixture.run_until(|s| s.bag.iter().any(|i| i.id == ItemId::PokeBall && i.quantity > 50));
    println!("stocked: {} Poké Balls, ¥{}, box {}",
        stocked.bag.iter().find(|i| i.id == ItemId::PokeBall).unwrap().quantity, stocked.money,
        stocked.current_box + 1);
    assert_eq!(stocked.current_box, BOX, "the sweep needs an empty box open");

    let state = fixture.run_leg(|s| s.pokedex_owned.species().len() >= owned_before + 3);
    for species in [PokemonSpecies::Ekans, PokemonSpecies::Drowzee, PokemonSpecies::Diglett] {
        assert!(state.pokedex_owned.contains(&species), "{species:?} should be owned");
    }
    assert_eq!(state.map.map, Map::Route11, "the leg ends outdoors so the next Fly is allowed");
    println!("dex owned {} · {} balls left · ¥{}", state.pokedex_owned.species().len(),
        state.bag.iter().find(|i| i.id == ItemId::PokeBall).map_or(0, |i| i.quantity), state.money);

    fixture.save_state_named("src/pokemon/data/postgame-sweep-vermilion.bin").unwrap();
}

/// H5a's output: Route 11, box 2 open with the leg's catches in it, Poké Balls in the bag.
const SWEEP_VERMILION: &[u8] = include_bytes!("../../data/postgame-sweep-vermilion.bin");

/// **Task H5b** — Route 1 and Viridian Forest: the cheapest seven species in the game.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sweep_the_viridian_grounds() {
    let mut fixture = TestFixture::new(SWEEP_VERMILION, Duration::from_mins(300),
        PolicyStep::dex_sweep_viridian_steps(MIN_SHARE));

    let owned_before = fixture.game_state().pokedex_owned.species().len();
    let state = fixture.run_leg(|s| s.map.map == Map::Route2 && s.pokedex_owned.species().len() > owned_before);
    for species in [PokemonSpecies::Pidgey, PokemonSpecies::Rattata,
                    PokemonSpecies::Weedle, PokemonSpecies::Kakuna] {
        assert!(state.pokedex_owned.contains(&species), "{species:?} should be owned");
    }
    println!("dex owned {} (was {owned_before}) · {} balls left",
        state.pokedex_owned.species().len(),
        state.bag.iter().find(|i| i.id == ItemId::PokeBall).map_or(0, |i| i.quantity));

    fixture.save_state_named("src/pokemon/data/postgame-sweep-viridian.bin").unwrap();
}


/// H5b's output.
const SWEEP_VIRIDIAN: &[u8] = include_bytes!("../../data/postgame-sweep-viridian.bin");

/// **Task H5c** — the three Lavender grounds: Route 8, Pokémon Tower 7F, and Rock Tunnel.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_sweep_the_lavender_grounds() {
    let mut fixture = TestFixture::new(SWEEP_VIRIDIAN, Duration::from_mins(400),
        PolicyStep::dex_sweep_lavender_steps(MIN_SHARE));

    let owned_before = fixture.game_state().pokedex_owned.species().len();
    let state = fixture.run_leg(|s| s.map.map == Map::Route10 && s.pokedex_owned.species().len() > owned_before);
    for species in [PokemonSpecies::Gastly, PokemonSpecies::Zubat,
                    PokemonSpecies::Geodude, PokemonSpecies::Machop] {
        assert!(state.pokedex_owned.contains(&species), "{species:?} should be owned");
    }
    println!("dex owned {} (was {owned_before}) · {} balls left",
        state.pokedex_owned.species().len(),
        state.bag.iter().find(|i| i.id == ItemId::PokeBall).map_or(0, |i| i.quantity));

    fixture.save_state_named("src/pokemon/data/postgame-sweep-lavender.bin").unwrap();
}


/// H5c's output.
const SWEEP_LAVENDER: &[u8] = include_bytes!("../../data/postgame-sweep-lavender.bin");

/// **Task H5d/H5e** — Route 7's Oddish and the Pokémon Mansion, then the **Exp.All** aide.
///
/// The sweep's last leg and H5's observable in one: this is where the count is meant to cross 50, so
/// it takes Route 7 for the species the earlier legs did not reach, then the Mansion — the densest
/// ground a Fly lands next to — for as much margin as it needs, and finally walks into
/// `Route15Gate2F`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_the_exp_all() {
    /// Box 2 is carrying the first three legs' catches; the Mansion's would overflow it.
    const BOX: u8 = 2;
    /// ⚠️ **Free bag slots, or the aide refuses.** Exactly H3's trap, walked into again: the run
    /// reached 52 owned, the aide said *"You have caught 52 kinds of POKéMON! Congratulations!"* — and
    /// then *"Oh! I see you don't have any room for the EXP.ALL."* and said goodbye.
    ///
    /// **Two** slots, not one, and the second is the interesting one: an indoor `SweepDex` wanders by
    /// walking to the farthest reachable *sprite*, and arriving at a sprite presses A — so a floor with
    /// item balls on it, like Pokémon Mansion 1F, **collects one** as a side effect. Shedding a single
    /// slot was exactly cancelled out by that pickup, and the aide refused a second time. The Silph
    /// Scope goes back now that H5c's tower climb is done.
    const SHED: &[ItemId] = &[ItemId::MaxRevive, ItemId::SilphScope];

    let mut steps = PolicyStep::dex_sweep_outfit_steps(SHED, 57, BOX);
    steps.extend(PolicyStep::dex_sweep_celadon_steps());
    steps.extend(PolicyStep::dex_sweep_mansion_steps());
    steps.extend(PolicyStep::exp_all_steps());
    let mut fixture = TestFixture::new(SWEEP_LAVENDER, Duration::from_mins(400), steps);

    let owned_before = fixture.game_state().pokedex_owned.species().len();
    println!("entering with dex owned {owned_before}");

    let fifty = fixture.run_until(|s| s.pokedex_owned.species().len() >= 50);
    println!("dex owned {} — Exp.All's gate cleared at {:?}",
        fifty.pokedex_owned.species().len(), fifty.map.map);

    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::ExpAll));
    assert!(state.pokedex_owned.species().len() >= 50, "H5's aide wants 50 species owned");
    println!("Exp.All in the bag · dex owned {}", state.pokedex_owned.species().len());

    fixture.save_state_named("src/pokemon/data/postgame-aides.bin").unwrap();
}



/// Diagnostic for **H5** — does each planned ground actually offer the agent grass to stand in?
///
/// The lesson Route 8 taught, and the one §11 already records for Route 15: a route having grass in
/// the ROM says nothing about that grass being *reachable*. `SweepDex` with nowhere to trigger an
/// encounter does not error — it paces between the two farthest tiles on the map for its whole budget.
/// Run this before adding a ground, not after the leg times out.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_sweep_grounds() {
    use crate::pokemon::postgame::aides;
    // Navigation only — every `SweepDex` is stripped, because a sweep on a *grassless* ground is
    // exactly the non-terminating step this probe exists to predict.
    let nav = |steps: Vec<PolicyStep>| -> Vec<PolicyStep> {
        steps.into_iter().filter(|s| !matches!(s, PolicyStep::SweepDex { .. })).collect()
    };
    let grounds: &[(&str, Vec<PolicyStep>, Map)] = &[
        ("Tower 7F", nav(PolicyStep::dex_sweep_lavender_steps(MIN_SHARE)), Map::PokemonTower7F),
        ("Rock Tunnel 1F", nav(PolicyStep::dex_sweep_lavender_steps(MIN_SHARE)), Map::RockTunnel1F),
        ("Route 7", nav(PolicyStep::dex_sweep_celadon_steps()), Map::Route7),
        ("Mansion 1F", nav(PolicyStep::dex_sweep_mansion_steps()), Map::PokemonMansion1F),
    ];
    for (name, steps, map) in grounds {
        let mut fixture = TestFixture::new(SWEEP_VIRIDIAN, Duration::from_mins(90), steps.clone());
        fixture.run_until(|s| s.map.map == *map);
        for _ in 0..60 { fixture.step(); }
        let s2 = fixture.game_state();
        let grass = s2.map.actions().into_iter().find(|a| a.tile == MetaTile::Grass);
        println!("== {name} ({:?} @ {}) grass: {}", s2.map.map, s2.map.player_position,
            grass.map_or("NONE REACHABLE".to_string(), |a| format!("{} ({} steps)", a.destination, a.route.len())));
        println!("   targets left: {:?}", aides::sweep_remaining(&s2, *map, MIN_SHARE));
    }
}

/// Diagnostic for **H5** — read back the state a *stalled* leg left, and dump the map around it.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_stall_artifact() {
    let path = "target/test-artifacts/test_stall_state.bin";
    let Ok(bytes) = std::fs::read(path) else { println!("no artifact at {path}"); return };
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    println!("== {:?} @ {} ({}x{}) can_surf {} — tile {:?}",
        s.map.map, s.map.player_position, s.map.width, s.map.height, s.map.can_surf, s.map.player_tile());
    for a in s.map.actions().iter().take(14) {
        println!("   {:?} @ {} ({} steps)", a.tile, a.destination, a.route.len());
    }
    let water: Vec<Point8> = (0..s.map.height as u8).flat_map(|y| (0..s.map.width as u8).map(move |x| Point8 { x, y }))
        .filter(|&p| s.map.tile_at_checked(p) == Some(MetaTile::Water)).collect();
    println!("   {} Water tiles: {:?}", water.len(), &water[..water.len().min(20)]);
}

/// Diagnostic for **H5** — read back the state a timed-out leg left in `target/test-artifacts/`.
///
/// A stalled pacer leaves no log line and no agent state in the snapshot, so the only way to see what
/// it was looking at is to reload where it stopped and ask the map the same questions the driver did:
/// which neighbours are grass, and which of those the tile-pair table actually lets you step onto.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_timeout_artifact() {
    let path = "target/test-artifacts/test_timeout_state.bin";
    let Ok(bytes) = std::fs::read(path) else { println!("no artifact at {path}"); return };
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let s = fixture.game_state();
    let pos = s.map.player_position;
    println!("== {:?} @ {pos} facing {:?} — tile {:?}, mode {:?}",
        s.map.map, s.map.player_direction, s.map.player_tile(), s.mode);
    for (dx, dy, name) in [(0i16, -1i16, "up"), (0, 1, "down"), (-1, 0, "left"), (1, 0, "right")] {
        let n = Point8 { x: (pos.x as i16 + dx) as u8, y: (pos.y as i16 + dy) as u8 };
        println!("   {name:<6} {n} tile {:?} pair_blocked {}",
            s.map.tile_at_checked(n), s.map.pair_blocked(pos, n));
    }
    println!("   actions:");
    for a in s.map.actions().iter().take(12) {
        println!("     {:?} @ {} ({} steps)", a.tile, a.destination, a.route.len());
    }
    super::super::fixture::print_coverage("timeout artifact", &bytes);
    println!("   party: {}", s.pokemon.iter()
        .map(|p| format!("{:?} {}/{}hp [{}]", p.species, p.current_hp, p.stats.hp,
            p.moves.iter().flatten().map(|m| format!("{:?}:{}", m.name, m.pp)).collect::<Vec<_>>().join(" ")))
        .collect::<Vec<_>>().join(" | "));
}

/// Diagnostic for **H5** — is the sweep's pacing actually stepping, and are encounters firing?
///
/// A wander that never triggers a battle and a wander that never *moves* look identical from the
/// event log: `PacingForEncounters` prints one state change and then nothing for the whole budget.
/// This prints the position each second so the two can be told apart.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_sweep_pacing() {
    let mut fixture = TestFixture::new(HIDDEN_ITEM, Duration::from_mins(60), vec![
        // Enter from Vermilion, the way the real leg does — the west end of the route, not the gate
        // end H4 leaves the agent on.
        PolicyStep::Fly { to: Map::VermilionCity },
        PolicyStep::goto(Map::Route11),
        PolicyStep::SweepDex { on_map: Map::Route11, min_share: MIN_SHARE, ball: None },
    ]);
    let mut seen = std::collections::HashSet::new();
    for tick in 0..12000 {
        fixture.step();
        if tick % 250 == 0 {
            let s = fixture.game_state();
            seen.insert(s.map.player_position);
            println!("t{tick:<5} {:?} @ {} mode {:?} tile {:?} — {} distinct tiles",
                s.map.map, s.map.player_position, s.mode, s.map.player_tile(), seen.len());
        }
    }
}

/// Diagnostic for **H5** — which species are still missing, and where each is cheapest to catch.
///
/// Exp.All wants **50 owned** against 31, so the question is which 19 species cost the least walking.
/// Answering it by hand off a walkthrough is how you end up hunting a 1.2 % slot on the wrong route;
/// this reads `WildDataPointers` instead and reports, per unowned species, the map where its summed
/// encounter share is fattest.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_dex_sweep_candidates() {
    use crate::pokemon::wild::{self, Terrain};
    use std::collections::{HashMap, HashSet};
    use strum::IntoEnumIterator;

    let mut fixture = TestFixture::new(HIDDEN_ITEM, Duration::from_mins(1), vec![]);
    let state = fixture.game_state();
    let owned: HashSet<PokemonSpecies> = state.pokedex_owned.species().into_iter().collect();
    println!("owned {} — need {} more for Exp.All", owned.len(), 50usize.saturating_sub(owned.len()));

    // species → its best (share, map, terrain, level)
    let mut best: HashMap<PokemonSpecies, (f64, Map, &str, u8)> = HashMap::new();
    // map → how many *unowned* species it offers, and their summed share
    let mut per_map: HashMap<Map, (usize, f64)> = HashMap::new();
    for map in Map::iter() {
        let Some(wild) = wild::encounters(map) else { continue };
        for (terrain, label) in [(Terrain::Grass, "grass"), (Terrain::Water, "water")] {
            for (species, share, level) in wild.species(terrain) {
                if owned.contains(&species) { continue }
                let entry = best.entry(species).or_insert((0.0, map, label, level));
                if share > entry.0 { *entry = (share, map, label, level); }
                let m = per_map.entry(map).or_insert((0, 0.0));
                m.0 += 1;
                m.1 += share;
            }
        }
    }

    println!("\n== {} catchable species not yet owned, best map each", best.len());
    let mut rows: Vec<_> = best.into_iter().collect();
    rows.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    for (species, (share, map, terrain, level)) in &rows {
        println!("   {:<14} {:>5.1}% {:<22} {} lv{}", format!("{species:?}"), share * 100.0, format!("{map}"), terrain, level);
    }

    println!("\n== maps by how many unowned species they offer");
    let mut maps: Vec<_> = per_map.into_iter().collect();
    maps.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.total_cmp(&a.1.1)));
    for (map, (count, share)) in maps.iter().take(30) {
        let wild = wild::encounters(*map).unwrap();
        let missing: Vec<String> = [Terrain::Grass, Terrain::Water].iter()
            .flat_map(|t| wild.species(*t))
            .filter(|(s, _, _)| !owned.contains(s))
            .map(|(s, p, l)| format!("{s:?} {:.0}%/lv{l}", p * 100.0))
            .collect();
        println!("   {:<24} {count} species, {:>5.1}% — {}",
            format!("{map}"), share * 100.0, missing.join(", "));
    }
}

/// Diagnostic for **H3/H4** — the east end of Route 11: the gate the Itemfinder aide is upstairs in,
/// and the hidden Escape Rope at (48,5) that H4 collects.
///
/// Both facts this prints are ones the ROM gives coordinates for but not *reachability*:
/// `Route11_Object` puts the gate's west doors at (49,8)/(49,9) and `HiddenItemCoords` puts the item
/// at x=48, y=5, and the question is whether the walk from Vermilion reaches either — Route 11 has
/// ten trainers on it, and a trainer standing in the corridor is an obstacle in the block map.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_route11_hidden_item() {
    let mut fixture = TestFixture::new(SAFARI, Duration::from_mins(60), vec![
        PolicyStep::Fly { to: Map::VermilionCity },
        PolicyStep::goto(Map::Route11),
    ]);
    fixture.run_until(|s| s.map.map == Map::Route11);
    for _ in 0..50 { fixture.step(); }
    let state = fixture.game_state();
    println!("== Route11 @ {} ({}x{})", state.map.player_position, state.map.width, state.map.height);
    for action in state.map.actions() {
        println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
    // The east end only — Route 11 is 60 tiles wide and the interesting column is x=40..60.
    print!("      ");
    for x in 40..state.map.width.min(60) as u8 { print!("{}", x / 10); }
    println!();
    print!("      ");
    for x in 40..state.map.width.min(60) as u8 { print!("{}", x % 10); }
    println!();
    for y in 0..state.map.height.min(20) as u8 {
        let row: String = (40..state.map.width.min(60) as u8).map(|x| match state.map.tile_at_checked(Point8 { x, y }) {
            Some(MetaTile::Obstacle) => '#',
            Some(MetaTile::Empty) => '.',
            Some(MetaTile::Warp { .. }) => 'W',
            Some(MetaTile::Connection { .. }) => 'C',
            Some(MetaTile::Jump(_)) => 'J',
            Some(MetaTile::Sprite(_)) => 'S',
            Some(MetaTile::Grass) => 'g',
            Some(MetaTile::CutTree) => 'T',
            _ => '?',
        }).collect();
        println!("  y{y:>2} {row}");
    }
}
