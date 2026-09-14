//! `home/money.asm` and `engine/items/subtract_paid_money.asm`: money and coins, as the three- and
//! two-byte BCD numbers the cartridge keeps, over `home/compare.asm`'s `StringCmp`.

use super::math::{add_bcd, divide_bcd, sub_bcd};

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

/// `DisplayChooseQuantityMenu`'s `hMoney`: one price added `quantity` times, then halved for a
/// sale. The count is tested after it is decremented, so a quantity of 0 adds the price 256 times;
/// `AddBCD` saturates, so a large enough purchase costs ¥999999.
pub fn total_price(each: [u8; 3], quantity: u8, halved: bool) -> [u8; 3] {
    let mut total = [0u8; 3];
    let mut count = quantity;
    loop {
        add_bcd(&mut total, &each);
        count = count.wrapping_sub(1);
        if count == 0 {
            break;
        }
    }
    if halved {
        total = divide_bcd(&mut total, [0, 0, 2]);
    }
    total
}

/// A price case as its fixture stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PriceInput {
    pub each: [u8; 3],
    pub quantity: u8,
    pub halved: bool,
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use super::*;

    #[test]
    fn a_sale_is_half_the_price_of_the_same_purchase() {
        assert_eq!(total_price(bcd(300), 3, false), bcd(900));
        assert_eq!(total_price(bcd(300), 3, true), bcd(450));
        assert_eq!(total_price(bcd(1), 1, true), bcd(0), "the half penny goes");
        assert_eq!(total_price(bcd(10000), 99, false), bcd(990000));
        assert_eq!(total_price(bcd(20000), 60, false), bcd(999999), "AddBCD saturates");
    }

    /// The price, the screen `.handleNewQuantity` prints it to, from the quantity to the border.
    #[test]
    fn every_harvested_price_matches() {
        for (input, (total, _row), _) in cases::<PriceInput, ([u8; 3], Vec<u8>)>(include_str!("../../fixtures/items/choose_quantity_price.jsonl")) {
            assert_eq!(total_price(input.each, input.quantity, input.halved), total, "{input:?}");
        }
    }

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
