//! Which Pokémon a TM or HM will teach, read out of the cartridge's base-stats table.

use crate::pokemon::item::ItemId;
use crate::pokemon::mon_gfx::base_stats_entry;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::pokered_symbols;
use crate::pokemon::GameState;

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

/// What to say when a teach is aimed at a Pokémon the game will refuse.
pub fn teach_refusal(state: &GameState, item: ItemId, slot: u8) -> String {
    let name = |mon: &crate::pokemon::pokemon::Pokemon| {
        let nickname = mon.nickname.to_default_string();
        match nickname.eq_ignore_ascii_case(&mon.species.to_string()) {
            true => nickname,
            false => format!("{nickname} the {}", mon.species),
        }
    };
    let subject = match state.pokemon.get(slot as usize) {
        Some(mon) => format!("{} in slot {slot}", name(mon)),
        None => format!("Slot {slot}"),
    };
    let taught = match machine_move(item) {
        Some(mv) => format!("{mv} ({item})"),
        None => item.to_string(),
    };
    let takers: Vec<String> = state.pokemon.iter().enumerate()
        .filter(|(_, mon)| can_learn(mon.species, item))
        .map(|(i, mon)| format!("slot {i} {}", name(mon)))
        .collect();
    match takers.as_slice() {
        [] => format!(
            "{subject} cannot learn {taught}, and nor can anything else in the party. Every machine \
             works on a fixed list of Pokémon and the game refuses the rest, so teaching this one \
             needs a party member that is on that list; nothing you own is. Catching or swapping in \
             a Pokémon that can learn it is the only way past."),
        _ => format!("{subject} cannot learn {taught}. In the party, {} can.", takers.join(", ")),
    }
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

    #[test]
    fn a_refusal_with_no_taker_reads_as_one_sentence() {
        let refusal = teach_refusal(&GameState::default(), ItemId::Hm03Surf, 0);
        assert!(refusal.contains("nor can anything else in the party"), "the no-taker arm: {refusal}");
        assert!(!refusal.contains('—'), "no em dashes in what the agent writes: {refusal}");
        assert!(!refusal.contains("  "), "a `\\` was eaten out of a continued literal: {refusal}");
    }

    /// A stone or the Rare Candy is not refused by the machine check.
    #[test]
    fn a_stone_is_not_a_machine() {
        assert!(can_learn(PokemonSpecies::Pidgey, ItemId::RareCandy));
        assert!(can_learn(PokemonSpecies::Eevee, ItemId::WaterStone));
    }
}
