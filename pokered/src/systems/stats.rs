use serde::{Deserialize, Serialize};

/// `c` for `CalcStat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stat {
    Hp = 1,
    Attack,
    Defense,
    Speed,
    Special,
}

/// A mon's two DV bytes: attack and defense, then speed and special, a nybble each, high first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dvs(pub [u8; 2]);

impl Dvs {
    /// The HP DV is the low bit of each of the others.
    pub fn get(self, stat: Stat) -> u8 {
        let [attack_defense, speed_special] = self.0;
        match stat {
            Stat::Attack => attack_defense >> 4,
            Stat::Defense => attack_defense & 0xF,
            Stat::Speed => speed_special >> 4,
            Stat::Special => speed_special & 0xF,
            Stat::Hp => (attack_defense >> 4 & 1) << 3 | (attack_defense & 1) << 2
                | (speed_special >> 4 & 1) << 1 | speed_special & 1,
        }
    }
}

pub const MAX_STAT_VALUE: u16 = 999;

/// `calc_stat`'s arguments, as its fixture stores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatInput {
    pub stat: Stat,
    pub base: u8,
    pub dvs: Dvs,
    pub stat_exp: Option<u16>,
    pub level: u8,
}

/// `CalcStat`. `stat_exp` is `None` where the caller passes `b = 0`.
pub fn calc_stat(stat: Stat, base: u8, dvs: Dvs, stat_exp: Option<u16>, level: u8) -> u16 {
    let bonus = stat_exp.map_or(0, stat_exp_bonus) / 4;
    let scaled = ((base as u32 + dvs.get(stat) as u32) * 2 + bonus as u32) * level as u32 / 100;
    let value = scaled + if stat == Stat::Hp { level as u32 + 10 } else { 5 };
    (value as u16).min(MAX_STAT_VALUE)
}

/// `CalcStats`: all five, from a species' base stats, in `CalcStat`'s order.
pub fn calc_stats(base: [u8; 5], dvs: Dvs, stat_exp: Option<[u16; 5]>, level: u8) -> [u16; 5] {
    const STATS: [Stat; 5] = [Stat::Hp, Stat::Attack, Stat::Defense, Stat::Speed, Stat::Special];
    STATS.map(|stat| {
        let i = stat as usize - 1;
        calc_stat(stat, base[i], dvs, stat_exp.map(|exp| exp[i]), level)
    })
}

/// `.statExpLoop`: the least `b` from 1 with `b * b >= stat_exp`, stopping at 255, so zero stat
/// experience still counts 1.
fn stat_exp_bonus(stat_exp: u16) -> u8 {
    (1..=255).find(|&b: &u16| b * b >= stat_exp).unwrap_or(255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Case {
        input: StatInput,
        output: u16,
    }

    #[test]
    fn every_harvested_case_of_calc_stat() {
        let cases: Vec<Case> = include_str!("../../fixtures/stats/calc_stat.jsonl").lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(cases.len() >= 300, "{} cases", cases.len());
        for Case { input: i, output } in cases {
            assert_eq!(calc_stat(i.stat, i.base, i.dvs, i.stat_exp, i.level), output,
                "{i:?}");
        }
    }
}
