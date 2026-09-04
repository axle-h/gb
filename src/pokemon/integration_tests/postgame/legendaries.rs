//! Tests for workstream `legendaries` — see `docs/postgame-coverage-plan.md` §6 and
//! [`crate::pokemon::postgame::legendaries`].

#[allow(unused_imports)]
use super::super::*;

/// Workstream D roots at **B's output**, not `postgame-phase0.bin`: D is three trips to opposite
/// corners of Kanto (Victory Road, the Power Plant, Cerulean Cave) and `postgame-fly-bike.bin` is the
/// first fixture from which each of those is one `Fly` step away. Fuchsia City (1,16), Fly on Articuno,
/// bag 16/20, ¥41,209 — and ⚠️ Venusaur's Solarbeam at 0 PP, so heal before fighting anything.
const FLY_BIKE: &[u8] = include_bytes!("../../data/postgame-fly-bike.bin");

/// Dump every action the pathfinder can reach from where the agent is standing.
fn dump_reachable(fixture: &mut TestFixture) {
    // Let the map settle: sprites read out of bounds for a few ticks after a warp.
    for _ in 0..50 {
        fixture.step();
    }
    let state = fixture.game_state();
    println!("reachable from {} @ {}:", state.map.map, state.map.player_position);
    for action in state.map.actions() {
        println!("   {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
}


/// ASCII-dump the meta-tile grid, clipped to a window. `#` wall, `.` floor, `~` water, `W` warp,
/// `C`/`c` land/water map connection, `J` ledge, `T` cut tree, `g` grass, `S` sprite, `n` counter.
fn dump_map(fixture: &mut TestFixture, x0: u8, x1: u8, y0: u8, y1: u8) {
    let state = fixture.game_state();
    println!("can_surf={} player={} map {}:", state.map.can_surf, state.map.player_position, state.map.map);
    for y in y0..(y1.min(state.map.height.saturating_sub(1) as u8)) {
        let row: String = (x0..(x1.min(state.map.width.saturating_sub(1) as u8)))
            .map(|x| match state.map.tile_at_checked(Point8 { x, y }) {
                Some(MetaTile::Water) => '~',
                Some(MetaTile::Obstacle) => '#',
                Some(MetaTile::Empty) => '.',
                Some(MetaTile::Warp { .. }) => 'W',
                Some(MetaTile::Connection { .. }) => 'C',
                Some(MetaTile::ConnectionWater(_)) => 'c',
                Some(MetaTile::Jump(_)) => 'J',
                Some(MetaTile::Sprite(_)) => 'S',
                Some(MetaTile::Grass) => 'g',
                Some(MetaTile::CutTree) => 'T',
                Some(MetaTile::Counter) => 'n',
                Some(MetaTile::Pc) => 'P',
                Some(MetaTile::Switch { .. }) => 's',
                // Never in `meta_tiles` — a fishing spot and a boulder push are actions, not tiles.
                Some(MetaTile::Fish { .. } | MetaTile::Boulder { .. } | MetaTile::Cut { .. }) => '.',
                None => ' ',
            })
            .collect();
        println!("   y={y:3} {row}");
    }
}

/// Probe (not a sub-step): how do you get to Moltres?
///
/// Route 23 has two Victory Road doors — `warp_event 4, 31, VICTORY_ROAD_1F` and
/// `warp_event 14, 31, VICTORY_ROAD_2F` (`data/maps/objects/Route23.asm:16-17`) — and neither of the
/// two obvious short cuts works:
///
/// 1. Entering at the 2F door lands at VR2F (29,7), a **sealed east pocket** whose only exits are back
///    to Route 23 and up to VR3F. Moltres at (11,5) is not reachable from it.
/// 2. Coming **down** Route 23 from the Indigo Plateau, the only reachable warp is that same 2F door
///    (47 steps); the 1F door four tiles away at (4,31) is on the *other* terrace and unreachable.
///
/// So Victory Road has to be entered from the bottom, the long way, exactly as `victory_road_1f_steps`
/// does it. This run walks that and asks what VR2F looks like from the 1F ladder.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "probe — run with --ignored --nocapture"]
fn probe_route_to_moltres() {
    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(120), vec![
        PolicyStep::Fly { to: Map::ViridianCity },
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::Route22),
        PolicyStep::enter(Map::Route22Gate),
        PolicyStep::Interact(MapSprite::ROUTE22GATE_GUARD),
        PolicyStep::enter(Map::Route23),
        PolicyStep::goto(Map::VictoryRoad1F),
        PolicyStep::UseStrength { target: PartyRef::Slot(3) },
        PolicyStep::SolveBoulders { switch: Point8 { x: 17, y: 13 } },
        PolicyStep::enter(Map::VictoryRoad2F),
        PolicyStep::UseStrength { target: PartyRef::Slot(3) },
        PolicyStep::SolveBoulders { switch: Point8 { x: 1, y: 16 } },
        // Moltres is not in the (1,16)-opened region either — the north strip that holds it, Super
        // Nerd 2 (4,2) and the Guard Spec (11,0) is entered only through VR2F's own (1,1) ladder,
        // which is the far side of VR3F's (2,0) warp (`VictoryRoad3F.asm:20`).
        PolicyStep::enter(Map::VictoryRoad3F),
        PolicyStep::enter_at(Map::VictoryRoad2F, 1, 1),
    ]);

    fixture.run_until(|s| s.map.map == Map::VictoryRoad3F);
    dump_reachable(&mut fixture);
    fixture.step_until_exhausted();
    dump_reachable(&mut fixture);
}

