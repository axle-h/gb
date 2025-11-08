use std::fmt::Display;
use regex::Regex;
use std::collections::HashMap;
use once_cell::sync::Lazy;
use crate::mmu::MMU;

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
    pub fn id(&self) -> u8 {
        match self {
            DmgBank::ROM { bank } => *bank,
            DmgBank::SRAM { bank } => *bank,
            _ => 0
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct NamedDmgPointer {
    name: &'static str,
    bank: DmgBank,
    address: u16
}

impl NamedDmgPointer {
    pub fn flat_address(&self) -> u16 {
        assert!(matches!(self.bank, DmgBank::VRAM | DmgBank::HRAM | DmgBank::WRAM), "a pointer to {} cannot be flattened", self.bank);
        self.address
    }
}

impl Display for NamedDmgPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // print in rgblink format e.g. '00:cd2e wEnemyMonAttackMod'
        write!(f, "{:02}:{:04X} {}", self.bank.id(), self.address, self.name)
    }
}


static POKEMON_MEMORY_MAP: Lazy<PokemonMemoryMap> = Lazy::new(PokemonMemoryMap::new);

pub struct PokemonMemoryMap {
    pointers: HashMap<&'static str, NamedDmgPointer>,
    constants: HashMap<String, u8>,
}

impl PokemonMemoryMap {
    pub fn default() -> &'static PokemonMemoryMap {
        &POKEMON_MEMORY_MAP
    }

    fn new() -> Self {
        let raw = include_bytes!("../../pokered/pokered.sym");
        let mut pointers: HashMap<&'static str, NamedDmgPointer> = HashMap::new();
        let mut constants = HashMap::new();
        let entry_regex = Regex::new(r"^([0-9a-fA-F]{2}):([0-9a-fA-F]{4})\s+(\S+)$").unwrap();
        let const_regex = Regex::new(r"^([0-9a-fA-F]{2})\s+(\S+)$").unwrap();

        for line in raw.split(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(line).trim().to_string();
            if line.is_empty() {
                continue;
            }

            // Try parsing as bank:address entry
            if let Some(caps) = entry_regex.captures(&line) {
                let bank_id = u8::from_str_radix(&caps[1], 16).unwrap();
                let address = u16::from_str_radix(&caps[2], 16).unwrap();
                let name = caps[3].to_string();

                if let Some(bank) = Self::infer_bank(&name, bank_id, address) {
                    let name_static = Box::leak(name.into_boxed_str());
                    pointers.insert(name_static, NamedDmgPointer {
                        name: name_static,
                        bank,
                        address,
                    });
                }
            }
            // Try parsing as constant
            else if let Some(caps) = const_regex.captures(&line) {
                let value = u8::from_str_radix(&caps[1], 16).unwrap();
                let name = caps[2].to_string();
                constants.insert(name, value);
            }
        }

        Self { pointers, constants }
    }

    pub fn pointer(name: &str) -> &NamedDmgPointer {
        POKEMON_MEMORY_MAP.get_pointer(name).expect("Pointer not found")
    }

    pub fn address(name: &str) -> u16 {
        Self::pointer(name).flat_address()
    }

    pub fn constant(name: &str) -> u8 {
        POKEMON_MEMORY_MAP.get_constant(name).expect("Constant not found")
    }

    pub fn get_pointer(&self, name: &str) -> Option<&NamedDmgPointer> {
        self.pointers.get(name)
    }

    pub fn get_constant(&self, name: &str) -> Option<u8> {
        self.constants.get(name).copied()
    }

    fn infer_bank(name: &str, bank_id: u8, address: u16) -> Option<DmgBank> {
        let first_char = name.chars().next()?;

        match first_char {
            'w' => {
                // WRAM: 0xC000-0xDFFF
                assert!((0xC000..=0xDFFF).contains(&address),
                        "WRAM address {:#X} out of range for {}", address, name);
                Some(DmgBank::WRAM)
            }
            's' => {
                // SRAM: 0xA000-0xBFFF
                assert!((0xA000..=0xBFFF).contains(&address),
                        "SRAM address {:#X} out of range for {}", address, name);
                Some(DmgBank::SRAM { bank: bank_id })
            }
            'v' => {
                // VRAM: 0x8000-0x9FFF
                assert!((0x8000..=0x9FFF).contains(&address),
                        "VRAM address {:#X} out of range for {}", address, name);
                Some(DmgBank::VRAM)
            }
            'h' => {
                // HRAM: 0xFF80-0xFFFE
                assert!((0xFF80..=0xFFFE).contains(&address),
                        "HRAM address {:#X} out of range for {}", address, name);
                Some(DmgBank::HRAM)
            }
            _ => {
                // ROM: 0x0000-0x7FFF
                assert!(address <= 0x7FFF,
                        "ROM address {:#X} out of range for {}", address, name);
                Some(DmgBank::ROM { bank: bank_id })
            }
        }
    }
}

