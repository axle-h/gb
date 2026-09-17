//! The day care: `IncrementDayCareMonExp` every step, and what `DaycareGentlemanText` works out when
//! the mon is collected.

use serde::{Deserialize, Serialize};
use crate::party::BoxMon;
use crate::systems::experience::{calc_experience, calc_level_from_experience};
use crate::systems::math::add_bcd;

/// `MAX_LEVEL`.
pub const MAX_LEVEL: u8 = 100;

/// `IncrementDayCareMonExp`: one point, and only when the carry reaches the top byte is that byte
/// held to `$50`, so the low two bytes run on past `$500000`.
pub fn increment_day_care_mon_exp(exp: u32) -> u32 {
    let exp = exp.wrapping_add(1) & 0xFF_FFFF;
    if exp & 0xFFFF != 0 || exp >> 16 < 0x50 {
        return exp;
    }
    0x50_0000
}

/// `.daycareInUse` up to the price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    /// The level the experience makes, held to 100 with the experience cut to 100's.
    pub level: u8,
    pub exp: u32,
    /// `wDayCareStartLevel`: the box level it was left at.
    pub start_level: u8,
    /// `wDayCareNumLevelsGrown`.
    pub levels_grown: u8,
    /// `wDayCareTotalCost`, BCD: ¥100 for being taken in and ¥100 a level.
    pub cost: [u8; 2],
}

pub fn collection(mon: &BoxMon) -> Collection {
    let growth = mon.base_stats().growth_rate;
    let mut level = calc_level_from_experience(growth, mon.exp);
    let mut exp = mon.exp;
    if level >= MAX_LEVEL {
        level = MAX_LEVEL;
        exp = calc_experience(growth, MAX_LEVEL);
    }
    let start_level = mon.box_level;
    let levels_grown = if start_level == level { 0 } else { level.wrapping_sub(start_level) };
    let mut cost = [0, 0];
    let mut times = levels_grown.wrapping_add(1);
    loop {
        add_bcd(&mut cost, &[0x01, 0x00]);
        times = times.wrapping_sub(1);
        if times == 0 {
            break;
        }
    }
    Collection { level, exp, start_level, levels_grown, cost }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use super::*;

    #[test]
    fn every_harvested_case_of_increment_day_care_mon_exp() {
        let jsonl = include_str!("../../../fixtures/events/increment_day_care_mon_exp.jsonl");
        for (exp, expected, _) in cases::<u32, u32>(jsonl) {
            assert_eq!(increment_day_care_mon_exp(exp), expected, "{exp:#08x}");
        }
    }

    #[test]
    fn every_harvested_case_of_the_day_care_s_level_and_price() {
        let jsonl = include_str!("../../../fixtures/events/day_care_collection.jsonl");
        for (mon, expected, _) in cases::<BoxMon, Collection>(jsonl) {
            assert_eq!(collection(&mon), expected, "{} at box level {} with {} exp", mon.species, mon.box_level, mon.exp);
        }
    }

    #[test]
    fn the_experience_is_held_to_50_only_as_the_carry_reaches_the_top_byte() {
        assert_eq!(increment_day_care_mon_exp(0x00_FFFF), 0x01_0000);
        assert_eq!(increment_day_care_mon_exp(0x50_0000), 0x50_0001);
        assert_eq!(increment_day_care_mon_exp(0x4F_FFFF), 0x50_0000);
        assert_eq!(increment_day_care_mon_exp(0x50_FFFF), 0x50_0000);
        assert_eq!(increment_day_care_mon_exp(0xFF_FFFF), 0x00_0000, "the top byte wraps to 0, below $50");
    }
}
