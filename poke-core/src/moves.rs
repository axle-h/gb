use serde::{Deserialize, Serialize};
use crate::move_name::PokemonMoveName;
use crate::rom_gfx::rom_slice;
use crate::symbols::pokered_symbols;

const MOVE_LENGTH: usize = 6;

/// One row of `Moves`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveData {
    pub animation: u8,
    pub effect: u8,
    pub power: u8,
    pub move_type: u8,
    /// Out of 255: the `percent` macro's `* $ff / 100`.
    pub accuracy: u8,
    pub pp: u8,
}

impl MoveData {
    pub fn of(id: u8) -> Self {
        assert!(id != 0, "NO_MOVE has no row");
        let row = &rom_slice(pokered_symbols::Moves)[(id as usize - 1) * MOVE_LENGTH..][..MOVE_LENGTH];
        Self { animation: row[0], effect: row[1], power: row[2], move_type: row[3], accuracy: row[4], pp: row[5] }
    }

    pub fn of_move(name: PokemonMoveName) -> Self {
        Self::of(name as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_move_matches_the_hand_written_table() {
        for name in (1..=u8::MAX).filter_map(PokemonMoveName::from_repr) {
            let (rom, hand) = (MoveData::of_move(name), name.metadata());
            assert_eq!(rom.animation, name as u8, "{name}");
            assert_eq!(rom.effect, hand.effect.clone() as u8, "{name}");
            // By hand a status move and a fixed-damage one both have no power; on the cartridge a
            // status move has 0 and the others 1.
            assert!(hand.power.map_or(rom.power <= 1, |power| power == rom.power), "{name}: {}", rom.power);
            assert_eq!(rom.move_type, hand.move_type as u8, "{name}");
            assert_eq!(rom.accuracy, (hand.accuracy as u16 * 0xFF / 100) as u8, "{name}");
            assert_eq!(rom.pp, hand.pp, "{name}");
        }
    }
}
