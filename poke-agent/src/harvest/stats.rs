use pokered::systems::stats::{Dvs, Stat, StatInput};
use crate::pokemon::symbols::pokered_symbols;
use super::Oracle;

/// `CalcStat` with `hl` at `wPartyMon1HPExp - 1`, the base in `wMonHeader` and the level in
/// `wCurEnemyLevel`; the result is `hMultiplicand + 1`, big-endian.
fn calc_stat(oracle: &mut Oracle, input: StatInput) -> u16 {
    let c = input.stat as u8;
    oracle.write(pokered_symbols::wMonHeader + c as u16, &[input.base]);
    oracle.write(pokered_symbols::wPartyMon1HPExp + 2 * (c as u16 - 1), &input.stat_exp.unwrap_or(0).to_be_bytes());
    oracle.write(pokered_symbols::wPartyMon1DVs, &input.dvs.0);
    oracle.write(pokered_symbols::wCurEnemyLevel, &[input.level]);
    let hl = pokered_symbols::wPartyMon1HPExp.address - 1;
    let registers = oracle.registers_mut();
    registers.b = input.stat_exp.is_some() as u8;
    registers.c = c;
    registers.set_hl(hl);
    let called = oracle.call(pokered_symbols::CalcStat);
    assert!(called.rng.is_empty());
    let result = oracle.read(pokered_symbols::hMultiplicand + 1, 2);
    u16::from_be_bytes([result[0], result[1]])
}

#[cfg(feature = "slow-tests")]
fn inputs() -> Vec<StatInput> {
    use rand::{RngExt, SeedableRng};
    const STATS: [Stat; 5] = [Stat::Hp, Stat::Attack, Stat::Defense, Stat::Speed, Stat::Special];
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xCA1C);
    let mut inputs: Vec<StatInput> = [0, 1, 2, 4, 65024, 65025, 65026, 65535].into_iter()
        .flat_map(|exp| STATS.map(|stat| StatInput { stat, base: 255, dvs: Dvs([0xFF, 0xFF]), stat_exp: Some(exp), level: 100 }))
        .collect();
    for level in [0, 1, 255] {
        inputs.extend(STATS.map(|stat| StatInput { stat, base: 255, dvs: Dvs([0xFF, 0xFF]), stat_exp: Some(65535), level }));
    }
    while inputs.len() < 400 {
        let exp = match rng.random_range(0..4) {
            0 => None,
            1 => Some(rng.random_range(0..=400)),
            _ => Some(rng.random()),
        };
        inputs.push(StatInput {
            stat: STATS[rng.random_range(0..5)],
            base: rng.random_range(1..=255),
            dvs: Dvs([rng.random(), rng.random()]),
            stat_exp: exp,
            level: rng.random_range(1..=100),
        });
    }
    inputs
}

/// The oracle's own check: a few values worked out by hand.
#[test]
fn calc_stat_answers_as_the_formula_does() {
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let max = StatInput { stat: Stat::Attack, base: 255, dvs: Dvs([0xFF, 0xFF]), stat_exp: Some(65535), level: 100 };
    assert_eq!(calc_stat(&mut oracle, max), 608, "(255 + 15) * 2 + 63, and 5");
    let none = StatInput { stat: Stat::Hp, base: 45, dvs: Dvs([0, 0]), stat_exp: None, level: 5 };
    assert_eq!(calc_stat(&mut oracle, none), 45 * 2 * 5 / 100 + 5 + 10);
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/stats/calc_stat.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_calc_stat() {
    use super::{write_fixture, Case};
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let cases: Vec<Case<StatInput, u16>> = inputs().into_iter()
        .map(|input| Case { input, output: calc_stat(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("stats", "calc_stat", &cases);
}

