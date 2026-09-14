use crate::pokemon::symbols::pokered_symbols;
use super::Oracle;

/// `CountSetBits` over an array written into `wBuffer`: `hl` is the array and `b` its length. The
/// count comes back in `wNumSetBits`, which is `$d11e` — the byte `wPokedexNum` also answers to.
fn count_set_bits(oracle: &mut Oracle, flags: &[u8]) -> u8 {
    oracle.write(pokered_symbols::wBuffer, flags);
    let registers = oracle.registers_mut();
    registers.set_hl(pokered_symbols::wBuffer.address);
    registers.b = flags.len() as u8;
    let called = oracle.call(pokered_symbols::CountSetBits);
    assert!(called.rng.is_empty());
    oracle.read(pokered_symbols::wNumSetBits, 1)[0]
}

/// `PokedexToIndex`, whose argument and answer are both `wPokedexNum`.
fn pokedex_to_index(oracle: &mut Oracle, dex: u8) -> u8 {
    oracle.write(pokered_symbols::wPokedexNum, &[dex]);
    let called = oracle.call(pokered_symbols::PokedexToIndex);
    assert!(called.rng.is_empty());
    oracle.read(pokered_symbols::wPokedexNum, 1)[0]
}

#[cfg(feature = "slow-tests")]
fn flag_inputs() -> Vec<Vec<u8>> {
    use rand::{RngExt, SeedableRng};
    use pokered::systems::pokedex::FLAG_BYTES;
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xDE15);
    // The two arrays' own length, the empty and full cases, and one long enough to wrap the
    // one-byte counter: 32 bytes of ones is 256 flags, which counts as none.
    let mut inputs = vec![vec![0u8; FLAG_BYTES], vec![0xFF; FLAG_BYTES], vec![0xFF; 32], vec![1u8]];
    while inputs.len() < 400 {
        let length = rng.random_range(1..=FLAG_BYTES);
        inputs.push((0..length).map(|_| rng.random()).collect());
    }
    inputs
}

/// Every number the cartridge can be asked about, and the 0 that finds the first MISSINGNO.
#[cfg(feature = "slow-tests")]
fn dex_inputs() -> Vec<u8> {
    (0..=pokered::systems::pokedex::NUM_POKEMON).collect()
}

/// The oracle's own check, worked out by hand.
#[test]
fn the_count_is_the_bits_that_are_set() {
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    assert_eq!(count_set_bits(&mut oracle, &[0b1010_1010, 0x00, 0xFF]), 12);
    assert_eq!(count_set_bits(&mut oracle, &[0; 19]), 0);
}

/// Dex 1 is Bulbasaur, whose index is `$99`, and dex 112 is Rhydon, the cartridge's index 1.
#[test]
fn a_dex_number_becomes_the_cartridge_s_own_index() {
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    assert_eq!(pokedex_to_index(&mut oracle, 1), 0x99);
    assert_eq!(pokedex_to_index(&mut oracle, 112), 1);
}

#[test]
#[cfg(feature = "slow-tests")]
fn the_port_matches_the_cartridge() {
    use pokered::systems::pokedex;
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    for flags in flag_inputs() {
        assert_eq!(pokedex::count_set_bits(&flags), count_set_bits(&mut oracle, &flags), "{flags:02X?}");
    }
    for dex in dex_inputs() {
        assert_eq!(pokedex::pokedex_to_index(dex), pokedex_to_index(&mut oracle, dex), "dex {dex}");
    }
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/pokedex/count_set_bits.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_count_set_bits() {
    use super::{write_fixture, Case};
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let cases: Vec<Case<Vec<u8>, u8>> = flag_inputs().into_iter()
        .map(|input| Case { output: count_set_bits(&mut oracle, &input), input, rng: vec![] })
        .collect();
    write_fixture("pokedex", "count_set_bits", &cases);
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/pokedex/pokedex_to_index.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_pokedex_to_index() {
    use super::{write_fixture, Case};
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let cases: Vec<Case<u8, u8>> = dex_inputs().into_iter()
        .map(|input| Case { input, output: pokedex_to_index(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("pokedex", "pokedex_to_index", &cases);
}
