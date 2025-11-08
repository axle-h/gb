use std::ops::{Index, IndexMut};
use crate::pokemon::pokemon::Pokemon;
use crate::pokemon::encoding::PokemonBlockAddresses;

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct PokemonParty(Vec<Pokemon>);

impl PokemonParty {
    pub fn push(&mut self, pokemon: Pokemon) -> Result<(), String> {
        if self.0.len() >= PokemonBlockAddresses::PARTY_MAX as usize {
            Err("Party is full".to_string())
        } else {
            self.0.push(pokemon);
            Ok(())
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
    
    pub fn pokemon(&self) -> &[Pokemon] {
        &self.0
    }
}

impl Index<usize> for PokemonParty {
    type Output = Pokemon;

    fn index(&self, index: usize) -> &Self::Output {
        self.0.index(index)
    }
}

impl IndexMut<usize> for PokemonParty {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.0.index_mut(index)
    }
}

impl IntoIterator for PokemonParty {
    type Item = Pokemon;
    type IntoIter = std::vec::IntoIter<Pokemon>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}