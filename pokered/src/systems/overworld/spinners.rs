//! The arrow tiles of the Rocket Hideout and the Viridian Gym: which way the one under the player
//! sends them, and which facing the spin turns to next.

/// A map's `ArrowTilePlayerMovement`: an arrow's square and the `(press, count)` runs it makes.
pub type ArrowTable = &'static [((u8, u8), &'static [(u8, u8)])];

/// `SpinnerPlayerFacingDirections`, which holds the facing that comes *next* rather than the facings
/// in their own order.
pub fn spinner_facing(image_index: u8) -> u8 {
    poke_core::tables::SPINNER_PLAYER_FACING_DIRECTIONS[(image_index >> 2) as usize & 3]
}

/// `DecodeArrowMovementRLE`'s search of a map's `ArrowTilePlayerMovement`: the RLE list of presses
/// the arrow at (`x`, `y`) makes.
pub fn arrow_movement(table: ArrowTable, x: u8, y: u8) -> Option<&'static [(u8, u8)]> {
    table.iter().find(|&&(at, _)| at == (x, y)).map(|&(_, list)| list)
}

#[cfg(test)]
mod tests {
    use poke_core::tables::{ROCKET_HIDEOUT_B2F_ARROWS, VIRIDIAN_GYM_ARROWS};
    use super::*;
    use crate::input::Joypad;
    use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_LEFT, SPRITE_FACING_RIGHT, SPRITE_FACING_UP};

    #[test]
    fn a_spin_runs_down_left_up_right_and_round_again() {
        assert_eq!(spinner_facing(SPRITE_FACING_DOWN), SPRITE_FACING_LEFT);
        assert_eq!(spinner_facing(SPRITE_FACING_LEFT), SPRITE_FACING_UP);
        assert_eq!(spinner_facing(SPRITE_FACING_UP), SPRITE_FACING_RIGHT);
        assert_eq!(spinner_facing(SPRITE_FACING_RIGHT), SPRITE_FACING_DOWN);
    }

    /// The walking frame of an image index is the low two bits, which the spin drops.
    #[test]
    fn a_walking_frame_spins_the_same_way_as_a_standing_one() {
        assert_eq!(spinner_facing(SPRITE_FACING_DOWN + 3), SPRITE_FACING_LEFT);
    }

    #[test]
    fn only_a_square_with_an_arrow_on_it_has_a_movement_list() {
        let table = ROCKET_HIDEOUT_B2F_ARROWS;
        // `RocketHideout2ArrowMovement1`, `db PAD_LEFT, 2`, which (4, 9) and (4, 19) share.
        let first = arrow_movement(table, 4, 9).expect("the first arrow is in the table");
        assert_eq!(first, [(Joypad::LEFT.bits(), 2)]);
        assert_eq!(arrow_movement(table, 4, 19), Some(first), "and two squares may share a list");
        assert_eq!(arrow_movement(table, 3, 9), None);
        assert_eq!(arrow_movement(VIRIDIAN_GYM_ARROWS, 4, 9), None, "the gym's arrows are its own");
    }
}
