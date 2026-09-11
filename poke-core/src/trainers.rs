use crate::rom_gfx::rom_slice;
use crate::symbols::{pokered_symbols, DmgBank, DmgPointer};

pub const NUM_TRAINERS: u8 = 47;

fn in_bank(address: u16) -> &'static [u8] {
    let DmgBank::ROM { bank } = pokered_symbols::TrainerDataPointers.bank else { unreachable!() };
    rom_slice(DmgPointer { bank: DmgBank::ROM { bank }, address })
}

/// The parties of trainer class `class` (1-based, as `wTrainerClass`), in order, each as
/// `(level, internal species id)`. A party is `level, species…, 0`, or `$FF, level, species…, 0`
/// when levels differ.
pub fn parties(class: u8) -> Vec<Vec<(u8, u8)>> {
    assert!((1..=NUM_TRAINERS).contains(&class), "trainer class {class}");
    let pointers = rom_slice(pokered_symbols::TrainerDataPointers);
    let at = |class: u8| u16::from_le_bytes([pointers[(class as usize - 1) * 2], pointers[(class as usize - 1) * 2 + 1]]);
    let start = at(class);
    let data = in_bank(start);
    let end = if class < NUM_TRAINERS { (at(class + 1) - start) as usize } else { data.len() };
    let mut parties = vec![];
    let mut rest = &data[..end];
    while let Some(&first) = rest.first() {
        let body_end = rest.iter().skip(1).position(|&b| b == 0).map(|p| p + 1).unwrap_or(rest.len());
        let body = &rest[1..body_end];
        parties.push(if first == 0xFF {
            body.chunks_exact(2).map(|pair| (pair[0], pair[1])).collect()
        } else {
            body.iter().map(|&species| (first, species)).collect()
        });
        rest = &rest[(body_end + 1).min(rest.len())..];
        if class == NUM_TRAINERS && parties.len() == LAST_CLASS_PARTIES {
            break;
        }
    }
    parties
}

/// `Lance`, the last class, has no class after it to bound its data.
const LAST_CLASS_PARTIES: usize = 1;

/// `TrainerPicAndMoneyPointers`: the pic's address and the base reward, BCD.
pub fn pic_and_money(class: u8) -> (u16, [u8; 3]) {
    let row = &rom_slice(pokered_symbols::TrainerPicAndMoneyPointers)[(class as usize - 1) * 5..][..5];
    (u16::from_le_bytes([row[0], row[1]]), [row[2], row[3], row[4]])
}

/// `TrainerClassMoveChoiceModifications`: the AI's move-choice layers for a class.
pub fn move_choices(class: u8) -> Vec<u8> {
    rom_slice(pokered_symbols::TrainerClassMoveChoiceModifications)
        .split(|&b| b == 0)
        .nth(class as usize - 1)
        .expect("every class has an entry")
        .to_vec()
}

#[cfg(test)]
mod tests {
    use crate::species::PokemonSpecies;
    use super::*;

    #[test]
    fn the_first_youngster_on_route_3_has_a_rattata_and_an_ekans() {
        let youngster = parties(1);
        let [(l1, a), (l2, b)] = youngster[0][..] else { panic!("{:?}", youngster[0]) };
        assert_eq!((l1, l2), (11, 11));
        assert_eq!([PokemonSpecies::from_repr(a), PokemonSpecies::from_repr(b)], [Some(PokemonSpecies::Rattata), Some(PokemonSpecies::Ekans)]);
    }

    #[test]
    fn every_party_is_one_to_six_real_pokemon() {
        for class in 1..=NUM_TRAINERS {
            for (index, party) in parties(class).iter().enumerate() {
                assert!((1..=6).contains(&party.len()), "class {class} party {index}: {party:?}");
                for &(level, species) in party {
                    assert!((1..=100).contains(&level) && PokemonSpecies::from_repr(species).is_some(),
                        "class {class} party {index}: {party:?}");
                }
            }
        }
    }

    #[test]
    fn a_youngster_pays_fifteen_hundred_a_level_and_a_sailor_chooses_with_layers_one_and_three() {
        assert_eq!(pic_and_money(1).1, [0x00, 0x15, 0x00]);
        assert_eq!(move_choices(4), [1, 3]);
    }
}
