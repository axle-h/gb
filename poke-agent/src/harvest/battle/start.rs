//! `pokered/fixtures/battle/`: `LoadEnemyMonData` and `TryRunningFromBattle`.

use poke_core::species::PokemonSpecies;
use pokered::systems::battle::Arena;
use serde_json::{json, Value};
use crate::pokemon::symbols::pokered_symbols as sym;
use super::{changed, read_arena, run, text_printed, write_arena, Oracle};

const BATTLE_TYPE_NORMAL: u8 = 0;
const BATTLE_TYPE_SAFARI: u8 = 2;

/// `LoadEnemyMonData` for `species` at `level` from party slot `which`, with a Pokédex that has
/// seen nothing: what changed, and the seen flags after.
fn load_enemy_mon_data(oracle: &mut Oracle, arena: &Arena, species: PokemonSpecies, level: u8, which: u8) -> (Value, Vec<u8>) {
    write_arena(oracle, arena);
    oracle.write(sym::wEnemyMonSpecies2, &[species as u8]);
    // `WriteMonMoves` reads the learnset of `wCurPartySpecies`, which every caller sets to the same.
    oracle.write(sym::wCurPartySpecies, &[species as u8]);
    oracle.write(sym::wCurEnemyLevel, &[level]);
    oracle.write(sym::wWhichPokemon, &[which]);
    oracle.write(sym::wPokedexSeen, &[0; 19]);
    let ran = run(oracle, sym::LoadEnemyMonData, &[], &[], &mut |_, _| {});
    let seen = oracle.read(sym::wPokedexSeen, 19);
    let after = read_arena(oracle);
    let change = changed(&serde_json::to_value(arena).unwrap(), &serde_json::to_value(&after).unwrap()).unwrap_or(Value::Null);
    (json!({"changed": change, "returned": seen}), ran.rng)
}

/// `TryRunningFromBattle` from the battle menu, or from the prompt after a faint with the first
/// party mon's speed, in a normal battle or the Safari Zone.
fn try_running_from_battle(oracle: &mut Oracle, arena: &Arena, from_menu: bool, safari: bool, attempts: u8) -> (Value, Vec<u8>) {
    write_arena(oracle, arena);
    oracle.write(sym::wBattleType, &[if safari { BATTLE_TYPE_SAFARI } else { BATTLE_TYPE_NORMAL }]);
    oracle.write(sym::wNumRunAttempts, &[attempts]);
    oracle.write(sym::wActionResultOrTookBattleTurn, &[0]);
    // Not a ghost battle: a map outside Pokémon Tower.
    oracle.write(sym::wCurMap, &[0]);
    let speed = if from_menu { sym::wBattleMonSpeed } else { sym::wPartyMon1Speed };
    let registers = oracle.registers_mut();
    registers.set_hl(speed.address);
    registers.set_de(sym::wEnemyMonSpeed.address);
    let mut texts = vec![];
    let ran = run(oracle, sym::TryRunningFromBattle, &[], &[("PrintText", sym::PrintText),
        ("PlaySoundWaitForCurrent", sym::PlaySoundWaitForCurrent), ("WaitForSoundToFinish", sym::WaitForSoundToFinish),
        ("SaveScreenTilesToBuffer1", sym::SaveScreenTilesToBuffer1), ("LoadScreenTilesFromBuffer1", sym::LoadScreenTilesFromBuffer1)],
        &mut |name, gb| text_printed(&mut texts, name, gb));
    let escaped = oracle.registers().flags.c;
    let took_turn = oracle.read(sym::wActionResultOrTookBattleTurn, 1)[0] != 0;
    let attempts_after = oracle.read(sym::wNumRunAttempts, 1)[0];
    let after = read_arena(oracle);
    let change = changed(&serde_json::to_value(arena).unwrap(), &serde_json::to_value(&after).unwrap()).unwrap_or(Value::Null);
    (json!({"changed": change, "returned": {"escaped": escaped, "took_turn": took_turn, "texts": texts,
        "attempts": attempts_after}}), ran.rng)
}