/// Trait for reading memory using pokered symbol file pointers
trait NamedPointerRead {
    fn read_named(&self, pointer: &NamedDmgPointer) -> u8;
    fn read_named_u16_le(&self, pointer: &NamedDmgPointer) -> u16;
    fn read_named_u16_be(&self, pointer: &NamedDmgPointer) -> u16;
    fn read_named_slice(&self, pointer: &NamedDmgPointer, length: usize) -> Vec<u8>;
}

impl NamedPointerRead for MMU {
    fn read_named(&self, pointer: &NamedDmgPointer) -> u8 {
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

    fn read_named_u16_le(&self, pointer: &NamedDmgPointer) -> u16 {
        match pointer.bank {
            DmgBank::ROM { bank } => {
                let bytes = self.rom_data_from_pointer(bank as usize, pointer.address, Some(2));
                u16::from_le_bytes([bytes[0], bytes[1]])
            }
            DmgBank::VRAM | DmgBank::WRAM | DmgBank::HRAM => {
                self.read_u16_le(pointer.address)
            }
            DmgBank::SRAM { .. } => {
                panic!("SRAM banking not implemented")
            }
        }
    }

    fn read_named_u16_be(&self, pointer: &NamedDmgPointer) -> u16 {
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

    fn read_named_slice(&self, pointer: &NamedDmgPointer, length: usize) -> Vec<u8> {
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
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sym_file() {
        let map = PokemonMemoryMap::default();

        // Test ROM entry
        let calc_checksum = map.get_pointer("CalcCheckSum")
            .expect("CalcCheckSum not found");
        assert_eq!(calc_checksum.bank, DmgBank::ROM { bank: 0x1c });
        assert_eq!(calc_checksum.address, 0x7856);

        // Test WRAM entry
        let enemy_mon = map.get_pointer("wEnemyMonUnmodifiedSpecial")
            .expect("wEnemyMonUnmodifiedSpecial not found");
        assert_eq!(enemy_mon.bank, DmgBank::WRAM);
        assert_eq!(enemy_mon.address, 0xcd2c);

        // Test SRAM entry
        let cur_box = map.get_pointer("sCurBoxData")
            .expect("sCurBoxData not found");
        assert_eq!(cur_box.bank, DmgBank::SRAM { bank: 0x01 });
        assert_eq!(cur_box.address, 0xb0c0);

        // Test VRAM entry
        let tileset = map.get_pointer("vTileset")
            .expect("vTileset not found");
        assert_eq!(tileset.bank, DmgBank::VRAM);
        assert_eq!(tileset.address, 0x9000);

        // Test HRAM entry
        let slide_amount = map.get_pointer("hSlideAmount")
            .expect("hSlideAmount not found");
        assert_eq!(slide_amount.bank, DmgBank::HRAM);
        assert_eq!(slide_amount.address, 0xff8b);

        // Test constants
        assert_eq!(map.constants.get("ROUTE6GATE_GUARD"), Some(&0x01));
        assert_eq!(map.constants.get("ROUTE6_COOLTRAINER_M1"), Some(&0x01));
        assert_eq!(map.constants.get("ROUTE7GATE_GUARD"), Some(&0x01));
        assert_eq!(map.constants.get("PEWTERPOKECENTER_GENTLEMAN"), Some(&0x02));
        assert_eq!(map.constants.get("FUCHSIAPOKECENTER_ROCKER"), Some(&0x02));
        assert_eq!(map.constants.get("GAMECORNERPRIZEROOM_GAMBLER"), Some(&0x02));
    }
}
