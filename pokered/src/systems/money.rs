//! `home/money.asm` and `engine/items/subtract_paid_money.asm`: money and coins, as the three- and
//! two-byte BCD numbers the cartridge keeps, over `home/compare.asm`'s `StringCmp`.

use super::math::{add_bcd, sub_bcd};

/// `StringCmp`: most significant byte first, stopping at the first that differs. BCD bytes order
/// the same way as their digits, so this is the numbers' own order.
pub fn has_enough(held: &[u8], price: &[u8]) -> bool {
    held >= price
}

/// `SubtractAmountPaidFromMoney_`: false, and nothing spent, when there is not enough.
pub fn subtract_paid(money: &mut [u8; 3], price: &[u8; 3]) -> bool {
    if !has_enough(money, price) {
        return false;
    }
    sub_bcd(money, price);
    true
}

/// `AddAmountSoldToMoney`. `AddBCD` saturates, so money stops at ¥999999.
pub fn add_sold(money: &mut [u8; 3], price: &[u8; 3]) {
    add_bcd(money, price);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three BCD bytes, most significant first.
    fn bcd(mut n: u32) -> [u8; 3] {
        let mut bytes = [0; 3];
        for byte in bytes.iter_mut().rev() {
            *byte = (((n / 10) % 10) as u8) << 4 | (n % 10) as u8;
            n /= 100;
        }
        bytes
    }

    #[test]
    fn a_purchase_needs_the_money_and_then_spends_it() {
        let mut money = bcd(3000);
        assert!(subtract_paid(&mut money, &bcd(1200)));
        assert_eq!(money, bcd(1800));
        assert!(!subtract_paid(&mut money, &bcd(9999)), "not enough");
        assert_eq!(money, bcd(1800), "and nothing is spent");
        assert!(subtract_paid(&mut money, &bcd(1800)), "exactly enough");
        assert_eq!(money, bcd(0));
    }

    #[test]
    fn a_sale_saturates_at_six_digits() {
        let mut money = bcd(999000);
        add_sold(&mut money, &bcd(500));
        assert_eq!(money, bcd(999500));
        add_sold(&mut money, &bcd(1000));
        assert_eq!(money, bcd(999999), "AddBCD fills every byte on an overflow");
    }
}
