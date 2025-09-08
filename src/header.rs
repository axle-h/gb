/// https://gbdev.io/pandocs/The_Cartridge_Header.html#0147--cartridge-type
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::FromRepr)]
#[repr(u8)]
pub enum CartType {
    RomOnly = 0x00,
    MBC1 = 0x01,
    MBC1Ram = 0x02,
    MBC1RamBattery = 0x03,
    MBC2 = 0x05,
    MBC2Battery = 0x06,
    MMM01 = 0x0B,
    MMM01Ram = 0x0C,
    MMM01RamBattery = 0x0D,
    NBC3TimerBattery = 0x0F,
    MBC3TimerRamBattery = 0x10,
    MBC3 = 0x11,
    MBC3Ram = 0x12,
    MBC3RamBattery = 0x13,
    MBC5 = 0x19,
    MBC5Ram = 0x1A,
    MBC5RamBattery = 0x1B,
    MBC5Rumble = 0x1C,
    MBC5RumbleRam = 0x1D,
    MBC5RumbleRamBattery = 0x1E,
    MBC6 = 0x20,
    MBC7SensorRumbleRamBattery = 0x22,
    PocketCamera = 0xFC,
    BandaiTama5 = 0xFD,
    HuC3 = 0xFE,
    HuC1RamBattery = 0xFF,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CGBMode {
    None,
    Enhanced,
    Exclusive
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartHeader {
    title: String,
    cgb_mode: CGBMode,
    cart_type: CartType,
    rom_banks: usize,
    ram_banks: usize,
}

impl CartHeader {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let title_bytes = data.get(0x0134..0x0143).ok_or("Invalid title length")?;
        let title_length = title_bytes.iter()
            .position(|&c| c == b'\0') // terminate at null byte
            .unwrap_or(title_bytes.len());
        let title = std::str::from_utf8(&title_bytes[0..title_length])
            .map_err(|_| "Invalid UTF-8 in title")
            ?.to_string();

        let cgb_mode = match data.get(0x0143) {
            Some(&0x80) => CGBMode::Enhanced,
            Some(&0xC0) => CGBMode::Exclusive,
            _ => CGBMode::None,
        };

        let cart_type = data.get(0x0147)
            .and_then(|&cart_type_byte| CartType::from_repr(cart_type_byte))
            .ok_or("Invalid cartridge type")?;

        let rom_banks = data.get(0x0148)
            .and_then(|&value| {
                if value < 0x09 {
                    Some(1 << (value + 1))
                } else {
                    None
                }
            })
            .ok_or("Invalid ROM size")?;

        let ram_banks = data.get(0x0149)
            .and_then(|&value| {
                match value {
                    0x00 | 0x01 => Some(0),
                    0x02 => Some(1),
                    0x03 => Some(4),
                    0x04 => Some(16),
                    0x05 => Some(8),
                    _ => None,
                }
            })
            .ok_or("Invalid RAM size")?;

        Ok(Self { title, cgb_mode, cart_type, rom_banks, ram_banks })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn cgb_mode(&self) -> CGBMode {
        self.cgb_mode
    }

    pub fn cart_type(&self) -> CartType {
        self.cart_type
    }

    pub fn rom_banks(&self) -> usize {
        self.rom_banks
    }

    pub fn ram_banks(&self) -> usize {
        self.ram_banks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_instrs() {
        let header = CartHeader::parse(crate::roms::blargg_cpu::ROM)
            .expect("Failed to parse CPU_INSTRS header");
        assert_eq!(header.title(), "CPU_INSTRS");
        assert_eq!(header.cgb_mode(), CGBMode::Enhanced);
        assert_eq!(header.cart_type(), CartType::MBC1);
        assert_eq!(header.rom_banks(), 4); // 64KB ROM
        assert_eq!(header.ram_banks(), 0); // No RAM
    }

    #[test]
    fn parse_cpu_tetris() {
        let header = CartHeader::parse(crate::roms::commercial::TETRIS)
            .expect("Failed to parse TETRIS header");
        assert_eq!(header.title(), "TETRIS");
        assert_eq!(header.cgb_mode(), CGBMode::None);
        assert_eq!(header.cart_type(), CartType::RomOnly);
        assert_eq!(header.rom_banks(), 2); // 32KB ROM
        assert_eq!(header.ram_banks(), 0); // No RAM
    }
}