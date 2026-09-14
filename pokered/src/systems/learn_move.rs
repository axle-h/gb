//! What `LearnMove` and the status screen's second page decide without waiting on a frame:
//! `FormatMovesString` and `IsMoveHM`.
//!
//! Exact: a full set of four names ends in a `<NEXT>` before its `@`, and a list with a gap leaves
//! `wNumMovesMinusOne` wherever the last caller left it when the gap is in the first slot.

use poke_core::move_name::PokemonMoveName;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_symbols;
use serde::{Deserialize, Serialize};
use crate::party::NUM_MOVES;

const NEXT: u8 = 0x4E;
const TERMINATOR: u8 = 0x50;
const DASH: u8 = 0xE3;

/// What `FormatMovesString` leaves: `wMovesString` up to and including its `@`, and
/// `wNumMovesMinusOne`, which it writes only for a name and so not at all when the first slot is
/// empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovesString {
    pub string: Vec<u8>,
    pub num_moves_minus_one: Option<u8>,
}

/// `FormatMovesString`: the names up to the first empty slot, each followed by `<NEXT>`, then a
/// `-` for every slot left, the dashes separated rather than followed.
pub fn format_moves_string(moves: &[Option<PokemonMoveName>; NUM_MOVES]) -> MovesString {
    let mut string = Vec::new();
    let mut num_moves_minus_one = None;
    let mut slot = 0;
    while slot < NUM_MOVES {
        let Some(known) = moves[slot] else { break };
        string.extend(known.name());
        num_moves_minus_one = Some(slot as u8);
        string.push(NEXT);
        slot += 1;
    }
    if slot < NUM_MOVES {
        loop {
            string.push(DASH);
            slot += 1;
            if slot == NUM_MOVES {
                break;
            }
            string.push(NEXT);
        }
    }
    string.push(TERMINATOR);
    MovesString { string, num_moves_minus_one }
}

/// `IsMoveHM`, over `HMMoves`.
pub fn is_move_hm(known: PokemonMoveName) -> bool {
    rom_slice(pokered_symbols::HMMoves).iter().take_while(|&&id| id != 0xFF).any(|&id| id == known as u8)
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use super::*;
    use PokemonMoveName::*;

    #[test]
    fn every_harvested_case_of_format_moves_string() {
        for (moves, output, _) in cases::<[Option<PokemonMoveName>; NUM_MOVES], MovesString>(
            include_str!("../../fixtures/status_screen/format_moves_string.jsonl")) {
            assert_eq!(format_moves_string(&moves), output, "{moves:?}");
        }
    }

    #[test]
    fn four_names_end_in_a_next_and_two_end_in_two_dashes() {
        let full = format_moves_string(&[Some(Cut), Some(Fly), Some(Surf), Some(Flash)]);
        assert_eq!(full.string[full.string.len() - 2..], [NEXT, TERMINATOR]);
        assert_eq!(full.num_moves_minus_one, Some(3));
        let two = format_moves_string(&[Some(Cut), Some(Fly), None, None]);
        assert_eq!(two.string[two.string.len() - 4..], [DASH, NEXT, DASH, TERMINATOR]);
        assert_eq!(format_moves_string(&[None; 4]).num_moves_minus_one, None);
    }

    #[test]
    fn the_five_hms_and_nothing_else() {
        assert!([Cut, Fly, Surf, Strength, Flash].into_iter().all(is_move_hm));
        assert!(!is_move_hm(Tackle) && !is_move_hm(Dig) && !is_move_hm(Teleport));
    }
}
