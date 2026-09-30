use crate::tables::{
    TrainerParty, LONE_MOVES, TEAM_MOVES, TRAINER_AI_POINTERS, TRAINER_BASE_MONEY, TRAINER_CLASS_MOVE_CHOICE_MODIFICATIONS,
    TRAINER_PARTIES,
};

pub const NUM_TRAINERS: u8 = 47;

/// The parties of trainer class `class` (1-based, as `wTrainerClass`), in order.
pub fn parties(class: u8) -> &'static [TrainerParty] {
    assert!((1..=NUM_TRAINERS).contains(&class), "trainer class {class}");
    TRAINER_PARTIES[class as usize - 1]
}

/// `TrainerPicAndMoneyPointers`' pic, as tiles.
pub fn pic(class: u8) -> &'static [u8] {
    crate::gfx::TRAINER_PICS[class as usize - 1]
}

/// The class's base reward, BCD.
pub fn base_money(class: u8) -> [u8; 3] {
    crate::item::bcd3(TRAINER_BASE_MONEY[class as usize - 1])
}

/// `TrainerClassMoveChoiceModifications`: the AI's move-choice layers for a class.
pub fn move_choices(class: u8) -> &'static [u8] {
    TRAINER_CLASS_MOVE_CHOICE_MODIFICATIONS[class as usize - 1]
}

/// `LoneMoves`: for a gym leader, `(index of the party mon from 0, move)`, looked up by
/// `wLoneAttackNo` from 1.
pub fn lone_moves() -> [(u8, u8); 8] {
    LONE_MOVES
}

/// `TeamMoves`: `(trainer class, move)` for the Elite Four.
pub fn team_moves() -> &'static [(u8, u8)] {
    TEAM_MOVES
}

/// `TrainerAIPointers`: how many times a class's AI may act for each mon, and the routine's label.
pub fn ai_pointer(class: u8) -> (u8, &'static str) {
    TRAINER_AI_POINTERS[class as usize - 1]
}

#[cfg(test)]
mod tests {
    use crate::species::PokemonSpecies;
    use super::*;

    #[test]
    fn the_first_youngster_on_route_3_has_a_rattata_and_an_ekans() {
        let youngster = parties(1);
        let [(l1, a), (l2, b)] = youngster[0].mons[..] else { panic!("{:?}", youngster[0]) };
        assert_eq!((l1, l2), (11, 11));
        assert_eq!([PokemonSpecies::from_repr(a), PokemonSpecies::from_repr(b)], [Some(PokemonSpecies::Rattata), Some(PokemonSpecies::Ekans)]);
    }

    /// The last class's parties end with its label's, not at whatever follows them in the ROM.
    #[test]
    fn lance_has_one_party() {
        assert_eq!(parties(NUM_TRAINERS).len(), 1);
    }

    #[test]
    fn every_party_is_one_to_six_real_pokemon() {
        for class in 1..=NUM_TRAINERS {
            for (index, party) in parties(class).iter().enumerate() {
                assert!((1..=6).contains(&party.mons.len()), "class {class} party {index}: {party:?}");
                for &(level, species) in party.mons {
                    assert!((1..=100).contains(&level) && PokemonSpecies::from_repr(species).is_some(),
                        "class {class} party {index}: {party:?}");
                }
            }
        }
    }

    #[test]
    fn brock_s_onix_bides_and_lorelei_s_fifth_mon_blizzards() {
        use crate::move_name::PokemonMoveName as M;
        assert_eq!(lone_moves()[0], (1, M::Bide as u8));
        assert_eq!(team_moves(), [(44, M::Blizzard as u8), (33, M::Fissure as u8), (46, M::Toxic as u8), (47, M::Barrier as u8)]);
        assert_eq!(ai_pointer(34), (5, "BrockAI"));
    }

    #[test]
    fn a_youngster_pays_fifteen_hundred_a_level_and_a_sailor_chooses_with_layers_one_and_three() {
        assert_eq!(base_money(1), [0x00, 0x15, 0x00]);
        assert_eq!(move_choices(4), [1, 3]);
    }
}
