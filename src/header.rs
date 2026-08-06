use bincode::{Decode, Encode};

/// https://gbdev.io/pandocs/The_Cartridge_Header.html#0147--cartridge-type
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::FromRepr, Decode, Encode)]
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

impl CartType {
    /// Whether the cartridge carries a real-time clock chip.
    ///
    /// Only two types declare one, and it matters because the clock's registers replace cartridge
    /// RAM at `0xA000` when `0x08..=0x0C` is selected — so on a cartridge *without* a timer, that
    /// same selection is an ordinary RAM-bank number and wraps. `pokered.gbc` is `0x13`, which has
    /// no timer.
    pub fn has_rtc(self) -> bool {
        matches!(self, CartType::NBC3TimerBattery | CartType::MBC3TimerRamBattery)
    }

    /// Whether the RAM this cartridge addresses is built into the mapper rather than described by
    /// header byte `0x149`.
    ///
    /// MBC2 is the only one: it has 512 nibbles on the chip and declares **zero** banks, so a
    /// bank has to be allocated for it regardless of what the header says.
    pub fn has_builtin_ram(self) -> bool {
        matches!(self, CartType::MBC2 | CartType::MBC2Battery)
    }

    /// Whether `gb` emulates this mapper at all.
    ///
    /// **D7.** The rejected set is gambatte's (`cartridge.cpp:592-615`): refusing to load is
    /// honest, where running MMM01 as though it were MBC1 produces a machine that looks like it
    /// works and is quietly wrong. `RomOnly`, MBC1, MBC2, MBC3, MBC5 and HuC1 are emulated; the
    /// multi-game, sensor and camera mappers are not.
    pub fn is_emulated(self) -> bool {
        use CartType::*;
        !matches!(
            self,
            MMM01
                | MMM01Ram
                | MMM01RamBattery
                | MBC6
                | MBC7SensorRumbleRamBattery
                | PocketCamera
                | BandaiTama5
                | HuC3
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Decode, Encode)]
pub enum CGBMode {
    None,
    Enhanced,
    Exclusive
}

#[derive(Debug, Clone, PartialEq, Eq, Decode, Encode)]
pub struct CartHeader {
    title: String,
    cgb_mode: CGBMode,
    cart_type: CartType,
    rom_banks: usize,
    ram_banks: usize,
}

/// Why a cartridge could not be loaded.
///
/// **D8.** This replaces `Result<_, String>`. Only two things are genuinely fatal — the image is
/// too small to hold a header, and the mapper is one `gb` does not emulate. Everything the old
/// code rejected besides those was **a bug**: real cartridges hit those paths. See
/// [`CartHeader::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Smaller than the 336-byte header, so there is nothing to parse.
    TooSmall { len: usize },
    /// Byte `0x147` is not a cartridge type this build knows at all.
    UnknownCartType(u8),
    /// A mapper `gb` recognises but cannot emulate. Gambatte rejects these at load rather than
    /// mis-emulating them (`cartridge.cpp:592-615`), and so does this.
    UnsupportedMbc(CartType),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall { len } => {
                write!(f, "ROM is {len} bytes, too small to contain a cartridge header")
            }
            Self::UnknownCartType(byte) => write!(f, "unknown cartridge type {byte:#04X}"),
            Self::UnsupportedMbc(cart_type) => {
                write!(f, "{cart_type:?} is not an emulated mapper")
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl CartHeader {
    /// Parse the cartridge header at `0x0134..=0x014F`.
    ///
    /// ⚠️ **Three things here used to reject perfectly valid cartridges**, and every one of them
    /// was found by trying to run somebody else's test ROMs:
    ///
    /// 1. **The title was decoded as UTF-8.** Real headers put the manufacturer code and the CGB
    ///    flag *inside* `0x134..=0x143`, so bytes like `0x80` and `0xC0` land in the slice and the
    ///    decode fails. It is a fixed-width byte field, not a string; it is filtered, not decoded.
    /// 2. **ROM-size bytes `0x52`, `0x53` and `0x54` were rejected.** They are legal. The value is
    ///    now advisory anyway — [`crate::mmu::MMU`] derives the real bank count from the file
    ///    length (D1), because cartridges lie about this field.
    /// 3. **An unknown RAM size was rejected.** Now it defaults, because gambatte's own test ROMs
    ///    declare `0x147 = 0x03` (MBC1+RAM) with `0x149 = 0x00`, and dropping every SRAM write is
    ///    a far worse failure than allocating four banks nobody uses.
    pub fn parse(data: &[u8]) -> Result<Self, LoadError> {
        if data.len() < 0x0150 {
            return Err(LoadError::TooSmall { len: data.len() });
        }

        let cart_type = CartType::from_repr(data[0x0147])
            .ok_or(LoadError::UnknownCartType(data[0x0147]))?;
        if !cart_type.is_emulated() {
            return Err(LoadError::UnsupportedMbc(cart_type));
        }

        let cgb_mode = match data[0x0143] {
            0x80 => CGBMode::Enhanced,
            0xC0 => CGBMode::Exclusive,
            _ => CGBMode::None,
        };

        // A CGB-aware header shortens the title to 15 bytes to make room for the flag, but the
        // printable-byte filter handles that without needing to know which layout this is.
        let title = data[0x0134..0x0143]
            .iter()
            .copied()
            .take_while(|&c| (0x20..0x80).contains(&c))
            .map(char::from)
            .collect::<String>()
            .trim_end()
            .to_string();

        // Advisory only — see the doc comment. Anything out of range is reported as the minimum.
        let rom_banks = match data[0x0148] {
            value @ 0x00..=0x08 => 1 << (value + 1),
            0x52 => 72,
            0x53 => 80,
            0x54 => 96,
            _ => 2,
        };

        let ram_banks = if cart_type.has_builtin_ram() {
            // MBC2's 512 nibbles are on the mapper and its header always says zero.
            1
        } else {
            match data[0x0149] {
                0x00 | 0x01 => 0,
                0x02 => 1,
                0x03 => 4,
                0x04 => 16,
                0x05 => 8,
                // Unknown. Four banks is the common case and costs 32 KB; refusing to load, or
                // allocating none and silently dropping every write, are both worse.
                _ => 4,
            }
        };

        Ok(Self { title, cgb_mode, cart_type, rom_banks, ram_banks })
    }

    /// Whether byte `0x14D` agrees with the header it covers.
    ///
    /// The boot ROM refuses to start a cartridge that fails this, but `gb` does not run a boot
    /// ROM, so it is **advisory** — reported by [`CartHeader::parse`]'s caller as a warning rather
    /// than enforced. Plenty of homebrew and test ROMs ship a wrong one and run fine on hardware
    /// with a flash cart.
    pub fn checksum_valid(data: &[u8]) -> bool {
        let Some(slice) = data.get(0x0134..0x014D) else { return false };
        let sum = slice.iter().fold(0u8, |acc, &b| acc.wrapping_sub(b).wrapping_sub(1));
        data.get(0x014D) == Some(&sum)
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

    /// A header with the bytes a real cartridge puts there. `0x134..=0x142` holds the title *and*
    /// the manufacturer code, so high bytes land in it — and the old UTF-8 decode rejected the
    /// whole cartridge for it.
    #[test]
    fn a_title_with_non_ascii_bytes_still_parses() {
        let mut rom = crate::pokemon::roms::POKERED.to_vec();
        rom[0x0140] = 0x80; // a manufacturer-code byte, invalid UTF-8 on its own
        rom[0x0141] = 0xC0;
        let header = CartHeader::parse(&rom).expect("a high byte is not a parse failure");
        assert_eq!(header.title(), "POKEMON RED", "the title stops at the first non-printable");
    }

    /// ⚠️ `0x52`, `0x53` and `0x54` are legal ROM sizes and used to be rejected outright.
    #[test]
    fn the_odd_rom_sizes_are_legal() {
        for (byte, expected) in [(0x52, 72), (0x53, 80), (0x54, 96)] {
            let mut rom = crate::pokemon::roms::POKERED.to_vec();
            rom[0x0148] = byte;
            let header = CartHeader::parse(&rom).expect("a legal ROM size");
            assert_eq!(header.rom_banks(), expected);
        }
    }

    /// An unknown RAM size defaults rather than failing. Gambatte's own test ROMs declare
    /// `0x147 = 0x03` with `0x149 = 0x00`, and dropping every SRAM write is the worse failure.
    #[test]
    fn an_unknown_ram_size_defaults() {
        let mut rom = crate::pokemon::roms::POKERED.to_vec();
        rom[0x0149] = 0x7F;
        assert_eq!(CartHeader::parse(&rom).expect("still loads").ram_banks(), 4);
    }

    /// MBC2's RAM is on the mapper, so its bank count does not come from the header at all.
    #[test]
    fn mbc2_gets_a_ram_bank_whatever_the_header_says() {
        let mut rom = crate::pokemon::roms::POKERED.to_vec();
        rom[0x0147] = 0x05; // MBC2
        rom[0x0149] = 0x00; // "no RAM", as every MBC2 header says
        assert_eq!(CartHeader::parse(&rom).expect("loads").ram_banks(), 1);
    }

    /// **D7.** A mapper `gb` cannot emulate is refused, not run as something else.
    #[test]
    fn an_unsupported_mapper_is_rejected() {
        let mut rom = crate::pokemon::roms::POKERED.to_vec();
        rom[0x0147] = 0xFE; // HuC3
        assert_eq!(
            CartHeader::parse(&rom),
            Err(LoadError::UnsupportedMbc(CartType::HuC3)),
        );

        rom[0x0147] = 0x21; // not a cartridge type at all
        assert_eq!(CartHeader::parse(&rom), Err(LoadError::UnknownCartType(0x21)));
    }

    /// An image too small to hold a header is the one other fatal case.
    #[test]
    fn a_truncated_image_is_rejected() {
        assert_eq!(CartHeader::parse(&[0; 0x100]), Err(LoadError::TooSmall { len: 0x100 }));
    }

    /// **D8.** The fallible constructor reports rather than panics.
    #[test]
    fn try_dmg_reports_a_bad_cartridge() {
        let mut rom = crate::pokemon::roms::POKERED.to_vec();
        rom[0x0147] = 0xFD; // TAMA5
        let error = crate::game_boy::GameBoy::try_dmg(&rom).expect_err("must not load");
        assert_eq!(error, LoadError::UnsupportedMbc(CartType::BandaiTama5));
        assert!(error.to_string().contains("not an emulated mapper"), "{error}");
    }

    /// Every committed ROM has a valid header checksum, so the load-time warning stays quiet.
    /// This also pins the algorithm: five independent ROMs agreeing is not a coincidence.
    #[test]
    fn the_committed_roms_all_checksum() {
        for (name, rom) in [
            ("pokered", crate::pokemon::roms::POKERED),
            ("cpu_instrs", crate::roms::blargg_cpu::ROM),
            ("dmg_sound", crate::roms::blargg_dmg_sound::ROM),
            ("dmg-acid2", crate::roms::acid::ROM),
        ] {
            assert!(CartHeader::checksum_valid(rom), "{name} should checksum");
        }

        let mut broken = crate::pokemon::roms::POKERED.to_vec();
        broken[0x0134] ^= 0xFF;
        assert!(!CartHeader::checksum_valid(&broken), "...and a corrupted header should not");
    }

    #[test]
    fn parse_cpu_tetris() {
        let header = CartHeader::parse(crate::pokemon::roms::POKERED)
            .expect("Failed to parse POKERED header");
        assert_eq!(header.title(), "POKEMON RED");
        assert_eq!(header.cgb_mode(), CGBMode::None);
        assert_eq!(header.cart_type(), CartType::MBC3RamBattery);
        assert_eq!(header.rom_banks(), 64);
        assert_eq!(header.ram_banks(), 4);
    }
}