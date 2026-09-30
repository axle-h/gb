use crate::move_name::PokemonMoveName;
use crate::pokemon::PokemonType;
use crate::species::PokemonSpecies;

/// One `BaseStats` entry, as `GetMonHeader` copies it to `wMonHeader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStats {
    pub dex: u8,
    /// HP, attack, defense, speed, special: `CalcStat`'s order.
    pub stats: [u8; 5],
    pub types: [u8; 2],
    pub catch_rate: u8,
    pub base_exp: u8,
    /// The first four moves known, `NO_MOVE` as 0.
    pub level_1_moves: [u8; 4],
    pub growth_rate: u8,
    /// The moves its machines can teach it.
    pub tm_hm: &'static [u8],
}

impl BaseStats {
    pub fn of(species: PokemonSpecies) -> Self {
        let e = crate::tables::BASE_STATS[species.metadata().pokedex_number as usize - 1];
        Self {
            dex: e.dex,
            stats: e.stats,
            types: e.types,
            catch_rate: e.catch_rate,
            base_exp: e.base_exp,
            level_1_moves: e.level_1_moves,
            growth_rate: e.growth_rate,
            tm_hm: e.tm_hm,
        }
    }

    pub fn types(&self) -> [Option<PokemonType>; 2] {
        self.types.map(PokemonType::from_repr)
    }

    pub fn level_1_moves(&self) -> impl Iterator<Item = PokemonMoveName> + '_ {
        self.level_1_moves.iter().filter_map(|&m| PokemonMoveName::from_repr(m))
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;
    use super::*;

    /// The hand-written species table and the cartridge's agree.
    #[test]
    fn every_species_matches_the_hand_written_table() {
        for species in PokemonSpecies::iter() {
            let (base, meta) = (BaseStats::of(species), species.metadata());
            assert_eq!(base.dex, meta.pokedex_number, "{species}");
            let b = meta.base_stats;
            assert_eq!(base.stats.map(u16::from), [b.hp, b.attack, b.defense, b.speed, b.special], "{species}");
            let second = meta.type2.unwrap_or(meta.type1);
            assert_eq!(base.types(), [Some(meta.type1), Some(second)], "{species}");
        }
    }
}
