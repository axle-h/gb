//! `ItemUseBall`'s capture calculation from `engine/items/item_effects.asm`: whether the ball
//! holds, and how many times it wobbles when it does not.
//!
//! The throw itself is not harvestable as one call, because the routine waits on frames for its
//! animation, so what pins this is the `Multiply` and `Divide` under it, which are, plus the
//! worked examples below. The screens, the Safari ball count, the ghost and the box-full refusal
//! belong to the battle mode that throws.

use poke_core::item::ItemId;
use serde::{Deserialize, Serialize};
use crate::rng::Rng;
use super::math::{divide, multiply};

/// `(1 << FRZ) | SLP_MASK`: the statuses a throw counts double for.
const FROZEN_OR_ASLEEP: u8 = 1 << 5 | 0b111;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BallInput {
    pub ball: ItemId,
    /// `wEnemyMonActualCatchRate`, which a Safari Zone throw has already scaled.
    pub catch_rate: u8,
    pub hp: u16,
    pub max_hp: u16,
    pub status: u8,
}

/// `wPokeBallAnimData`, as the throw decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Throw {
    /// `$43`.
    Caught,
    /// `$20`, `$61`, `$62`, `$63`.
    Broke { shakes: u8 },
}

/// One throw. Takes one random byte, and a second only where the first did not settle it.
pub fn throw(input: &BallInput, rng: &mut impl Rng) -> Throw {
    // `.loop`: a Great Ball redraws above 200 and an Ultra or Safari Ball above 150, so the
    // number that comes out is already inside the range the ball allows.
    let mut rand1 = loop {
        let drawn = rng.random();
        match input.ball {
            ItemId::MasterBall => return Throw::Caught,
            ItemId::PokeBall => break drawn,
            _ if drawn > 200 => continue,
            ItemId::GreatBall => break drawn,
            _ if drawn > 150 => continue,
            _ => break drawn,
        }
    };

    // `.checkForAilments`: a status worth more than the number drawn holds the mon outright.
    if input.status != 0 {
        let penalty = if input.status & FROZEN_OR_ASLEEP != 0 { 25 } else { 12 };
        match rand1.checked_sub(penalty) {
            Some(left) => rand1 = left,
            None => return Throw::Caught,
        }
    }

    // W = ((MaxHP * 255) / BallFactor) / max(HP / 4, 1), every division floored.
    let ball_factor = if input.ball == ItemId::GreatBall { 8 } else { 12 };
    let (scaled, _) = divide(multiply(input.max_hp as u32, 255).to_be_bytes(), ball_factor, 4);
    // Only the low byte of a quarter of the HP is kept, and a quarter of nothing counts as one.
    let hp_quarter = ((input.hp >> 2) as u8).max(1);
    let (w, _) = divide(u32::from_be_bytes(scaled).to_be_bytes(), hp_quarter, 4);
    let over_255 = w[2] != 0;
    let x = if over_255 { 255 } else { w[3] };

    if input.catch_rate >= rand1 {
        if over_255 {
            return Throw::Caught;
        }
        if x >= rng.random() {
            return Throw::Caught;
        }
    }

    // `.failedToCapture`: Y = (CatchRate * 100) / BallFactor2, then Z = (X * Y) / 255.
    let factor2 = match input.ball {
        ItemId::PokeBall => 255,
        ItemId::GreatBall => 200,
        _ => 150,
    };
    let (y, _) = divide(multiply(input.catch_rate as u32, 100).to_be_bytes(), factor2, 4);
    if y[2] != 0 {
        return Throw::Broke { shakes: 3 };
    }
    // `Multiply` takes the low three bytes the divide left behind as its multiplicand.
    let product = multiply(u32::from_be_bytes([0, y[1], y[2], y[3]]), x);
    let (z, _) = divide(product.to_be_bytes(), 255, 4);
    let status2 = match input.status {
        0 => 0,
        s if s & FROZEN_OR_ASLEEP != 0 => 10,
        _ => 5,
    };
    let z = z[3].wrapping_add(status2);
    let shakes = if z < 10 { 0 } else if z < 30 { 1 } else if z < 70 { 2 } else { 3 };
    Throw::Broke { shakes }
}

#[cfg(test)]
mod tests {
    use crate::rng::GameRng;
    use super::*;

    fn input(ball: ItemId, catch_rate: u8, hp: u16, status: u8) -> BallInput {
        BallInput { ball, catch_rate, hp, max_hp: 100, status }
    }

    fn throw_with(input: &BallInput, tape: &[u8]) -> Throw {
        throw(input, &mut GameRng::tape(tape.to_vec()))
    }

    #[test]
    fn a_master_ball_holds_whatever_it_is_thrown_at() {
        let at = input(ItemId::MasterBall, 3, 100, 0);
        assert_eq!(throw_with(&at, &[255]), Throw::Caught);
    }

    /// MaxHP 100 and HP 100 give W = ((100 * 255) / 12) / 25 = 85, so X is 85.
    #[test]
    fn a_second_number_above_x_breaks_the_ball() {
        let at = input(ItemId::PokeBall, 255, 100, 0);
        assert_eq!(throw_with(&at, &[0, 80]), Throw::Caught, "80 is under X");
        assert_eq!(throw_with(&at, &[0, 200]), Throw::Broke { shakes: 2 },
            "Y is (255 * 100) / 255 = 100, so Z is (85 * 100) / 255 = 33");
    }

    #[test]
    fn a_number_over_the_catch_rate_never_gets_that_far() {
        let at = input(ItemId::PokeBall, 20, 100, 0);
        assert_eq!(throw_with(&at, &[21]), Throw::Broke { shakes: 0 },
            "Y is (20 * 100) / 255 = 7, Z is (85 * 7) / 255 = 2, and one number settled it");
    }

    #[test]
    fn sleep_is_worth_25_and_can_hold_it_outright() {
        let asleep = input(ItemId::PokeBall, 3, 100, 0b111);
        assert_eq!(throw_with(&asleep, &[24]), Throw::Caught, "24 is under the 25 sleep is worth");
        let burned = input(ItemId::PokeBall, 3, 100, 1 << 4);
        assert_eq!(throw_with(&burned, &[24]), Throw::Broke { shakes: 0 },
            "a burn is only worth 12, so 24 becomes 12 and beats the catch rate of 3");
    }

    #[test]
    fn a_great_ball_redraws_above_200_and_an_ultra_ball_above_150() {
        let great = input(ItemId::GreatBall, 255, 100, 0);
        assert_eq!(throw_with(&great, &[201, 0, 0]), Throw::Caught, "201 is thrown away");
        let ultra = input(ItemId::UltraBall, 255, 100, 0);
        assert_eq!(throw_with(&ultra, &[201, 151, 0, 0]), Throw::Caught, "both are thrown away");
    }

    #[test]
    fn a_nearly_fainted_mon_is_held_whatever_the_second_number_is() {
        // A quarter of 1 HP floors to 0 and counts as 1, so W is 2125 and over 255.
        let at = BallInput { hp: 1, ..input(ItemId::PokeBall, 255, 1, 0) };
        assert_eq!(throw_with(&at, &[0]), Throw::Caught);
    }
}
