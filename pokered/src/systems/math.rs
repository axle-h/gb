//! `home/math.asm`, `engine/math/{multiply_divide,bcd}.asm` and `engine/flag_action.asm`. BCD
//! numbers are big-endian bytes, as the cartridge keeps them.

use serde::{Deserialize, Serialize};

/// `Multiply`: a 3-byte multiplicand by a 1-byte multiplier.
pub fn multiply(multiplicand: u32, multiplier: u8) -> u32 {
    (multiplicand & 0xFF_FFFF) * multiplier as u32
}

/// `_Divide`, ported step for step: `bytes` is `b`, how many of the dividend's four bytes it
/// divides. Returns `hQuotient` and `hRemainder`; a divisor of 0 never returns on the cartridge.
pub fn divide(mut dividend: [u8; 4], mut divisor: u8, mut bytes: u8) -> ([u8; 4], u8) {
    assert!(divisor != 0, "_Divide by zero never returns");
    let mut buffer = [0u8; 5];
    let mut e = 9u8;
    loop {
        let high = u16::from_be_bytes([dividend[0], dividend[1]]);
        let subtrahend = u16::from_be_bytes([divisor, buffer[0]]);
        if high >= subtrahend {
            [dividend[0], dividend[1]] = (high - subtrahend).to_be_bytes();
            buffer[4] = buffer[4].wrapping_add(1);
            continue;
        }
        if bytes == 1 {
            return ([buffer[1], buffer[2], buffer[3], buffer[4]], dividend[1]);
        }
        let shifted = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) << 1;
        [buffer[1], buffer[2], buffer[3], buffer[4]] = shifted.to_be_bytes();
        e -= 1;
        if e == 0 {
            e = 8;
            divisor = buffer[0];
            buffer[0] = 0;
            dividend[0] = dividend[1];
            dividend[1] = dividend[2];
            dividend[2] = dividend[3];
        }
        if e == 1 {
            bytes -= 1;
        }
        let carry = divisor & 1;
        divisor >>= 1;
        buffer[0] = buffer[0] >> 1 | carry << 7;
    }
}

/// The CPU's `daa` after an add (`subtract` false) or a subtract, returning the carry it leaves.
pub fn daa(a: u8, subtract: bool, half_carry: bool, carry: bool) -> (u8, bool) {
    let mut adjust = 0u8;
    let mut carry_out = carry;
    if subtract {
        if half_carry { adjust |= 0x06; }
        if carry { adjust |= 0x60; }
        (a.wrapping_sub(adjust), carry_out)
    } else {
        if half_carry || a & 0x0F > 0x09 { adjust |= 0x06; }
        if carry || a > 0x99 {
            adjust |= 0x60;
            carry_out = true;
        }
        (a.wrapping_add(adjust), carry_out)
    }
}

/// `AddBCD`: `destination += source`, both `len` bytes; an overflow leaves every byte `$99`.
pub fn add_bcd(destination: &mut [u8], source: &[u8]) {
    let mut carry = false;
    for (d, &s) in destination.iter_mut().zip(source).rev() {
        let sum = *d as u16 + s as u16 + carry as u16;
        let half = (*d & 0x0F) + (s & 0x0F) + carry as u8 > 0x0F;
        (*d, carry) = daa(sum as u8, false, half, sum > 0xFF);
    }
    if carry {
        destination.fill(0x99);
    }
}

/// `SubBCD`: `destination -= source`; going below zero leaves every byte `$00` and returns true.
pub fn sub_bcd(destination: &mut [u8], source: &[u8]) -> bool {
    let mut borrow = false;
    for (d, &s) in destination.iter_mut().zip(source).rev() {
        let difference = *d as i16 - s as i16 - borrow as i16;
        let half = ((*d & 0x0F) as i16) - ((s & 0x0F) as i16) - (borrow as i16) < 0;
        (*d, borrow) = daa(difference as u8, true, half, difference < 0);
    }
    if borrow {
        destination.fill(0);
    }
    borrow
}