/// **Task D1a** — the shared toolkit: Thunder Wave on Slowpoke, 10 Ultra + 40 Poké Balls, 10 Hyper
/// Potions.
///
/// Measured: ~3 min emulated, **~8 s wall clock**, most of it Nugget Bridge's five trainers walking
/// into us on the way to the TM.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_arm_for_the_legendaries() {
    use crate::pokemon::postgame::legendaries::PARALYSER_SLOT;

    let mut fixture = TestFixture::new(FLY_BIKE, Duration::from_mins(45),
        PolicyStep::arm_for_legendaries_steps());

    let state = fixture.game_state();
    assert_eq!(state.pokemon[PARALYSER_SLOT as usize].species, PokemonSpecies::Slowpoke,
        "slot {PARALYSER_SLOT} should be the only Thunder Wave-compatible party member");
    assert!(!state.bag.iter().any(|i| i.id == ItemId::PokeBall || i.id == ItemId::UltraBall));

    // The Poké Balls are the last thing bought, so they are what "armed" means.
    let state = fixture.run_leg(|s| s.bag.iter().any(|i| i.id == ItemId::PokeBall && i.quantity >= 40));
    assert!(state.pokemon[PARALYSER_SLOT as usize].moves.iter().flatten()
        .any(|m| m.name == PokemonMoveName::ThunderWave), "Thunder Wave never landed on the paralyser");

    let held = |id: ItemId| state.bag.iter().find(|i| i.id == id).map_or(0, |i| i.quantity);
    println!("armed: {} Ultra Balls, {} Poké Balls, {} Hyper Potions, ¥{} left",
        held(ItemId::UltraBall), held(ItemId::PokeBall), held(ItemId::HyperPotion), state.money);
    assert_eq!(held(ItemId::UltraBall), 10);
    assert_eq!(held(ItemId::PokeBall), 40);
    assert_eq!(held(ItemId::HyperPotion), 10);
    // Slowpoke is the party's HM slave as well as its paralyser — Confusion is what should have gone.
    let moves: Vec<_> = state.pokemon[PARALYSER_SLOT as usize].moves.iter().flatten()
        .map(|m| m.name).collect();
    println!("slot {PARALYSER_SLOT} now knows {moves:?}");
    assert!(moves.contains(&PokemonMoveName::Strength) && moves.contains(&PokemonMoveName::Dig),
        "teaching Thunder Wave must not cost the HM slave its field moves");

    fixture.save_state_named("src/pokemon/data/postgame-thunder-wave.bin").unwrap();
}

/// D1a's output: Cerulean City, Slowpoke holding Thunder Wave / Strength / Headbutt / Dig,
/// 10 Ultra + 40 Poké Balls, 10 Hyper Potions, ¥6,209, party healed.
const ARMED: &[u8] = include_bytes!("../../data/postgame-thunder-wave.bin");

/// **Debug-tier seed — a Master Ball, so the catch itself cannot fail.**
///
/// This is a deliberate cheat and it is confined to this file. `docs/postgame-coverage-plan.md` §3
/// allows RAM writes in the `PokemonApi::debug_*` namespace for exactly this — fixture construction and
/// test seeding — and `postgame::debug::play_path_contains_no_debug_ram_writes` enforces that they stay
/// out of `policy.rs`, `agent.rs` and `postgame/`. Nothing a `Policy` can reach knows about it.
///
/// **What it does and does not prove.** With one of these in the bag the three catches test the parts
/// that are actually unsolved — the *routes* to Victory Road 2F's north strip, the Power Plant and
/// Cerulean Cave B1F, the static-encounter engagement, and that the caught mon lands in the party — and
/// skip the part that is understood and blocked, the fight (see the §11 "trapping moves and the
/// one-shot legendary" entry). `Bag::best_pokeball` ranks by item id and `MASTER_BALL` is `$01`, so
/// simply putting one in the bag is enough; the step lists still say `ball: None`.
///
/// The fixtures these tests write are therefore **debug-seeded**, and anything built on them inherits
/// that. They are proof the content is reachable, not a legitimate playthrough.
fn seed_master_ball(fixture: &mut TestFixture) {
    fixture.api().debug_give_item(ItemId::MasterBall, 1).expect("bag should have room");
}