/// The oracle's own check: a faster mon always gets away from a wild battle.
#[test]
fn a_faster_mon_gets_away() {
    let mut oracle = super::oracle();
    let mut arena = Arena::baseline();
    arena.battle.player.mon.stats[3] = 200;
    arena.battle.enemy.mon.stats[3] = 100;
    let (output, _) = try_running_from_battle(&mut oracle, &arena, true, false, 0);
    assert_eq!(output["returned"], json!({"escaped": true, "took_turn": false, "texts": ["GotAwayText"], "attempts": 1}));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use pokered::rng::GameRng;
    use pokered::systems::add_mon::{new_party_mon, Origin};
    use pokered::systems::battle::{BattleKind, Status3};
    use rand::RngExt;
    use super::super::generate::*;
    use super::super::super::{write_fixture, Case};
    use super::super::sparse;
    use super::*;

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/load_enemy_mon_data.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_load_enemy_mon_data() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x10AD);
        let cases: Vec<Case<Value, Value>> = (0..200).map(|_| {
            let mut arena = Arena::baseline();
            let battle = &mut arena.battle;
            battle.kind = pick(&mut rng, &[BattleKind::Wild, BattleKind::Trainer]);
            let len = rng.random_range(1..=3);
            battle.enemy_party = (0..len).map(|_| {
                let mut mon = new_party_mon(a_species(&mut rng), rng.random_range(2..=70), 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
                mon.mon.hp = rng.random_range(0..=mon.stats[0]);
                mon.mon.status = a_status(&mut rng);
                mon
            }).collect();
            if rng.random_bool(0.15) {
                battle.enemy.status3 |= Status3::TRANSFORMED;
                battle.transformed_enemy_original_dvs = pokered::systems::stats::Dvs([rng.random(), rng.random()]);
            }
            battle.enemy.mon.status = a_status(&mut rng);
            battle.enemy.stat_mods = [(); 6].map(|_| a_stat_mod(&mut rng));
            let (species, level, which) = (a_species(&mut rng), rng.random_range(2..=100), rng.random_range(0..len as u8));
            let input = json!({"arena": sparse(&arena), "species": species, "level": level, "which": which});
            let (output, rng_bytes) = load_enemy_mon_data(&mut oracle, &arena, species, level, which);
            Case { input, output, rng: rng_bytes }
        }).collect();
        write_fixture("battle", "load_enemy_mon_data", &cases);
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/try_running_from_battle.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_try_running_from_battle() {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(0x2A11);
        let cases: Vec<Case<Value, Value>> = (0..300).map(|_| {
            let mut arena = Arena::baseline();
            arena.battle.kind = pick(&mut rng, &[BattleKind::Wild, BattleKind::Wild, BattleKind::Wild, BattleKind::Trainer]);
            let slow = rng.random_range(1..=300);
            arena.battle.player.mon.stats[3] = if rng.random_bool(0.2) { a_stat(&mut rng) } else { slow };
            arena.party[0].stats[3] = a_stat(&mut rng);
            let any = rng.random_range(1..=999);
            arena.battle.enemy.mon.stats[3] = pick(&mut rng, &[slow.saturating_add(1), 3, 4, 1023, 1024, any]);
            let from_menu = rng.random_bool(0.8);
            let safari = rng.random_bool(0.05);
            let any: u8 = rng.random();
            let attempts = pick(&mut rng, &[0, 1, 2, 5, 8, 255, any]);
            let input = json!({"arena": sparse(&arena), "from_menu": from_menu, "safari": safari, "attempts": attempts});
            let (output, rng_bytes) = try_running_from_battle(&mut oracle, &arena, from_menu, safari, attempts);
            Case { input, output, rng: rng_bytes }
        }).collect();
        write_fixture("battle", "try_running_from_battle", &cases);
    }
}
