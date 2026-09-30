//! `EvosMovesPointerTable`: how each species evolves and what it learns by levelling up.

use serde::{Deserialize, Serialize};
use crate::item::ItemId;
use crate::move_name::PokemonMoveName;
use crate::species::PokemonSpecies;
use crate::tables::{Evolution as Row, EVOS_MOVES};

/// One evolution entry, in the order `EvolutionAfterBattle` tries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Evolution {
    Level { level: u8, into: PokemonSpecies },
    Item { item: ItemId, min_level: u8, into: PokemonSpecies },
    Trade { min_level: u8, into: PokemonSpecies },
}

impl Evolution {
    pub fn into(self) -> PokemonSpecies {
        match self {
            Self::Level { into, .. } | Self::Item { into, .. } | Self::Trade { into, .. } => into,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvosMoves {
    pub evolutions: Vec<Evolution>,
    /// `(level, move)` in increasing level, as `WriteMonMoves` assumes.
    pub learnset: Vec<(u8, PokemonMoveName)>,
}

impl EvosMoves {
    pub fn of(species: PokemonSpecies) -> Self {
        let entry = EVOS_MOVES[species as usize - 1];
        let species = |id: u8| PokemonSpecies::from_repr(id).unwrap_or_else(|| panic!("species {id:#04x}"));
        let evolutions = entry.evolutions.iter().map(|&row| match row {
            Row::Level { level, into } => Evolution::Level { level, into: species(into) },
            Row::Item { item, min_level, into } => {
                Evolution::Item { item: ItemId::from_repr(item).expect("an evolution item"), min_level, into: species(into) }
            }
            Row::Trade { min_level, into } => Evolution::Trade { min_level, into: species(into) },
        }).collect();
        let learnset = entry.learnset.iter()
            .map(|&(level, id)| (level, PokemonMoveName::from_repr(id).expect("a learnset move")))
            .collect();
        Self { evolutions, learnset }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;
    use super::*;
    use PokemonSpecies::*;

    #[test]
    fn every_species_decodes_with_a_sorted_learnset() {
        for species in PokemonSpecies::iter() {
            let entry = EvosMoves::of(species);
            assert!(entry.learnset.is_sorted_by_key(|&(level, _)| level), "{species}");
            assert!(entry.evolutions.len() <= 3, "{species}");
        }
    }

    #[test]
    fn the_three_kinds_of_evolution() {
        assert_eq!(EvosMoves::of(Bulbasaur).evolutions, [Evolution::Level { level: 16, into: Ivysaur }]);
        assert_eq!(EvosMoves::of(Kadabra).evolutions, [Evolution::Trade { min_level: 1, into: Alakazam }]);
        assert_eq!(EvosMoves::of(Eevee).evolutions, [
            Evolution::Item { item: ItemId::FireStone, min_level: 1, into: Flareon },
            Evolution::Item { item: ItemId::ThunderStone, min_level: 1, into: Jolteon },
            Evolution::Item { item: ItemId::WaterStone, min_level: 1, into: Vaporeon },
        ]);
        assert!(EvosMoves::of(Mew).evolutions.is_empty());
    }

    #[test]
    fn a_learnset_is_level_and_move() {
        let learnset = EvosMoves::of(Bulbasaur).learnset;
        assert_eq!(learnset.first(), Some(&(7, PokemonMoveName::LeechSeed)));
        assert_eq!(learnset.last(), Some(&(48, PokemonMoveName::Solarbeam)));
    }
}
