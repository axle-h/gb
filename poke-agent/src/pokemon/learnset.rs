//! Which Pokémon a TM or HM will actually teach, read out of the cartridge's own base-stats table.
//!
//! The game answers a teach it will not allow with `MonCannotLearnMachineMoveText` and then
//! `jr .chooseMon` (`engine/items/item_effects.asm`) — straight back to the party menu with the
//! cursor where it was. That is the closed-loop-under-A shape the whole agent keeps meeting, and
//! `TeachingMove`'s only exit is "the mon knows the move", so an incompatible target wedges it until
//! `DRIVER_ESCAPE_SILENCE` gives up a minute later and the policy asks for the same thing again.
//! The answer is not to detect the message but to never ask: the compatibility is a bitfield sitting
//! in ROM beside the base stats, and it is knowable before a single button is pressed.

use crate::pokemon::item::ItemId;
use crate::pokemon::mon_gfx::base_stats_entry;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::pokered_symbols;
use crate::pokemon::GameState;

/// `wMonHLearnset` — where the 7-byte TM/HM flag array sits within a 28-byte base-stats entry.
/// Everything before it is fixed-width (`wMonHeader` in `ram/wram.asm`): the dex number, five base
/// stats, two types, catch rate, base EXP, sprite dimensions, two pic pointers, four starting moves
/// and the growth rate.
const BASE_LEARNSET: usize = 20;

/// The flag `CanLearnTM` tests for `item`, or `None` if the item is not a machine at all.
///
/// ⚠️ **`\1_TMNUM` is not the item id and the two run in opposite directions.** `add_hm` numbers the
/// HMs **51-55** and `add_tm` numbers the TMs **1-50** (`constants/item_constants.asm`), while the
/// item ids put the five HMs at `$C4-$C8` *below* the fifty TMs at `$C9-$FA`. The flag index is
/// `TMNUM - 1`, which is also the row `TMToMove` reads out of `TechnicalMachines`.
pub const fn tm_hm_flag(item: ItemId) -> Option<usize> {
    let id = item as u8;
    match id {
        0xC4..=0xC8 => Some(50 + (id - 0xC4) as usize), // HM01-HM05 → TMNUM 51-55
        0xC9..=0xFA => Some((id - 0xC9) as usize),      // TM01-TM50 → TMNUM 1-50
        _ => None,
    }
}

/// Whether the game will let `species` learn the machine `item`.
///
/// `true` for anything that is not a TM or an HM, because "can this Pokémon learn a Rare Candy" is
/// not a question the cartridge asks — `UseTMHM` is only reached for a machine, and every other
/// caller here is a stone or a consumable that opens the same menu chain.
pub fn can_learn(species: PokemonSpecies, item: ItemId) -> bool {
    let Some(flag) = tm_hm_flag(item) else { return true };
    // `FlagAction` (`engine/flag_action.asm`): byte `c >> 3`, bit `c & 7`, least significant first.
    base_stats_entry(species)[BASE_LEARNSET + flag / 8] & (1 << (flag % 8)) != 0
}

/// The move a TM or HM teaches, from the cartridge's own `TechnicalMachines` table.
///
/// Read rather than transcribed because a second copy of a 55-row table is a second place to be
/// wrong, and because the partial one that already exists (`policy::hm_move`) names only the seven
/// machines the scripted policy uses. What the model needs said is the **move**: `Hm01Cut` is the
/// bag's name for it and "Cut" is the game's.
pub fn machine_move(item: ItemId) -> Option<PokemonMoveName> {
    let flag = tm_hm_flag(item)?;
    PokemonMoveName::from_repr(rom_slice(pokered_symbols::TechnicalMachines)[flag])
}

/// What to say when a teach is aimed at a Pokémon the game will refuse.
///
/// ⚠️ **It names the alternative rather than the refusal.** The driver used to report this as
/// "got no answer from the game for 60s; starting over", which reads as a malfunction and gives a
/// model nothing to do differently: it re-issued the identical teach. The decision actually on the
/// table is *which* party member takes the machine, so that is what the sentence answers, and when
/// the answer is "none of them" it says that outright rather than leaving it to be inferred from an
/// empty list.
///
/// ⚠️ **No em dashes**: this goes to the model and onto the page.
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
            "{subject} cannot learn {taught}, and nor can anything else in the party. Every machine              works on a fixed list of Pokémon and the game refuses the rest, so teaching this one              needs a party member that is on that list; nothing you own is. Catching or swapping in              a Pokémon that can learn it is the only way past."),
        _ => format!("{subject} cannot learn {taught}. In the party, {} can.", takers.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case the deployed run wedged on, and its neighbour in the same party.
    ///
    /// ⚠️ **Read from the ROM rather than transcribed**, which is the point: a table of who learns
    /// what is 151 × 55 bits of exactly the sort of thing that gets a row wrong and is never noticed
    /// until an agent spends an hour in a party menu.
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

    /// TM01 is the low flag and HM05 the high one, so an off-by-one at either end of the array shows
    /// up here rather than as a plausible answer about some Pokémon in the middle.
    #[test]
    fn the_flag_runs_from_tm01_to_hm05() {
        assert_eq!(tm_hm_flag(ItemId::Hm01Cut), Some(50));
        assert_eq!(tm_hm_flag(ItemId::Hm05Flash), Some(54));
        assert_eq!(tm_hm_flag(ItemId::Tm06Toxic), Some(5));
        assert_eq!(tm_hm_flag(ItemId::Tm45ThunderWave), Some(44));
        assert_eq!(tm_hm_flag(ItemId::RareCandy), None);

        // Mega Punch is TM01 and Substitute TM50, at the two ends of the array. Both are Mewtwo's,
        // which is the one entry in `BaseStats` reachable only through Mew's special case not firing.
        assert!(can_learn(PokemonSpecies::Mewtwo, ItemId::Tm45ThunderWave));
        assert!(!can_learn(PokemonSpecies::Caterpie, ItemId::Tm45ThunderWave), "Caterpie learns nothing");
    }

    /// Mew is not in `BaseStats` (see [`base_stats_entry`]) and learns **every** machine, so a
    /// lookup that forgot it would read Mewtwo's entry and answer plausibly rather than failing.
    #[test]
    fn mew_learns_every_machine() {
        for item in [ItemId::Hm01Cut, ItemId::Hm02Fly, ItemId::Hm03Surf, ItemId::Hm04Strength,
                     ItemId::Hm05Flash, ItemId::Tm06Toxic, ItemId::Tm34Bide] {
            assert!(can_learn(PokemonSpecies::Mew, item), "Mew learns {item}");
        }
    }

    /// Not a machine, so not a question — the stones and the Rare Candy ride the same menu chain and
    /// must not be refused by a check written for TMs.
    #[test]
    fn a_stone_is_not_a_machine() {
        assert!(can_learn(PokemonSpecies::Pidgey, ItemId::RareCandy));
        assert!(can_learn(PokemonSpecies::Eevee, ItemId::WaterStone));
    }
}
