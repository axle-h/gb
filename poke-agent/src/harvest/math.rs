//! `Multiply`, `Divide`, the BCD routines, `FlagAction`, `PrintNumber` and `PrintBCDNumber`.

use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::Oracle;

const NO_TEXT_DELAY: u8 = 1 << 6;
const PRINTED: usize = 12;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

fn multiply(oracle: &mut Oracle, multiplicand: u32, multiplier: u8) -> u32 {
    oracle.write(sym::hMultiplicand, &multiplicand.to_be_bytes()[1..]);
    oracle.write(sym::hMultiplier, &[multiplier]);
    oracle.call(sym::Multiply);
    u32::from_be_bytes(oracle.read(sym::hProduct, 4).try_into().unwrap())
}

fn divide(oracle: &mut Oracle, dividend: [u8; 4], divisor: u8, bytes: u8) -> ([u8; 4], u8) {
    oracle.write(sym::hDividend, &dividend);
    oracle.write(sym::hDivisor, &[divisor]);
    oracle.registers_mut().b = bytes;
    oracle.call(sym::Divide);
    (oracle.read(sym::hQuotient, 4).try_into().unwrap(), oracle.read(sym::hRemainder, 1)[0])
}

/// `AddBCD` or `SubBCD` with the destination at `wBuffer` and the source just after; `de` and
/// `hl` point at the last bytes. Returns the destination and the carry.
fn bcd(oracle: &mut Oracle, routine: DmgPointer, destination: &[u8], source: &[u8]) -> (Vec<u8>, bool) {
    let len = destination.len() as u16;
    let (d, s) = (sym::wBuffer, sym::wBuffer + 8);
    oracle.write(d, destination);
    oracle.write(s, source);
    let registers = oracle.registers_mut();
    registers.set_de(d.address + len - 1);
    registers.set_hl(s.address + len - 1);
    registers.c = len as u8;
    oracle.call(routine);
    let carry = oracle.registers().flags.c;
    (oracle.read(d, len as usize), carry)
}

fn divide_bcd(oracle: &mut Oracle, money: [u8; 3], divisor: [u8; 3]) -> ([u8; 3], [u8; 3]) {
    oracle.write(sym::hMoney, &money);
    oracle.write(sym::hDivideBCDDivisor, &divisor);
    oracle.call(sym::DivideBCD);
    (oracle.read(sym::hDivideBCDQuotient, 3).try_into().unwrap(), oracle.read(sym::hMoney, 3).try_into().unwrap())
}

fn flag_action(oracle: &mut Oracle, flags: &[u8; 32], bit: u8, action: u8) -> (Vec<u8>, u8) {
    oracle.write(sym::wBuffer, flags);
    let registers = oracle.registers_mut();
    registers.set_hl(sym::wBuffer.address);
    registers.b = action;
    registers.c = bit;
    oracle.call(sym::FlagAction);
    (oracle.read(sym::wBuffer, 32), oracle.registers().c)
}

/// A print into blanks at the top-left of `wTileMap`: the tiles, and how far `hl` moved.
fn print(oracle: &mut Oracle, routine: DmgPointer, value: &[u8], b: u8, c: u8) -> (Vec<u8>, u16) {
    let flags = oracle.read(sym::wStatusFlags5, 1)[0];
    oracle.write(sym::wStatusFlags5, &[flags | NO_TEXT_DELAY]);
    oracle.write(sym::wTileMap, &[0x7F; PRINTED]);
    oracle.write(sym::wBuffer, value);
    let registers = oracle.registers_mut();
    registers.set_de(sym::wBuffer.address);
    registers.set_hl(sym::wTileMap.address);
    registers.b = b;
    registers.c = c;
    oracle.call(routine);
    (oracle.read(sym::wTileMap, PRINTED), oracle.registers().hl() - sym::wTileMap.address)
}

#[test]
fn the_oracle_multiplies_divides_and_adds() {
    let mut oracle = oracle();
    assert_eq!(multiply(&mut oracle, 0xFF_FFFF, 0xFF), 0xFF_FFFF * 0xFF);
    assert_eq!(divide(&mut oracle, [0, 0, 0x03, 0xE8], 7, 4), ([0, 0, 0, 142], 6));
    assert_eq!(bcd(&mut oracle, sym::AddBCD, &[0x00, 0x09, 0x95], &[0x00, 0x00, 0x07]), (vec![0x00, 0x10, 0x02], false));
}

