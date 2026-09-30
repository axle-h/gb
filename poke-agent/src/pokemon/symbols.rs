use gb::mmu::MMU;
use gb::ram::{RAM, ROM};
use crate::pokemon::strings::PokemonString;

pub use poke_core::symbols::{pokered_events, pokered_toggles, DmgBank, DmgPointer};

include!(concat!(env!("OUT_DIR"), "/pokered_symbols.rs"));

/// Trait for reading memory using pokered symbol file pointers
pub trait DmgPointerRead {
    fn read_pointer(&self, pointer: &DmgPointer) -> u8;
    fn read_pointer_u16_le(&self, pointer: &DmgPointer) -> u16;
    fn read_pointer_u16_be(&self, pointer: &DmgPointer) -> u16;

    fn read_pointer_u24_be(&self, pointer: &DmgPointer) -> u32 {
        let mut pointer = *pointer;
        let high = self.read_pointer_u16_be(&pointer) as u32;
        pointer += 2;
        let low = self.read_pointer(&pointer) as u32;
        (high << 8) | low
    }

    fn read_pointer_vec(&self, pointer: &DmgPointer, length: usize) -> Vec<u8>;

    fn rom_data_from_rom_pointer(&self, pointer: &DmgPointer, length: usize) -> &[u8];

    fn write_pointer(&mut self, pointer: &DmgPointer, value: u8) -> Result<(), String>;

    fn write_pointer_slice(&mut self, pointer: &DmgPointer, value: &[u8]) -> Result<(), String>;

    fn read_pointer_pokemon_string(&self, pointer: &DmgPointer) -> PokemonString;

    fn write_pointer_pokemon_string(&mut self, pointer: &DmgPointer, string: &PokemonString) -> Result<(), String>;
}

impl DmgPointerRead for MMU {
    fn read_pointer(&self, pointer: &DmgPointer) -> u8 {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                self.rom_data_from_pointer(bank as usize, pointer.address, Some(1))[0]
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.read(pointer.address)
            }
            DmgBank::SRAM { .. } => {
                panic!("SRAM banking not implemented")
            }
        }
    }

    fn read_pointer_u16_le(&self, pointer: &DmgPointer) -> u16 {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                let bytes = self.rom_data_from_pointer(bank as usize, pointer.address, Some(2));
                u16::from_le_bytes([bytes[0], bytes[1]])
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                // TODO presume all of the bytes are in the same bank so we can just read them
                self.read_u16_le(pointer.address)
            }
            DmgBank::SRAM { .. } => {
                panic!("SRAM banking not implemented")
            }
        }
    }

    fn read_pointer_u16_be(&self, pointer: &DmgPointer) -> u16 {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                let bytes = self.rom_data_from_pointer(bank as usize, pointer.address, Some(2));
                u16::from_be_bytes([bytes[0], bytes[1]])
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.read_u16_be(pointer.address)
            }
            DmgBank::SRAM { .. } => {
                panic!("SRAM banking not implemented")
            }
        }
    }

    fn read_pointer_vec(&self, pointer: &DmgPointer, length: usize) -> Vec<u8> {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                self.rom_data_from_pointer(bank as usize, pointer.address, Some(length)).to_vec()
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.read_slice(pointer.address, length)
            }
            DmgBank::SRAM { .. } => {
                panic!("SRAM banking not implemented")
            }
        }
    }

    fn rom_data_from_rom_pointer(&self, pointer: &DmgPointer, length: usize) -> &[u8] {
        let DmgBank::ROM { bank } = pointer.bank else {
            panic!("Pointer {pointer} is not a ROM pointer")
        };
        self.rom_data_from_pointer(bank as usize, pointer.address, length)
    }

    fn write_pointer(&mut self, pointer: &DmgPointer, value: u8) -> Result<(), String> {
        match pointer.bank {
            DmgBank::ROM { bank: _ } => {
                Err("ROM is not writable".to_string())
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.write(pointer.address, value);
                Ok(())
            }
            DmgBank::SRAM { .. } => {
                Err("SRAM banking not implemented".to_string())
            }
        }
    }

    fn write_pointer_slice(&mut self, pointer: &DmgPointer, value: &[u8]) -> Result<(), String> {
        match pointer.bank {
            DmgBank::ROM { bank: _ } => {
                Err("ROM is not writable".to_string())
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.write_slice(pointer.address, value);
                Ok(())
            }
            DmgBank::SRAM { .. } => {
                Err("SRAM banking not implemented".to_string())
            }
        }
    }

    fn read_pointer_pokemon_string(&self, pointer: &DmgPointer) -> PokemonString {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                let slice = self.rom_data_from_pointer(bank as usize, pointer.address, None);
                let mut bytes = vec![];
                for &byte in slice.into_iter() {
                    bytes.push(byte);
                    if byte == PokemonString::TERMINATOR {
                        break;
                    }
                }
                PokemonString(bytes)
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                let mut bytes = vec![];
                for i in 0..u16::MAX {
                    // Wrapping, because a scan that starts near the top of the address space runs
                    // off it and the release build has always wrapped here.
                    let byte = self.read(pointer.address.wrapping_add(i));
                    bytes.push(byte);
                    if byte == PokemonString::TERMINATOR {
                        break;
                    }
                }
                PokemonString(bytes)
            }
            DmgBank::SRAM { .. } => {
                panic!("SRAM banking not implemented")
            }
        }
    }

    fn write_pointer_pokemon_string(&mut self, pointer: &DmgPointer, string: &PokemonString) -> Result<(), String> {
        match pointer.bank {
            DmgBank::ROM { .. } => {
                Err("ROM is not writable".to_string())
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                for (index, byte) in string.0.iter().enumerate() {
                    self.write(pointer.address + index as u16, *byte);
                }
                Ok(())
            }
            DmgBank::SRAM { .. } => {
                Err("SRAM banking not implemented".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A symbol's prefix names its memory, and the exported one-byte constants are values.
    #[test]
    fn symbols_are_read_into_their_memory() {
        assert_eq!(pokered_symbols::CalcCheckSum, DmgPointer { bank: DmgBank::ROM { bank: 0x1C }, address: 0x7856 });
        assert_eq!(pokered_symbols::wEnemyMonUnmodifiedSpecial, DmgPointer { bank: DmgBank::WRAM, address: 0xCD2C });
        assert_eq!(pokered_symbols::sCurBoxData, DmgPointer { bank: DmgBank::SRAM { bank: 0x01 }, address: 0xB0C0 });
        assert_eq!(pokered_symbols::vTileset, DmgPointer { bank: DmgBank::VRAM, address: 0x9000 });
        assert_eq!(pokered_symbols::hSlideAmount, DmgPointer { bank: DmgBank::HRAM, address: 0xFF8B });
        assert_eq!(pokered_local_labels::DisableLCD::wait, DmgPointer { bank: DmgBank::ROM { bank: 0 }, address: 0x006B });
        assert_eq!((pokered_symbols::ROUTE6GATE_GUARD, pokered_symbols::PEWTERPOKECENTER_GENTLEMAN), (0x01, 0x02));
    }
}
