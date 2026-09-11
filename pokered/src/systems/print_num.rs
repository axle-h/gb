use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};

const ZERO: u8 = 0xF6;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NumberFormat {
    pub digits: u8,
    pub leading_zeroes: bool,
    pub left_align: bool,
}

/// `PrintNumber` of up to three bytes at tile index `at`, returning where `hl` ends. As on the
/// cartridge, a value too wide piles into the top digit, past '9', and the tens read the low byte.
pub fn print_number(ui: &mut UiSurface, mut at: usize, value: u32, format: NumberFormat) -> usize {
    assert!((2..=7).contains(&format.digits), "PrintNumber prints 2 to 7 digits");
    let mut rest = value & 0xFF_FFFF;
    // `hPastLeadingZeros`: the last high digit's tile, which is zero when '0' + 10 wraps.
    let mut past = 0u8;
    let next_digit = |at: &mut usize, past: u8| {
        if format.leading_zeroes || !format.left_align || past != 0 {
            *at += 1;
        }
    };
    let put = |ui: &mut UiSurface, at: usize, tile: u8| ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
    for place in (2..format.digits as u32).rev() {
        let power = 10u32.pow(place);
        let digit = (rest / power) as u8;
        rest %= power;
        if past | digit == 0 {
            if format.leading_zeroes { put(ui, at, ZERO); }
        } else {
            past = ZERO.wrapping_add(digit);
            put(ui, at, past);
        }
        next_digit(&mut at, past);
    }
    let low = (rest & 0xFF) as u8;
    let (tens, ones) = (low / 10, low % 10);
    past |= tens;
    if past == 0 {
        if format.leading_zeroes { put(ui, at, ZERO); }
    } else {
        put(ui, at, ZERO.wrapping_add(tens));
    }
    next_digit(&mut at, past);
    put(ui, at, ZERO + ones);
    at + 1
}

const YEN: u8 = 0xF0;

/// `PrintBCDNumber`'s flags in `c`, beside the length.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BcdFormat {
    pub skip_leading_zeroes: bool,
    pub left_align: bool,
    pub money_sign: bool,
}

/// `PrintBCDNumber` of `digits` at tile index `at`, returning where `hl` ends. Each tile it writes
/// is followed, on the cartridge, by a `PrintLetterDelay` that the caller's pacing supplies.
pub fn print_bcd(ui: &mut UiSurface, mut at: usize, digits: &[u8], format: BcdFormat) -> usize {
    let put = |ui: &mut UiSurface, at: &mut usize, tile: u8| {
        ui.set(*at % SCREEN_TILES_X, *at / SCREEN_TILES_X, tile);
        *at += 1;
    };
    let (mut skipping, mut money) = (format.skip_leading_zeroes, format.money_sign);
    if money && !skipping {
        put(ui, &mut at, YEN);
    }
    for digit in digits.iter().flat_map(|&byte| [byte >> 4, byte & 0x0F]) {
        if digit != 0 && skipping {
            if money {
                put(ui, &mut at, YEN);
                money = false;
            }
            skipping = false;
        }
        if digit != 0 || !skipping {
            put(ui, &mut at, ZERO.wrapping_add(digit));
        } else if !format.left_align {
            at += 1;
        }
    }
    if skipping {
        if !format.left_align {
            at -= 1;
        }
        if money {
            put(ui, &mut at, YEN);
        }
        put(ui, &mut at, ZERO);
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printed(value: u32, format: NumberFormat) -> (Vec<u8>, usize) {
        let mut ui = UiSurface::default();
        let end = print_number(&mut ui, 0, value, format);
        (ui.row(0)[..format.digits as usize].to_vec(), end)
    }

    const BLANK: u8 = UiSurface::BLANK;

    #[test]
    fn leading_zeroes_are_skipped_in_place() {
        let two = NumberFormat { digits: 2, ..NumberFormat::default() };
        assert_eq!(printed(5, two), (vec![BLANK, ZERO + 5], 2));
        assert_eq!(printed(42, two), (vec![ZERO + 4, ZERO + 2], 2));
    }

    #[test]
    fn left_aligned_numbers_start_where_they_are_printed() {
        let three = NumberFormat { digits: 3, left_align: true, ..NumberFormat::default() };
        assert_eq!(printed(7, three), (vec![ZERO + 7, BLANK, BLANK], 1));
    }

    #[test]
    fn leading_zeroes_print_when_asked() {
        let three = NumberFormat { digits: 3, leading_zeroes: true, ..NumberFormat::default() };
        assert_eq!(printed(7, three).0, [ZERO, ZERO, ZERO + 7]);
    }

    #[test]
    fn a_value_too_wide_for_its_digits_overflows_the_top_one() {
        let two = NumberFormat { digits: 2, ..NumberFormat::default() };
        assert_eq!(printed(150, two).0, [ZERO.wrapping_add(15), ZERO]);
    }

    fn printed_as_harvested(print: impl Fn(&mut UiSurface, &[u8], u8, u8) -> usize, fixture: &str) {
        for ((value, b, c), (tiles, end), _) in crate::fixtures::cases::<(Vec<u8>, u8, u8), (Vec<u8>, usize)>(fixture) {
            let mut ui = UiSurface::default();
            let at = print(&mut ui, &value, b, c);
            assert_eq!((ui.row(0)[..tiles.len()].to_vec(), at), (tiles, end), "{value:02X?}, b = ${b:02X}, c = ${c:02X}");
        }
    }

    #[test]
    fn print_number_as_harvested() {
        printed_as_harvested(|ui, value, b, c| {
            let number = value.iter().fold(0u32, |n, &byte| n << 8 | byte as u32);
            let format = NumberFormat { digits: c, leading_zeroes: b & 0x80 != 0, left_align: b & 0x40 != 0 };
            print_number(ui, 0, number, format)
        }, include_str!("../../fixtures/math/print_number.jsonl"));
    }

    #[test]
    fn print_bcd_as_harvested() {
        for ((value, c), (tiles, end), _) in crate::fixtures::cases::<(Vec<u8>, u8), (Vec<u8>, usize)>(include_str!("../../fixtures/math/print_bcd.jsonl")) {
            let format = BcdFormat { skip_leading_zeroes: c & 0x80 != 0, left_align: c & 0x40 != 0, money_sign: c & 0x20 != 0 };
            let mut ui = UiSurface::default();
            let at = print_bcd(&mut ui, 0, &value, format);
            assert_eq!((ui.row(0)[..tiles.len()].to_vec(), at), (tiles, end), "{value:02X?}, c = ${c:02X}");
        }
    }
}
