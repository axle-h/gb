//! `GetMonFieldMoves`: which of a mon's four moves can be used outside a battle, and how far left
//! the widest of their names reaches.
//!
//! The column is the menu's, not the move's: the box is sized from the leftmost name, so one
//! `SOFTBOILED` widens it for everything above it.

use poke_core::move_name::PokemonMoveName;
use serde::{Deserialize, Serialize};

/// `wFieldMovesLeftmostXCoord` before anything narrows it.
pub const DEFAULT_LEFTMOST: u8 = 12;

/// `FieldMoveDisplayData`: the move, its index into `FieldMoveNames`, and its leftmost tile.
/// `ANIM_B4` is in the cartridge's table between `FLY` and `SURF` with an empty name; no move can
/// be it, so it is left out here and the indices keep the gap it leaves.
const DISPLAY: [(PokemonMoveName, u8, u8); 8] = [
    (PokemonMoveName::Cut, 1, 0x0C),
    (PokemonMoveName::Fly, 2, 0x0C),
    (PokemonMoveName::Surf, 4, 0x0C),
    (PokemonMoveName::Strength, 5, 0x0A),
    (PokemonMoveName::Flash, 6, 0x0C),
    (PokemonMoveName::Dig, 7, 0x0C),
    (PokemonMoveName::Teleport, 8, 0x0A),
    (PokemonMoveName::Softboiled, 9, 0x08),
];

/// `GetMonFieldMoves`' arguments: the mon's four move slots, as raw ids with 0 for an empty one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMovesInput {
    pub moves: [u8; 4],
}

/// What it leaves behind: `wFieldMoves` and `wFieldMovesLeftmostXCoord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMoves {
    /// Indices into `FieldMoveNames`, counting from 1, in the order the mon knows them.
    pub names: Vec<u8>,
    pub leftmost: u8,
}

impl Default for FieldMoves {
    fn default() -> Self {
        Self { names: Vec::new(), leftmost: DEFAULT_LEFTMOST }
    }
}

impl FieldMoves {
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// `FieldMoveNames`, indexed from 1. Index 3 is the empty name `ANIM_B4` would have used.
pub fn name_of(index: u8) -> &'static str {
    match index {
        1 => "CUT",
        2 => "FLY",
        4 => "SURF",
        5 => "STRENGTH",
        6 => "FLASH",
        7 => "DIG",
        8 => "TELEPORT",
        9 => "SOFTBOILED",
        _ => "",
    }
}

/// The move a name index belongs to, which is how a chosen row becomes a move again.
pub fn move_of(index: u8) -> Option<PokemonMoveName> {
    DISPLAY.iter().find(|(_, name, _)| *name == index).map(|&(found, _, _)| found)
}

/// `GetMonFieldMoves`. An empty slot ends the scan rather than being skipped, so a move after a
/// gap is never reached.
pub fn field_moves(moves: [u8; 4]) -> FieldMoves {
    let mut found = FieldMoves::default();
    for &id in moves.iter() {
        if id == 0 {
            break;
        }
        if let Some(&(_, name, column)) = DISPLAY.iter().find(|&&(one, _, _)| one as u8 == id) {
            found.names.push(name);
            found.leftmost = found.leftmost.min(column);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::cases;

    fn ids(moves: [PokemonMoveName; 4]) -> [u8; 4] {
        moves.map(|one| one as u8)
    }

    #[test]
    fn every_harvested_case_of_get_mon_field_moves() {
        for (input, expected, _) in cases::<FieldMovesInput, FieldMoves>(include_str!("../../fixtures/field_moves/get_mon_field_moves.jsonl")) {
            assert_eq!(field_moves(input.moves), expected, "{:?}", input.moves);
        }
    }

    #[test]
    fn a_mon_with_none_keeps_the_default_column() {
        let none = ids([PokemonMoveName::Tackle; 4]);
        assert_eq!(field_moves(none), FieldMoves::default());
    }

    /// The box is sized from the leftmost name, so the widest move wins for all of them.
    #[test]
    fn the_leftmost_column_is_the_smallest_of_them() {
        let cut = ids([PokemonMoveName::Cut, PokemonMoveName::Tackle, PokemonMoveName::Tackle, PokemonMoveName::Tackle]);
        assert_eq!(field_moves(cut), FieldMoves { names: vec![1], leftmost: 0x0C });
        let with_strength = ids([PokemonMoveName::Cut, PokemonMoveName::Strength, PokemonMoveName::Tackle, PokemonMoveName::Tackle]);
        assert_eq!(field_moves(with_strength), FieldMoves { names: vec![1, 5], leftmost: 0x0A });
        let with_softboiled = ids([PokemonMoveName::Softboiled, PokemonMoveName::Cut, PokemonMoveName::Tackle, PokemonMoveName::Tackle]);
        assert_eq!(field_moves(with_softboiled), FieldMoves { names: vec![9, 1], leftmost: 0x08 });
    }

    /// A zero ends the scan, so a field move behind an empty slot is never seen.
    #[test]
    fn an_empty_slot_ends_the_scan() {
        assert_eq!(field_moves([0, PokemonMoveName::Cut as u8, 0, 0]), FieldMoves::default());
        assert_eq!(field_moves([PokemonMoveName::Cut as u8, 0, PokemonMoveName::Fly as u8, 0]),
            FieldMoves { names: vec![1], leftmost: 0x0C });
    }

    #[test]
    fn a_name_index_goes_back_to_the_move_it_came_from() {
        assert_eq!(move_of(5), Some(PokemonMoveName::Strength));
        assert_eq!(name_of(5), "STRENGTH");
        assert_eq!(move_of(3), None, "the name ANIM_B4 would have used");
        assert_eq!(name_of(3), "");
    }
}
