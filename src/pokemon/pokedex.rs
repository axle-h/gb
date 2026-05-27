use std::collections::BTreeMap;
use strum::IntoEnumIterator;
use crate::mmu::MMU;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_symbols, DmgPointer};
use crate::ram::ROM;

#[derive(Debug, Clone, Default)]
pub struct Pokedex {
    bytes: [u8; Self::BYTES_LENGTH],
}

impl Pokedex {
    pub const BYTES_LENGTH: usize = 19;

    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, String> {
        Ok(Self { bytes: bytes.try_into().map_err(|_| "invalid bytes length")? })
    }

    pub fn contains(&self, species: &PokemonSpecies) -> bool {
        let bit = species.metadata().pokedex_number - 1;
        let flag_byte = bit / 8;
        let flag_bit = bit % 8;
        self.bytes[flag_byte as usize] & (1 << flag_bit) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.iter().all(|byte| *byte == 0)
    }

    pub fn species(&self) -> Vec<PokemonSpecies> {
        if self.is_empty() {
            return Vec::new();
        }

        let pokedex_to_species: BTreeMap<u8, PokemonSpecies> = PokemonSpecies::iter()
            .map(|species| (species.metadata().pokedex_number, species))
            .collect();

        let mut result = vec!();
        for byte_id in 0..Self::BYTES_LENGTH {
            let byte = self.bytes[byte_id];

            if byte == 0 {
                continue;
            }
            for bit in 0..8 {
                if byte & (1 << bit) != 0 {
                    let pokedex_number = byte_id as u8 * 8 + bit + 1;
                    let species = pokedex_to_species.get(&pokedex_number).unwrap();
                    result.push(*species);
                }
            }
        }
        result
    }

}


pub trait PokedexReader {
    fn read_has_pokedex(&self) -> bool;
    fn read_pokedex(&self, base_pointer: &DmgPointer) -> Result<Pokedex, String>;
}

impl PokedexReader for MMU {
    fn read_has_pokedex(&self) -> bool {
        // EVENT_GOT_POKEDEX = event bit 37 (counted from pokedex_constants.asm).
        // byte = 37 / 8 = 4,  bit = 37 % 8 = 5,  mask = 0x20
        self.read(pokered_symbols::wEventFlags.address + 4) & 0x20 != 0
    }

    fn read_pokedex(&self, base_pointer: &DmgPointer) -> Result<Pokedex, String> {
        Pokedex::try_from_slice(self.read_wram_slice(base_pointer.address, Pokedex::BYTES_LENGTH)?)
    }
}