/// `DivideBCD`, ported step for step: `hMoney` by `hDivideBCDDivisor`, three bytes each. Returns
/// the quotient; `money` is left holding what the long division left behind.
pub fn divide_bcd(money: &mut [u8; 3], mut divisor: [u8; 3]) -> [u8; 3] {
    let mut buffer = [0u8; 3];
    let mut d = 1u8;
    while divisor[0] & 0xF0 == 0 {
        d += 1;
        let digits = u32::from_be_bytes([0, divisor[0], divisor[1], divisor[2]]) << 4;
        [_, divisor[0], divisor[1], divisor[2]] = (digits & 0xFF_FFF0).to_be_bytes();
    }
    let digits_wanted = d;
    let next_digit = |money: &mut [u8; 3], divisor: &[u8; 3]| {
        let mut count = 0u8;
        while *money >= *divisor {
            count += 1;
            sub_bcd(money, divisor);
        }
        count
    };
    let div_by_10 = |divisor: &mut [u8; 3]| {
        let digits = u32::from_be_bytes([0, divisor[0], divisor[1], divisor[2]]) >> 4;
        [_, divisor[0], divisor[1], divisor[2]] = digits.to_be_bytes();
    };
    for place in 0..6u8 {
        if place > 0 {
            div_by_10(&mut divisor);
        }
        let digit = next_digit(money, &divisor);
        let byte = &mut buffer[place as usize / 2];
        *byte = if place % 2 == 0 { digit << 4 } else { *byte | digit };
        d -= 1;
        if d == 0 {
            break;
        }
    }
    let mut quotient = buffer;
    for _ in 0..6 - digits_wanted {
        div_by_10(&mut quotient);
    }
    quotient
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlagAction {
    Reset,
    Set,
    Test,
}

/// `FlagAction` on bit `bit` of `flags`, returning its `c`: the byte after a set or reset, and
/// the flag's bit in place, not shifted down, after a test.
pub fn flag_action(flags: &mut [u8], bit: u8, action: FlagAction) -> u8 {
    let (byte, mask) = (&mut flags[bit as usize / 8], 1u8 << (bit % 8));
    match action {
        FlagAction::Reset => { *byte &= !mask; *byte }
        FlagAction::Set => { *byte |= mask; *byte }
        FlagAction::Test => *byte & mask,
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use super::*;

    #[test]
    fn multiply_as_harvested() {
        for ((a, b), product, _) in cases::<(u32, u8), u32>(include_str!("../../fixtures/math/multiply.jsonl")) {
            assert_eq!(multiply(a, b), product, "{a} × {b}");
        }
    }

    #[test]
    fn divide_as_harvested() {
        for ((dividend, divisor, bytes), expected, _) in cases::<([u8; 4], u8, u8), ([u8; 4], u8)>(include_str!("../../fixtures/math/divide.jsonl")) {
            assert_eq!(divide(dividend, divisor, bytes), expected, "{dividend:02X?} / {divisor}, b = {bytes}");
        }
    }

    #[test]
    fn add_and_sub_bcd_as_harvested() {
        for ((mut d, s), (expected, _carry), _) in cases::<(Vec<u8>, Vec<u8>), (Vec<u8>, bool)>(include_str!("../../fixtures/math/add_bcd.jsonl")) {
            let original = d.clone();
            add_bcd(&mut d, &s);
            assert_eq!(d, expected, "{original:02X?} + {s:02X?}");
        }
        for ((mut d, s), (expected, carry), _) in cases::<(Vec<u8>, Vec<u8>), (Vec<u8>, bool)>(include_str!("../../fixtures/math/sub_bcd.jsonl")) {
            let original = d.clone();
            let borrow = sub_bcd(&mut d, &s);
            assert_eq!((d, borrow), (expected, carry), "{original:02X?} - {s:02X?}");
        }
    }

    #[test]
    fn divide_bcd_as_harvested() {
        for ((mut money, divisor), (quotient, left), _) in cases::<([u8; 3], [u8; 3]), ([u8; 3], [u8; 3])>(include_str!("../../fixtures/math/divide_bcd.jsonl")) {
            let original = money;
            assert_eq!((divide_bcd(&mut money, divisor), money), (quotient, left), "{original:02X?} / {divisor:02X?}");
        }
    }

    #[test]
    fn flag_action_as_harvested() {
        for ((mut flags, bit, action), (expected, c), _) in cases::<(Vec<u8>, u8, u8), (Vec<u8>, u8)>(include_str!("../../fixtures/math/flag_action.jsonl")) {
            let action = [FlagAction::Reset, FlagAction::Set, FlagAction::Test][action as usize];
            assert_eq!(flag_action(&mut flags, bit, action), c, "{action:?} {bit}");
            assert_eq!(flags, expected);
        }
    }
}