/// **Task D1b** — catch **Moltres** on Victory Road 2F.
///
/// The trip is the cost: Viridian → Route 22 → Route 23 → VR1F → its boulder switch → VR2F → its
/// boulder switch → VR3F → back down into 2F's north strip, plus however many of Victory Road's nine
/// trainers see us on the way. Measured: ~10 min emulated, **~30 s wall clock**.
///
/// ⚠️ **Master-Ball-seeded** ([`seed_master_ball`]). The *fight* is still unsolved — Moltres opens with
/// Fire Spin about half the time and a lv30 Slowpoke cannot act through the trap, which is D's open
/// blocker (see the §11 "trapping moves and the one-shot legendary" entry). What this test proves is
/// everything either side of that: the route, the encounter, and the catch.
///
/// There is deliberately no "did it faint?" early-out: the only in-game trace of a lost legendary is
/// its map sprite disappearing, and that also reads as absent for a few ticks after any battle.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_moltres() {
    let mut fixture = TestFixture::new(ARMED, Duration::from_mins(90), PolicyStep::moltres_steps());
    seed_master_ball(&mut fixture);

    assert!(!fixture.game_state().pokedex_owned.contains(&PokemonSpecies::Moltres));

    let state = fixture.run_leg(|s| s.pokedex_owned.contains(&PokemonSpecies::Moltres));
    println!("caught Moltres — dex now OWNED {} / SEEN {}",
        state.pokedex_owned.species().len(), state.pokedex_seen.species().len());
    println!("party[{}]: {:?}", state.pokemon.len(),
        state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());

    fixture.save_state_named("src/pokemon/data/postgame-moltres.bin").unwrap();
}

/// Scratch probe: load the artifact a failed run leaves in `target/test-artifacts/` and dump what the
/// pathfinder can see from where it stalled.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "probe — run with --ignored --nocapture"]
fn probe_stall_artifact() {
    let Ok(bytes) = std::fs::read("target/test-artifacts/test_stall_state.bin") else {
        println!("no stall artifact — run a failing test first");
        return;
    };
    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(1), vec![]);
    let state = fixture.game_state();
    println!("{} @ {} mode={:?}", state.map.map, state.map.player_position, state.mode);
    dump_map(&mut fixture, 0, 12, 3, 17);
    let state = fixture.game_state();
    println!("(0,6)={:?} (0,7)={:?} (1,7)={:?} (2,7)={:?} (2,8)={:?}",
        state.map.tile_at_checked(Point8{x:0,y:6}), state.map.tile_at_checked(Point8{x:0,y:7}),
        state.map.tile_at_checked(Point8{x:1,y:7}), state.map.tile_at_checked(Point8{x:2,y:7}),
        state.map.tile_at_checked(Point8{x:2,y:8}));
    println!("tile ids row7: {:?}", (0..8u8).map(|x| state.map.raw_tile_ids[x as usize + 7*state.map.width]).collect::<Vec<_>>());
    println!("tile ids row8: {:?}", (0..8u8).map(|x| state.map.raw_tile_ids[x as usize + 8*state.map.width]).collect::<Vec<_>>());
    println!("pair collisions: {:?}", state.map.tile_pair_collisions);
    for s in &state.map.sprites {
        println!("   sprite {} hidden={} pos={:?}", s.name, s.hidden, s.position);
    }
    dump_reachable(&mut fixture);
}

/// D1b's output: Victory Road 2F's north strip, Moltres in the party at lv50, dex 8 owned / 114 seen.
const MOLTRES: &[u8] = include_bytes!("../../data/postgame-moltres.bin");

