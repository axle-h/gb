//! `pokered/fixtures/battle/`: the Safari Zone's bait, rock, eating and anger, and running away.

use pokered::systems::battle::Arena;
use serde_json::{json, Value};
use crate::pokemon::symbols::pokered_symbols as sym;
use super::{local_label, run_arena, run_arena_observed, text_printed, Oracle};

const BATTLE_TYPE_SAFARI: u8 = 2;

fn safari(oracle: &mut Oracle) {
    oracle.write(sym::wBattleType, &[BATTLE_TYPE_SAFARI]);
}

/// `ItemUseBait` or `ItemUseRock`, with the text, the animation and the wait returned from.
fn bait_or_rock(oracle: &mut Oracle, arena: &Arena, rock: bool) -> (Value, Vec<u8>) {
    safari(oracle);
    let entry = if rock { sym::ItemUseRock } else { sym::ItemUseBait };
    let skips = [("PrintText", sym::PrintText), ("MoveAnimation", sym::MoveAnimation), ("DelayFrames", sym::DelayFrames)];
    let (_, change, ran) = run_arena(oracle, arena, entry, &[], &skips);
    (json!({"changed": change}), ran.rng)
}

/// `PrintSafariZoneBattleText`: what it changed, and the text it printed if any.
fn safari_zone_battle_text(oracle: &mut Oracle, arena: &Arena) -> Value {
    let mut texts = vec![];
    let skips = [("PrintText", sym::PrintText), ("LoadScreenTilesFromBuffer1", sym::LoadScreenTilesFromBuffer1)];
    let (_, change, _) = run_arena_observed(oracle, arena, sym::PrintSafariZoneBattleText, &[], &skips,
        &mut |name, gb| text_printed(&mut texts, name, gb));
    json!({"changed": change, "returned": texts.first()})
}

/// `StartBattle` after a safari turn's text: whether it reaches `EnemyRan`.
fn safari_mon_runs(oracle: &mut Oracle, arena: &Arena) -> (Value, Vec<u8>) {
    safari(oracle);
    let stops = [("EnemyRan", sym::EnemyRan), ("checkAnyPartyAlive", local_label("StartBattle.checkAnyPartyAlive"))];
    let skips = [("PrintSafariZoneBattleText", sym::PrintSafariZoneBattleText)];
    let (_, change, ran) = run_arena(oracle, arena, local_label("StartBattle.notOutOfSafariBalls"), &stops, &skips);
    (json!({"changed": change, "returned": ran.exit == Some("EnemyRan")}), ran.rng)
}

/// The oracle's own check: a mon with a speed of 128 or more always runs.
#[test]
fn a_fast_safari_mon_always_runs() {
    let mut oracle = super::oracle();
    let mut arena = Arena::baseline();
    arena.battle.enemy.mon.stats[3] = 0x80;
    let (output, rng) = safari_mon_runs(&mut oracle, &arena);
    assert_eq!((output["returned"].clone(), rng.len()), (json!(true), 0));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use rand::RngExt;
    use super::super::generate::*;
    use super::super::super::{write_fixture, Case};
    use super::super::sparse;
    use super::*;

    /// An arena with the factors, the catch rate and the enemy's speed spread over what matters.
    fn an_arena(rng: &mut rand::rngs::StdRng) -> Arena {
        let mut arena = Arena::baseline();
        let battle = &mut arena.battle;
        let any: u8 = rng.random();
        battle.safari_bait_factor = pick(rng, &[0, 0, 1, 2, 5, 250, any]);
        let any: u8 = rng.random();
        battle.safari_escape_factor = pick(rng, &[0, 0, 1, 2, 5, 252, any]);
        let any: u8 = rng.random();
        battle.enemy_exp.catch_rate = pick(rng, &[0, 1, 30, 127, 128, 255, any]);
        battle.enemy.mon.species = a_species(rng);
        let any: u16 = rng.random_range(1..=999);
        battle.enemy.mon.stats[3] = pick(rng, &[1, 40, 63, 64, 127, 128, 0x17F, 0x100, any]);
        arena
    }

    fn cases(seed: u64, mut one: impl FnMut(&mut Oracle, &Arena) -> (Value, Vec<u8>)) -> Vec<Case<Value, Value>> {
        let mut oracle = super::super::oracle();
        let mut rng = seeded(seed);
        (0..150).map(|_| {
            let arena = an_arena(&mut rng);
            let (output, rng_bytes) = one(&mut oracle, &arena);
            Case { input: json!({"arena": sparse(&arena)}), output, rng: rng_bytes }
        }).collect()
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/battle/{throw_bait,throw_rock,safari_zone_battle_text,safari_mon_runs}.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_safari() {
        write_fixture("battle", "throw_bait", &cases(0x5AF1, |oracle, arena| bait_or_rock(oracle, arena, false)));
        write_fixture("battle", "throw_rock", &cases(0x5AF2, |oracle, arena| bait_or_rock(oracle, arena, true)));
        write_fixture("battle", "safari_zone_battle_text", &cases(0x5AF3, |oracle, arena| (safari_zone_battle_text(oracle, arena), vec![])));
        write_fixture("battle", "safari_mon_runs", &cases(0x5AF4, safari_mon_runs));
    }
}
