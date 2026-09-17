//! `TryDoWildEncounter` on the cartridge: the tiles under the player, the map, the repel and the two
//! random bytes written in, whether a battle starts and with what read out.

use poke_core::map::Map;
use poke_core::map_gfx::tileset_entry;
#[cfg(feature = "slow-tests")]
use poke_core::map_header::MapHeader;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_local_labels::TryDoWildEncounter;
use pokered::systems::overworld::encounters::{Encounter, EncounterInput, WildMons};
use crate::pokemon::symbols::pokered_symbols as sym;
use super::Oracle;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/pallet-town-state.bin"))
}

/// Writes `input` and the random bytes, and calls `routine` until it returns or reaches `stops`.
fn prepare(oracle: &mut Oracle, input: &EncounterInput, add: u8, sub: u8) {
    let wild = &input.wild;
    let slots = |slots: &[(u8, u8); 10]| slots.iter().flat_map(|&(level, species)| [level, species]).collect::<Vec<u8>>();
    oracle.write(sym::wGrassRate, &[wild.grass_rate]);
    oracle.write(sym::wGrassMons, &slots(&wild.grass));
    oracle.write(sym::wWaterRate, &[wild.water_rate]);
    oracle.write(sym::wWaterMons, &slots(&wild.water));
    oracle.write(sym::wCurMap, &[input.map as u8]);
    oracle.write(sym::wCurMapTileset, &[input.tileset as u8]);
    oracle.write(sym::wGrassTile, &[tileset_entry(input.tileset).grass_tile]);
    oracle.write(sym::wNPCMovementScriptPointerTableNum, &[0]);
    oracle.write(sym::wMovementFlags, &[0]);
    let cell = |x: u16, y: u16| crate::pokemon::symbols::DmgPointer { address: sym::wTileMap.address + y * 20 + x, ..sym::wTileMap };
    oracle.write(cell(8, 9), &[input.bottom_left]);
    oracle.write(cell(9, 9), &[input.bottom_right]);
    oracle.write(sym::wXCoord, &[input.x]);
    oracle.write(sym::wYCoord, &[input.y]);
    oracle.write(sym::wCurMapWidth, &[input.width]);
    oracle.write(sym::wCurMapHeight, &[input.height]);
    oracle.write(sym::wRepelRemainingSteps, &[input.repel_steps]);
    oracle.write(sym::wPartyMon1Level, &[input.lead_level]);
    oracle.write(sym::hRandomAdd, &[add]);
    oracle.write(sym::hRandomSub, &[sub]);
}

/// The cartridge's answer, and the random bytes it read: none unless it reached `.CanEncounter`.
fn try_do_wild_encounter(oracle: &mut Oracle, input: &EncounterInput, add: u8, sub: u8) -> (Encounter, Vec<u8>) {
    prepare(oracle, input, add, sub);
    let (_, stopped) = oracle.call_until(sym::TryDoWildEncounter, &[TryDoWildEncounter::CanEncounter]);
    let rng = if stopped.is_some() { vec![add, sub] } else { vec![] };
    prepare(oracle, input, add, sub);
    oracle.call(sym::TryDoWildEncounter);
    let encountered = oracle.registers().flags.z;
    let species = oracle.read(sym::wEnemyMonSpecies2, 1)[0];
    let level = oracle.read(sym::wCurEnemyLevel, 1)[0];
    let encounter = Encounter {
        mon: encountered.then(|| (PokemonSpecies::from_repr(species).expect("a wild species"), level)),
        repel_steps: oracle.read(sym::wRepelRemainingSteps, 1)[0],
        repel_wore_off: false,
    };
    (encounter, rng)
}

/// Maps with grass, water, both and neither, on every kind of square an encounter looks at.
#[cfg(feature = "slow-tests")]
fn inputs() -> Vec<(EncounterInput, u8, u8)> {
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};
    let mut rng = StdRng::seed_from_u64(0xE7C0);
    let maps: Vec<Map> = Map::all().filter(|map| map.header_pointer().is_some()).collect();
    let wild: Vec<Map> = maps.iter().copied().filter(|&map| poke_core::wild::encounters(map).is_some()).collect();
    // The tables as a walk through the maps leaves them, stale halves and all.
    let mut tables = WildMons::default();
    tables.load(Map::Route1);
    tables.load(Map::Route21);
    (0..2000).map(|_| {
        let from = if rng.random_bool(0.8) { &wild } else { &maps };
        let map = from[rng.random_range(0..from.len())];
        let header = MapHeader::read(map).unwrap();
        let grass = tileset_entry(header.tileset).grass_tile;
        let mut tile = || [grass, grass, 0x14, 0x14, rng.random_range(0..0x60)][rng.random_range(0..5)];
        let (bottom_left, bottom_right) = (tile(), tile());
        let edge = |rng: &mut StdRng, size: u8| match rng.random_range(0..10) {
            0 => size * 2,
            1 => 0xFF,
            _ => rng.random_range(0..size * 2),
        };
        let input = EncounterInput {
            map,
            tileset: header.tileset,
            bottom_left,
            bottom_right,
            x: edge(&mut rng, header.width),
            y: edge(&mut rng, header.height),
            width: header.width,
            height: header.height,
            // Not 1: the last step of a repel prints its text, which waits for frames.
            repel_steps: [0, 0, 0, 0, 0, 2, 50][rng.random_range(0..7)],
            lead_level: rng.random_range(1..=60),
            wild: { tables.load(map); tables },
        };
        // Half the time low enough to beat a map's rate.
        let add = if rng.random_bool(0.7) { rng.random_range(0..16) } else { rng.random() };
        (input, add, rng.random())
    }).collect()
}

#[test]
fn a_repel_holds_off_a_mon_below_the_lead_s_level() {
    let mut oracle = oracle();
    let grass = tileset_entry(poke_core::map_header::TileSetId::Overworld).grass_tile;
    let input = EncounterInput {
        map: Map::Route1, tileset: poke_core::map_header::TileSetId::Overworld, bottom_left: grass, bottom_right: grass,
        x: 10, y: 10, width: 10, height: 18, repel_steps: 5, lead_level: 50, wild: { let mut wild = WildMons::default(); wild.load(Map::Route1); wild },
    };
    let (encounter, rng) = try_do_wild_encounter(&mut oracle, &input, 0, 0);
    assert_eq!((encounter.mon, encounter.repel_steps, rng.len()), (None, 4, 2));
    let (encounter, _) = try_do_wild_encounter(&mut oracle, &EncounterInput { repel_steps: 0, ..input }, 0, 0);
    assert!(encounter.mon.is_some());
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/overworld/try_do_wild_encounter.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_try_do_wild_encounter() {
    use super::{write_fixture, Case};
    let mut oracle = oracle();
    let cases: Vec<_> = inputs().into_iter().map(|(input, add, sub)| {
        let (output, rng) = try_do_wild_encounter(&mut oracle, &input, add, sub);
        Case { input, output, rng }
    }).collect();
    let encountered = cases.iter().filter(|case| case.output.mon.is_some()).count();
    assert!(encountered > 200, "only {encountered} encounters among {}", cases.len());
    write_fixture("overworld", "try_do_wild_encounter", &cases);
}
