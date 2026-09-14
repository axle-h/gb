//! The PP arithmetic behind the PP items: `GetMaxPP`, `AddBonusPP`, `RestoreBonusPP` and what
//! `ItemUsePPUp` and `ItemUsePPRestore` do with them, from `engine/items/item_effects.asm`.

use poke_core::move_name::PokemonMoveName;
use poke_core::moves::MoveData;
use serde::{Deserialize, Serialize};
use super::math::divide;

/// A move's PP byte: PP left in the low six bits, PP Ups used in the top two.
pub const PP_MASK: u8 = 0b0011_1111;
pub const PP_UP_MASK: u8 = 0b1100_0000;
pub const MAX_PP_UPS: u8 = 3;

pub fn pp_left(pp: u8) -> u8 {
    pp & PP_MASK
}

pub fn pp_ups(pp: u8) -> u8 {
    (pp & PP_UP_MASK) >> 6
}

/// `AddBonusPP`: a fifth of the move's normal max PP, capped at 7, once per PP Up used — or once
/// only when a PP Up is being used right now.
///
/// With no PP Ups and no PP Up being used the cartridge runs its loop 256 times rather than none,
/// because it decrements the count before testing it. Adding one byte 256 times comes back to
/// where it started, so the bonus is nothing either way.
pub fn add_bonus_pp(pp: u8, normal_max: u8, using_pp_up: bool) -> u8 {
    let (quotient, _) = divide([0, 0, 0, normal_max], 5, 4);
    let bonus = quotient[3].min(7);
    let times = if using_pp_up { 1 } else { pp_ups(pp) };
    pp.wrapping_add(bonus.wrapping_mul(times))
}

/// `GetMaxPP`: the move's normal max PP with the bonus for the PP Ups already used, in six bits.
pub fn max_pp(mv: PokemonMoveName, pp: u8) -> u8 {
    let normal_max = MoveData::of_move(mv).pp;
    add_bonus_pp(pp & PP_UP_MASK | normal_max, normal_max, false) & PP_MASK
}

/// `.restorePP`: the new PP byte, or `None` where the item would do nothing.
///
/// `full` is a Max Ether or Max Elixir, and compares the whole byte against the max rather than
/// masking the PP Up count out of it first, so a move that has had any PP Up used on it never
/// reads as already full.
pub fn restore_pp(pp: u8, mv: PokemonMoveName, full: bool) -> Option<u8> {
    let max = max_pp(mv, pp);
    let restored = if full {
        (pp != max).then_some(max)?
    } else {
        let left = pp_left(pp);
        (left != max).then_some(())?;
        (left + 10).min(max)
    };
    Some(pp & PP_UP_MASK | restored)
}

/// `ItemUsePPUp`: the new PP byte, with the bonus the extra PP Up is worth added to what is left,
/// or `None` when three have been used already.
pub fn use_pp_up(pp: u8, mv: PokemonMoveName) -> Option<u8> {
    if pp_ups(pp) >= MAX_PP_UPS {
        return None;
    }
    Some(add_bonus_pp(pp + (1 << 6), MoveData::of_move(mv).pp, true))
}

/// A PP case as its fixtures store it: the move's PP byte, the move, and for an ether whether it
/// is a Max Ether.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PpInput {
    pub pp: u8,
    pub mv: PokemonMoveName,
    pub full: bool,
}

#[cfg(test)]
mod tests {
    use poke_core::move_name::PokemonMoveName::*;
    use crate::fixtures::cases;
    use super::*;

    #[test]
    fn every_harvested_ether_matches() {
        for (input, output, _) in cases::<PpInput, Option<u8>>(include_str!("../../fixtures/items/restore_pp.jsonl")) {
            assert_eq!(restore_pp(input.pp, input.mv, input.full), output, "{input:?}");
        }
    }

    #[test]
    fn every_harvested_pp_up_matches() {
        for (input, output, _) in cases::<PpInput, Option<u8>>(include_str!("../../fixtures/items/use_pp_up.jsonl")) {
            assert_eq!(use_pp_up(input.pp, input.mv), output, "{input:?}");
        }
    }

    #[test]
    fn a_move_with_no_pp_ups_has_its_own_max() {
        assert_eq!(max_pp(Tackle, 35), 35);
        assert_eq!(max_pp(Pound, 35), 35);
    }

    #[test]
    fn each_pp_up_is_worth_a_fifth_of_the_normal_max_capped_at_seven() {
        assert_eq!(max_pp(Tackle, 1 << 6), 35 + 7, "35 / 5 is 7");
        assert_eq!(max_pp(Tackle, 3 << 6), 35 + 21);
        assert_eq!(max_pp(HornDrill, 1 << 6), 5 + 1, "5 / 5 is 1");
        assert_eq!(max_pp(Psychic, 3 << 6), 10 + 6, "10 / 5 is 2, three times");
    }

    #[test]
    fn an_ether_adds_ten_and_stops_at_the_max() {
        assert_eq!(restore_pp(20, Tackle, false), Some(30));
        assert_eq!(restore_pp(30, Tackle, false), Some(35), "capped, not 40");
        assert_eq!(restore_pp(35, Tackle, false), None, "already full");
    }

    #[test]
    fn a_max_ether_fills_it_but_misreads_a_move_with_pp_ups() {
        assert_eq!(restore_pp(0, Tackle, true), Some(35));
        assert_eq!(restore_pp(35, Tackle, true), None);
        let full = 1 << 6 | 42;
        assert_eq!(max_pp(Tackle, full), 42, "35 and one PP Up");
        assert_eq!(restore_pp(full, Tackle, true), Some(full),
            "the byte is 106 and the max is 42, so a full move still takes the Max Ether");
    }

    #[test]
    fn three_pp_ups_is_the_limit() {
        let one = use_pp_up(35, Tackle).expect("the first");
        assert_eq!((pp_ups(one), pp_left(one)), (1, 42), "the bonus lands on what is left too");
        assert_eq!(pp_ups(use_pp_up(one, Tackle).unwrap()), 2);
        assert_eq!(use_pp_up(3 << 6 | 35, Tackle), None);
    }
}
