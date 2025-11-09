use std::fmt::Display;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use crate::mmu::MMU;
use crate::pokemon::strings::PokemonString;
use crate::ram::{RAM, ROM};

#[derive(Debug, Copy, Clone, PartialEq, Eq, strum_macros::Display)]
pub enum DmgBank {
    #[strum(serialize = "ROM:{bank:02X}")]
    ROM { bank: u8 },
    VRAM,
    #[strum(serialize = "SRAM:{bank:02X}")]
    SRAM { bank: u8 },
    WRAM,
    HRAM,
}

impl DmgBank {
    pub const fn id(&self) -> u8 {
        match self {
            DmgBank::ROM { bank } => *bank,
            DmgBank::SRAM { bank } => *bank,
            _ => 0
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmgPointer {
    pub bank: DmgBank,
    pub address: u16
}

impl Add<u16> for DmgPointer {
    type Output = DmgPointer;

    fn add(self, rhs: u16) -> Self::Output {
        Self::Output {
            bank: self.bank,
            address: self.address.overflowing_add(rhs).0,
        }
    }
}

impl AddAssign<u16> for DmgPointer {
    fn add_assign(&mut self, rhs: u16) {
        self.address = self.address.overflowing_add(rhs).0;
    }
}

impl Sub<u16> for DmgPointer {
    type Output = DmgPointer;

    fn sub(self, rhs: u16) -> Self::Output {
        Self::Output {
            bank: self.bank,
            address: self.address.overflowing_sub(rhs).0,
        }
    }
}

impl SubAssign<u16> for DmgPointer {
    fn sub_assign(&mut self, rhs: u16) {
        self.address = self.address.overflowing_sub(rhs).0;
    }
}

impl Display for DmgPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // print in rgblink format e.g. '00:cd2e'
        write!(f, "{:02}:{:04X}", self.bank.id(), self.address)
    }
}

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

    fn write_pointer(&mut self, pointer: &DmgPointer, value: u8) -> Result<(), String>;

    fn write_pointer_u16_le(&mut self, pointer: &DmgPointer, value: u16) -> Result<(), String>;

    fn write_pointer_u16_be(&mut self, pointer: &DmgPointer, value: u16) -> Result<(), String>;

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

    fn write_pointer(&mut self, pointer: &DmgPointer, value: u8) -> Result<(), String> {
        match pointer.bank {
            DmgBank::ROM { bank } => {
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

    fn write_pointer_u16_le(&mut self, pointer: &DmgPointer, value: u16) -> Result<(), String> {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                Err("ROM is not writable".to_string())
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.write_u16_le(pointer.address, value);
                Ok(())
            }
            DmgBank::SRAM { .. } => {
                Err("SRAM banking not implemented".to_string())
            }
        }
    }

    fn write_pointer_u16_be(&mut self, pointer: &DmgPointer, value: u16) -> Result<(), String> {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                Err("ROM is not writable".to_string())
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.write_u16_be(pointer.address, value);
                Ok(())
            }
            DmgBank::SRAM { .. } => {
                Err("SRAM banking not implemented".to_string())
            }
        }
    }

    fn write_pointer_slice(&mut self, pointer: &DmgPointer, value: &[u8]) -> Result<(), String> {
        match pointer.bank {
            DmgBank::ROM { bank } => {
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
                    let byte = self.read(pointer.address + i);
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

    #[test]
    fn test_parse_sym_file() {
        // Test ROM entry
        let calc_checksum = pokered_symbols::CalcCheckSum;
        assert_eq!(calc_checksum.bank, DmgBank::ROM { bank: 0x1c });
        assert_eq!(calc_checksum.address, 0x7856);

        // Test WRAM entry
        let enemy_mon = pokered_symbols::wEnemyMonUnmodifiedSpecial;
        assert_eq!(enemy_mon.bank, DmgBank::WRAM);
        assert_eq!(enemy_mon.address, 0xcd2c);

        // Test SRAM entry
        let cur_box = pokered_symbols::sCurBoxData;
        assert_eq!(cur_box.bank, DmgBank::SRAM { bank: 0x01 });
        assert_eq!(cur_box.address, 0xb0c0);

        // Test VRAM entry
        let tileset = pokered_symbols::vTileset;
        assert_eq!(tileset.bank, DmgBank::VRAM);
        assert_eq!(tileset.address, 0x9000);

        // Test HRAM entry
        let slide_amount = pokered_symbols::hSlideAmount;
        assert_eq!(slide_amount.bank, DmgBank::HRAM);
        assert_eq!(slide_amount.address, 0xff8b);

        // Test constants
        assert_eq!(pokered_symbols::ROUTE6GATE_GUARD, 0x01);
        assert_eq!(pokered_symbols::ROUTE6_COOLTRAINER_M1, 0x01);
        assert_eq!(pokered_symbols::ROUTE7GATE_GUARD, 0x01);
        assert_eq!(pokered_symbols::PEWTERPOKECENTER_GENTLEMAN, 0x02);
        assert_eq!(pokered_symbols::FUCHSIAPOKECENTER_ROCKER, 0x02);
        assert_eq!(pokered_symbols::GAMECORNERPRIZEROOM_GAMBLER, 0x02);
    }
}