/// **Tasks D2 + D3** — reach the Power Plant and catch **Zapdos**.
///
/// ⚠️ **Master-Ball-seeded** ([`seed_master_ball`]).
///
/// Dig out of Victory Road, Fly to Cerulean, then Route 9 → Route 10 → Surf across to the Power Plant
/// door. Measured: ~8 min emulated, **~25 s wall clock**, most of it Route 9's trainers.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_zapdos() {
    let mut fixture = TestFixture::new(MOLTRES, Duration::from_mins(90), PolicyStep::zapdos_steps());
    seed_master_ball(&mut fixture);

    assert!(!fixture.game_state().pokedex_owned.contains(&PokemonSpecies::Zapdos));

    fixture.run_until(|s| s.map.map == Map::PowerPlant);
    println!("reached the Power Plant");

    let state = fixture.run_leg(|s| s.pokedex_owned.contains(&PokemonSpecies::Zapdos));
    println!("caught Zapdos — dex now OWNED {} / SEEN {}",
        state.pokedex_owned.species().len(), state.pokedex_seen.species().len());
    println!("party[{}]: {:?}", state.pokemon.len(),
        state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());

    fixture.save_state_named("src/pokemon/data/postgame-zapdos.bin").unwrap();
}

/// D3's output: the Power Plant, Zapdos in the party at lv50, dex 9 owned / 117 seen, party of 6.
const ZAPDOS: &[u8] = include_bytes!("../../data/postgame-zapdos.bin");

/// **Tasks D5 + D6** — Cerulean Cave and **Mewtwo**, the last of the four.
///
/// ⚠️ **Master-Ball-seeded** ([`seed_master_ball`]) — and here the seed is doing more work than it does
/// for the birds: Mewtwo is lv70 with base speed 130, so it outruns even the fastest thing D could
/// obtain, and the honest fight is the open question of D6.
///
/// The party arrives full, so the leg banks **Moltres** at the Cerulean PC on the way past — a caught
/// Pokémon with no free slot goes to the box, and the nickname screen on that path wedges the agent.
/// Commit target: `postgame-legendaries.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_catch_mewtwo() {
    // A generous budget: this is much the longest leg in D — the Route 24 river, then Cerulean Cave's
    // three-floor ladder maze, whose Surf legs and 128-step walks are all wild-encounter territory.
    let mut fixture = TestFixture::new(ZAPDOS, Duration::from_mins(150), PolicyStep::mewtwo_steps());
    seed_master_ball(&mut fixture);

    let state = fixture.game_state();
    assert!(!state.pokedex_owned.contains(&PokemonSpecies::Mewtwo));
    assert_eq!(state.pokemon.len(), 6, "the leg starts with a full party and banks Moltres to make room");

    fixture.run_until(|s| s.map.map == Map::CeruleanCaveB1F);
    println!("reached Mewtwo's chamber");

    let state = fixture.run_leg(|s| s.pokedex_owned.contains(&PokemonSpecies::Mewtwo));
    println!("caught Mewtwo — dex now OWNED {} / SEEN {}",
        state.pokedex_owned.species().len(), state.pokedex_seen.species().len());
    println!("party[{}]: {:?}", state.pokemon.len(),
        state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    println!("box{}[{}]: {:?}", state.current_box + 1, state.boxed_pokemon.len(),
        state.boxed_pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());

    fixture.save_state_named("src/pokemon/data/postgame-legendaries.bin").unwrap();
}

/// Probe: the **Cerulean Cave approach**, which is three separate walls in a row.
///
/// 1. Cerulean City is cut in two. The Fly landing, the gym and the marts are all east of a lake and a
///    solid wall at x=8; the cave door at ROM (4,11) is west of it, ringed by ledges. No land route
///    joins them, and Cerulean's own west edge only meets Route 4's raised Mt Moon path, which the
///    pathfinder cannot climb back onto. The way through is **Route 24's left river seam**, surfed
///    south — a `ConnectionWater` at Route 24 (4..8, 36).
/// 2. That seam is unaskable through `enter()`: `actions()` emits one crossing per adjacent map and
///    Route 24's footbridge is two steps away. Hence `enter_at` + `water_connection_action`.
/// 3. Inside, 1F's ladder down to B1F at (0,6) is behind an **elevation boundary** — the strip in front
///    of it is raw tile 32, the room below is tile 5, and `(32, 5)` is in the Cavern tileset's
///    `TilePairCollisions`. It is reached by going up at 1F (3,11) and back down at 2F (1,3). Not any
///    other pair: the 2F piece that 1F (23,7) leads to reaches nothing but 1F (27,1) and itself.
///
/// Prints the reachable set at each stage.
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "probe — run with --ignored --nocapture"]
fn probe_route_to_cerulean_cave() {
    let mut fixture = TestFixture::new(ZAPDOS, Duration::from_mins(90), PolicyStep::mewtwo_steps());
    for map in [Map::Route24, Map::CeruleanCave1F, Map::CeruleanCave2F, Map::CeruleanCaveB1F] {
        fixture.run_until(|s| s.map.map == map);
        dump_reachable(&mut fixture);
    }
}
