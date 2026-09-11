//! Which Pokémon a TM or HM will teach, read out of the cartridge's base-stats table.

use crate::item::ItemId;
use crate::mon_gfx::base_stats_entry;
use crate::move_name::PokemonMoveName;
use crate::rom_gfx::rom_slice;
use crate::species::PokemonSpecies;
use crate::symbols::pokered_symbols;

/// Offset of the 7-byte TM/HM flag array in a base-stats entry (`wMonHLearnset`).
const BASE_LEARNSET: usize = 20;

/// The flag `CanLearnTM` tests for `item`, or `None` if the item is not a machine at all.
pub const fn tm_hm_flag(item: ItemId) -> Option<usize> {
    let id = item as u8;
    match id {
        0xC4..=0xC8 => Some(50 + (id - 0xC4) as usize), // HM01-HM05 → TMNUM 51-55
        0xC9..=0xFA => Some((id - 0xC9) as usize),      // TM01-TM50 → TMNUM 1-50
        _ => None,
    }
}

/// Whether the game will let `species` learn the machine `item`.
pub fn can_learn(species: PokemonSpecies, item: ItemId) -> bool {
    let Some(flag) = tm_hm_flag(item) else { return true };
    // `FlagAction`: byte `c >> 3`, bit `c & 7`, least significant first.
    base_stats_entry(species)[BASE_LEARNSET + flag / 8] & (1 << (flag % 8)) != 0
}

/// The move a TM or HM teaches, from the cartridge's own `TechnicalMachines` table.
pub fn machine_move(item: ItemId) -> Option<PokemonMoveName> {
    let flag = tm_hm_flag(item)?;
    PokemonMoveName::from_repr(rom_slice(pokered_symbols::TechnicalMachines)[flag])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_learnset_says_who_can_take_an_hm() {
        assert!(!can_learn(PokemonSpecies::Pidgey, ItemId::Hm01Cut), "Pidgey cannot learn Cut");
        assert!(can_learn(PokemonSpecies::Venusaur, ItemId::Hm01Cut), "Venusaur can");
        assert!(can_learn(PokemonSpecies::Pidgey, ItemId::Hm02Fly), "Pidgey is a Flyer");
        assert!(!can_learn(PokemonSpecies::Venusaur, ItemId::Hm02Fly), "Venusaur is not");
        assert!(can_learn(PokemonSpecies::Vaporeon, ItemId::Hm03Surf), "Vaporeon can learn Surf");
        assert!(!can_learn(PokemonSpecies::Pidgey, ItemId::Hm03Surf), "Pidgey cannot");
        assert!(can_learn(PokemonSpecies::Machop, ItemId::Hm04Strength), "Machop can learn Strength");
        assert!(!can_learn(PokemonSpecies::Gastly, ItemId::Hm04Strength), "Gastly cannot");
    }

    /// TM01 is the low flag and HM05 the high one.
    #[test]
    fn the_flag_runs_from_tm01_to_hm05() {
        assert_eq!(tm_hm_flag(ItemId::Hm01Cut), Some(50));
        assert_eq!(tm_hm_flag(ItemId::Hm05Flash), Some(54));
        assert_eq!(tm_hm_flag(ItemId::Tm06Toxic), Some(5));
        assert_eq!(tm_hm_flag(ItemId::Tm45ThunderWave), Some(44));
        assert_eq!(tm_hm_flag(ItemId::RareCandy), None);

        assert!(can_learn(PokemonSpecies::Mewtwo, ItemId::Tm45ThunderWave));
        assert!(!can_learn(PokemonSpecies::Caterpie, ItemId::Tm45ThunderWave), "Caterpie learns nothing");
    }

    /// Mew, outside `BaseStats`, learns every machine rather than reading Mewtwo's entry.
    #[test]
    fn mew_learns_every_machine() {
        for item in [ItemId::Hm01Cut, ItemId::Hm02Fly, ItemId::Hm03Surf, ItemId::Hm04Strength,
                     ItemId::Hm05Flash, ItemId::Tm06Toxic, ItemId::Tm34Bide] {
            assert!(can_learn(PokemonSpecies::Mew, item), "Mew learns {item}");
        }
    }

    /// A stone or the Rare Candy is not refused by the machine check.
    #[test]
    fn a_stone_is_not_a_machine() {
        assert!(can_learn(PokemonSpecies::Pidgey, ItemId::RareCandy));
        assert!(can_learn(PokemonSpecies::Eevee, ItemId::WaterStone));
    }
}
