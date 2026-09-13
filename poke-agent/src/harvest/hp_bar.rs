use pokered::systems::hp_bar::HpBarInput;
use crate::pokemon::symbols::pokered_symbols;
use super::Oracle;

/// Entered at `GetHPBarLength` rather than `HPBarLength`, because the latter opens with
/// `GetPredefRegisters`, which would overwrite the registers the arguments are in.
fn bar_length(oracle: &mut Oracle, input: HpBarInput) -> u8 {
    let registers = oracle.registers_mut();
    registers.set_bc(input.current);
    registers.set_de(input.max);
    let called = oracle.call(pokered_symbols::GetHPBarLength);
    assert!(called.rng.is_empty());
    oracle.registers().e
}

#[cfg(feature = "slow-tests")]
fn inputs() -> Vec<HpBarInput> {
    use rand::{RngExt, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xB4A);
    // The boundaries: a max either side of the byte the divisor is, and the colours' thresholds.
    let mut inputs: Vec<HpBarInput> = [1u16, 2, 100, 254, 255, 256, 257, 703, 999]
        .into_iter()
        .flat_map(|max| [0u16, 1, max / 4, max / 2, max - 1, max].map(|current| HpBarInput { current, max }))
        .filter(|input| input.max > 0 && input.current <= input.max)
        .collect();
    while inputs.len() < 400 {
        let max = rng.random_range(1..=999);
        inputs.push(HpBarInput { current: rng.random_range(0..=max), max });
    }
    inputs
}

/// The oracle's own check, worked out by hand.
#[test]
fn the_bar_is_as_long_as_the_arithmetic_says() {
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    assert_eq!(bar_length(&mut oracle, HpBarInput { current: 100, max: 100 }), 48, "a full bar");
    assert_eq!(bar_length(&mut oracle, HpBarInput { current: 50, max: 100 }), 24, "half of one");
    assert_eq!(bar_length(&mut oracle, HpBarInput { current: 1, max: 999 }), 1, "never nothing");
}

/// The port against the cartridge, on the cases the fixture is cut from.
#[test]
#[cfg(feature = "slow-tests")]
fn the_port_matches_the_cartridge() {
    use pokered::systems::hp_bar::hp_bar_length;
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    for input in inputs() {
        assert_eq!(hp_bar_length(input.current, input.max), bar_length(&mut oracle, input),
            "{}/{}", input.current, input.max);
    }
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/hp_bar/get_hp_bar_length.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_get_hp_bar_length() {
    use super::{write_fixture, Case};
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let cases: Vec<Case<HpBarInput, u8>> = inputs().into_iter()
        .map(|input| Case { input, output: bar_length(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("hp_bar", "get_hp_bar_length", &cases);
}