#[cfg(feature = "slow-tests")]
mod harvest {
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};
    use super::super::{write_fixture, Case};
    use super::*;

    fn bcd_byte(rng: &mut StdRng) -> u8 {
        rng.random_range(0..10u8) << 4 | rng.random_range(0..10u8)
    }

    #[test]
    #[ignore = "a tool: writes pokered/fixtures/math/*.jsonl under GB_REGEN_FIXTURES=1"]
    fn harvest_math() {
        let mut oracle = oracle();
        let mut rng = StdRng::seed_from_u64(0x3A7);

        let mut cases = vec![];
        for (a, b) in [(0, 0), (0xFF_FFFF, 0xFF), (1, 0xFF), (0xFF_FFFF, 1)].into_iter()
            .chain((0..300).map(|_| (rng.random_range(0..=0xFF_FFFFu32), rng.random()))) {
            cases.push(Case { input: (a, b), output: multiply(&mut oracle, a, b), rng: vec![] });
        }
        write_fixture("math", "multiply", &cases);

        let mut cases = vec![];
        for _ in 0..600 {
            let (dividend, divisor, bytes) = (rng.random::<[u8; 4]>(), rng.random_range(1..=255u8), rng.random_range(1..=4u8));
            cases.push(Case { input: (dividend, divisor, bytes), output: divide(&mut oracle, dividend, divisor, bytes), rng: vec![] });
        }
        write_fixture("math", "divide", &cases);

        for (routine, name) in [(sym::AddBCD, "add_bcd"), (sym::SubBCD, "sub_bcd")] {
            let mut cases = vec![];
            for i in 0..500 {
                let len = rng.random_range(1..=3usize);
                let byte = |rng: &mut StdRng| if i % 5 == 0 { rng.random() } else { bcd_byte(rng) };
                let (d, s): (Vec<u8>, Vec<u8>) = ((0..len).map(|_| byte(&mut rng)).collect(), (0..len).map(|_| byte(&mut rng)).collect());
                cases.push(Case { output: bcd(&mut oracle, routine, &d, &s), input: (d, s), rng: vec![] });
            }
            write_fixture("math", name, &cases);
        }

        let mut cases = vec![];
        for _ in 0..300 {
            let money = [bcd_byte(&mut rng), bcd_byte(&mut rng), bcd_byte(&mut rng)];
            let mut divisor = [0, 0, 0];
            while divisor == [0, 0, 0] {
                let digits = rng.random_range(1..=6);
                divisor = [bcd_byte(&mut rng), bcd_byte(&mut rng), bcd_byte(&mut rng)];
                let value = u32::from_be_bytes([0, divisor[0], divisor[1], divisor[2]]) >> (4 * (6 - digits));
                [_, divisor[0], divisor[1], divisor[2]] = value.to_be_bytes();
            }
            cases.push(Case { input: (money, divisor), output: divide_bcd(&mut oracle, money, divisor), rng: vec![] });
        }
        write_fixture("math", "divide_bcd", &cases);

        let mut cases = vec![];
        for _ in 0..300 {
            let (flags, bit, action) = (rng.random::<[u8; 32]>(), rng.random(), rng.random_range(0..=2u8));
            cases.push(Case { input: (flags.to_vec(), bit, action), output: flag_action(&mut oracle, &flags, bit, action), rng: vec![] });
        }
        write_fixture("math", "flag_action", &cases);

        let mut cases = vec![];
        for _ in 0..600 {
            let bytes = rng.random_range(1..=3u8);
            let value: Vec<u8> = (0..bytes).map(|_| if rng.random_range(0..3) == 0 { 0 } else { rng.random() }).collect();
            let (flags, digits) = (rng.random_range(0..4u8) << 6, rng.random_range(2..=7u8));
            cases.push(Case { output: print(&mut oracle, sym::PrintNumber, &value, flags | bytes, digits), input: (value, flags | bytes, digits), rng: vec![] });
        }
        write_fixture("math", "print_number", &cases);

        let mut cases = vec![];
        for _ in 0..400 {
            let len = rng.random_range(1..=3u8);
            let value: Vec<u8> = (0..len).map(|_| if rng.random_range(0..3) == 0 { 0 } else { bcd_byte(&mut rng) }).collect();
            let flags = rng.random_range(0..8u8) << 5;
            cases.push(Case { output: print(&mut oracle, sym::PrintBCDNumber, &value, 0, flags | len), input: (value, flags | len), rng: vec![] });
        }
        write_fixture("math", "print_bcd", &cases);
    }
}
