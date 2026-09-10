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

    pub fn iter(&self) -> std::slice::Iter<'_, Pokemon> {
        self.0.iter()
    }

    pub fn get(&self, index: usize) -> Option<&Pokemon> {
        self.0.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Pokemon> {
        self.0.get_mut(index)
    }

    /// Move the member at `index` to the front (slot 0), shifting the rest down one. No-op if out of
    /// range or already at the front. Used to make a trained bench mon the battle lead.
    pub fn move_to_front(&mut self, index: usize) {
        if index != 0 && index < self.0.len() {
            let m = self.0.remove(index);
            self.0.insert(0, m);
        }
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

impl<'a> IntoIterator for &'a PokemonParty {
    type Item = &'a Pokemon;
    type IntoIter = std::slice::Iter<'a, Pokemon>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

