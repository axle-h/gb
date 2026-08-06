use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Decode, Encode)]
pub struct FlagsRegister {
    pub z: bool, // Zero flag
    pub n: bool, // Subtract flag
    pub h: bool, // Half carry flag
    pub c: bool, // Carry flag
}

impl FlagsRegister {
    pub fn new() -> Self {
        Self {
            z: false,
            n: false,
            h: false,
            c: false,
        }
    }

    pub fn from_byte(byte: u8) -> Self {
        Self {
            z: (byte & 0x80) != 0,
            n: (byte & 0x40) != 0,
            h: (byte & 0x20) != 0,
            c: (byte & 0x10) != 0,
        }
    }

    pub fn to_byte(&self) -> u8 {
        (if self.z { 0x80 } else { 0 }) |
        (if self.n { 0x40 } else { 0 }) |
        (if self.h { 0x20 } else { 0 }) |
        (if self.c { 0x10 } else { 0 })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Decode, Encode)]
pub struct RegisterSet {
    pub a: u8,
    pub flags: FlagsRegister,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16, // Stack Pointer
    pub pc: u16, // Program Counter
}

impl RegisterSet {
    /// The register file the boot ROM leaves behind.
    ///
    /// `gb` never executes a boot ROM, so this stands in for it. **A game detects the console
    /// from `A`**: `0x11` means a Game Boy Color, and every CGB-aware cartridge branches on it.
    ///
    /// ⚠️ **CGB hardware has two different answers**, and which one applies depends on the
    /// *cartridge*, not just the console. A CGB running a DMG-only cartridge takes the boot ROM's
    /// `EmulateDMG` path, which overwrites `DE` and `L` on its way out and loads the title
    /// checksum into `B` — so compatibility mode does **not** share the CGB register file. Getting
    /// this wrong is invisible in practice (few games read these before setting them) and was
    /// shipped wrong once already; see the tests below, which pin both.
    ///
    /// Sources, in agreement: SameBoy's `cgb_boot.asm` traced through `Preboot`/`EmulateDMG`, and
    /// Pan Docs' "Power-Up Sequence" CPU-register table, itself confirmed against mooneye's
    /// `misc/boot_regs-cgb`. Note that **gambatte is wrong here** — `initstate.cpp:1174-1181` uses
    /// the DMG values for CGB with only `A` and `B` changed — so it is not the reference for this.
    ///
    /// ⚠️ The **DMG** values are `gb`'s originals, not hardware's — real DMG leaves `F` at `0xB0`
    /// (Z, H and C set), not `0x80`. Correcting that changes DMG boot behaviour and could move the
    /// committed Pokémon fixtures, so plan task **B11** owns it and needs Alex's call.
    pub fn boot(color_mode: crate::model::ColorMode, cart: &[u8]) -> Self {
        use crate::model::ColorMode;
        match color_mode {
            ColorMode::Dmg => Self::dmg(),
            // A CGB-aware cartridge: the boot ROM's own final block, untouched by `EmulateDMG`.
            ColorMode::Cgb => Self {
                a: 0x11,
                flags: FlagsRegister { z: true, n: false, h: false, c: false },
                b: 0x00,
                c: 0x00,
                d: 0xFF,
                e: 0x56,
                h: 0x00,
                l: 0x0D,
                sp: 0xFFFE,
                pc: 0x0100,
            },
            ColorMode::CgbCompat => {
                let b = crate::boot_palette::compatibility_b_register(cart);
                // `EmulateDMG` ends with `ld de, 8` / `ld l, $7C`, and the shared final block
                // leaves `H` equal to `C`, which is zero — so HL is 0x007C. The exception is the
                // two cartridges whose palette entry carries SameBoy's `$80` flag ("needs the DMG
                // boot tilemap"): loading that tilemap leaves HL pointing into VRAM instead. Those
                // are exactly the entries with title checksum 0x43 and 0x58, both unambiguous, so
                // testing B is equivalent and is how Pan Docs states the rule.
                let hl: u16 = if b == 0x43 || b == 0x58 { 0x991A } else { 0x007C };
                Self {
                    a: 0x11,
                    flags: FlagsRegister { z: true, n: false, h: false, c: false },
                    b,
                    c: 0x00,
                    d: 0x00,
                    e: 0x08,
                    h: (hl >> 8) as u8,
                    l: hl as u8,
                    sp: 0xFFFE,
                    pc: 0x0100,
                }
            }
        }
    }

    pub fn dmg() -> Self {
        Self {
            a: 0x01,
            flags: FlagsRegister {
                z: true,
                n: false,
                h: false,
                c: false,
            },
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
        }
    }

    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }

    pub fn hl_increment(&mut self) -> u16 {
        let value = self.hl();
        self.l = self.l.wrapping_add(1);
        if self.l == 0 {
            self.h = self.h.wrapping_add(1);
        }
        value
    }

    pub fn hl_decrement(&mut self) -> u16 {
        let value = self.hl();
        if self.l == 0 {
            self.h = self.h.wrapping_sub(1);
        }
        self.l = self.l.wrapping_sub(1);
        value
    }

    pub fn set_hl(&mut self, value: u16) {
        self.h = (value >> 8) as u8;
        self.l = value as u8;
    }

    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }

    pub fn set_bc(&mut self, value: u16) {
        self.b = (value >> 8) as u8;
        self.c = value as u8;
    }

    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }

    pub fn set_de(&mut self, value: u16) {
        self.d = (value >> 8) as u8;
        self.e = value as u8;
    }

    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.flags.to_byte()])
    }

    pub fn set_af(&mut self, value: u16) {
        self.a = (value >> 8) as u8;
        self.flags = FlagsRegister::from_byte(value as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_register_empty() {
        let flags = FlagsRegister::from_byte(0);
        assert!(!flags.z);
        assert!(!flags.n);
        assert!(!flags.h);
        assert!(!flags.c);
    }

    #[test]
    fn flags_register_from_byte() {
        let flags = FlagsRegister::from_byte(0b11110000);
        assert!(flags.z);
        assert!(flags.n);
        assert!(flags.h);
        assert!(flags.c);
    }

    #[test]
    fn flags_register_to_byte() {
        let mut flags = FlagsRegister::from_byte(0);
        assert_eq!(flags.to_byte(), 0b00000000);

        flags.z = true;
        assert_eq!(flags.to_byte(), 0b10000000);
        flags.n = true;
        assert_eq!(flags.to_byte(), 0b11000000);
        flags.h = true;
        assert_eq!(flags.to_byte(), 0b11100000);
        flags.c = true;
        assert_eq!(flags.to_byte(), 0b11110000);
    }

    #[test]
    fn register_set_initialization() {
        let registers = RegisterSet::dmg();
        assert_eq!(registers.a, 0x01);
        assert_eq!(registers.flags.z, true);
        assert_eq!(registers.flags.n, false);
        assert_eq!(registers.flags.h, false);
        assert_eq!(registers.flags.c, false);
        assert_eq!(registers.b, 0x00);
        assert_eq!(registers.c, 0x13);
        assert_eq!(registers.d, 0x00);
        assert_eq!(registers.e, 0xD8);
        assert_eq!(registers.h, 0x01);
    }

    #[test]
    fn register_set_hl() {
        let mut registers = RegisterSet::dmg();
        registers.set_hl(0x1234);
        assert_eq!(registers.hl(), 0x1234);
        assert_eq!(registers.h, 0x12);
        assert_eq!(registers.l, 0x34);
    }

    #[test]
    fn register_set_bc() {
        let mut registers = RegisterSet::dmg();
        registers.set_bc(0x5678);
        assert_eq!(registers.bc(), 0x5678);
        assert_eq!(registers.b, 0x56);
        assert_eq!(registers.c, 0x78);
    }

    #[test]
    fn register_set_de() {
        let mut registers = RegisterSet::dmg();
        registers.set_de(0x9ABC);
        assert_eq!(registers.de(), 0x9ABC);
        assert_eq!(registers.d, 0x9A);
        assert_eq!(registers.e, 0xBC);
    }

    #[test]
    fn register_set_af() {
        let mut registers = RegisterSet::dmg();
        registers.set_af(0x1234);
        assert_eq!(registers.af(), 0x1230);
        assert_eq!(registers.a, 0x12);
        assert_eq!(registers.flags.to_byte(), 0x30);
    }

    #[test]
    fn register_set_increment_hl() {
        let mut registers = RegisterSet::dmg();
        registers.set_hl(0x1234);
        let value = registers.hl_increment();
        assert_eq!(value, 0x1234);
        assert_eq!(registers.hl(), 0x1235);

        // Test low overflow
        registers.set_hl(0x00FF);
        let value = registers.hl_increment();
        assert_eq!(value, 0x00FF);
        assert_eq!(registers.hl(), 0x0100);

        // Test high overflow
        registers.set_hl(0xFFFF);
        let value = registers.hl_increment();
        assert_eq!(value, 0xFFFF);
        assert_eq!(registers.hl(), 0x0000); // Should wrap around to 0x0000
    }

    /// B9. **A cartridge detects the console from `A`.** `0x11` is a Game Boy Color; anything
    /// else is not, and a CGB-aware game will take its DMG path.
    ///
    /// The rest of the file matters less — few games read `B`/`DE`/`HL` before setting them — but
    /// it is pinned because **CGB hardware has two answers and the first version of this shipped
    /// only one of them**, using the CGB-mode file for compatibility mode too. Values from Pan
    /// Docs' "Power-Up Sequence" table, independently re-derived by tracing SameBoy's
    /// `cgb_boot.asm`; the two agree exactly.
    #[test]
    fn the_boot_register_file_matches_the_boot_rom() {
        use crate::model::ColorMode;

        let dmg = RegisterSet::boot(ColorMode::Dmg, crate::pokemon::roms::POKERED);
        assert_eq!(dmg, RegisterSet::dmg(), "the DMG path must be untouched");
        assert_eq!(dmg.a, 0x01);

        // A CGB-aware cartridge: the boot ROM's own final block.
        let cgb = RegisterSet::boot(ColorMode::Cgb, crate::roms::cgb_acid::ROM);
        assert_eq!(cgb.a, 0x11, "a CGB identifies itself in A");
        assert_eq!(cgb.flags.to_byte(), 0x80, "Z only, from the `xor a` before the handoff");
        assert_eq!((cgb.b, cgb.c), (0x00, 0x00));
        assert_eq!(cgb.de(), 0xFF56);
        assert_eq!(cgb.hl(), 0x000D);
        assert_eq!((cgb.sp, cgb.pc), (0xFFFE, 0x0100));

        // ...and compatibility mode, which is a *different* file: `EmulateDMG` overwrites DE and
        // L on its way out, and B carries the title checksum.
        let compat = RegisterSet::boot(ColorMode::CgbCompat, crate::pokemon::roms::POKERED);
        assert_eq!(compat.a, 0x11, "still CGB hardware");
        assert_eq!(compat.flags.to_byte(), 0x80);
        assert_eq!(compat.b, 0x14, "Pokemon Red's title checksum, via hTitleChecksum");
        assert_eq!(compat.c, 0x00);
        assert_eq!(compat.de(), 0x0008, "`ld de, 8` at the end of EmulateDMG");
        assert_eq!(compat.hl(), 0x007C, "`ld l, $7C`, and H = C = 0");
        assert_eq!((compat.sp, compat.pc), (0xFFFE, 0x0100));

        assert_ne!(cgb, compat, "the two CGB files must not be the same — that was the bug");
    }

    /// `B` is the title checksum only for a **first-party** cartridge — the same licensee check
    /// that gates the compatibility palette. A third-party cartridge gets `0x00`.
    #[test]
    fn the_compatibility_b_register_follows_the_licensee_check() {
        use crate::model::ColorMode;

        let mut rom = crate::pokemon::roms::POKERED.to_vec();
        assert_eq!(RegisterSet::boot(ColorMode::CgbCompat, &rom).b, 0x14);

        rom[0x14B] = 0x9C; // some other publisher
        assert_eq!(RegisterSet::boot(ColorMode::CgbCompat, &rom).b, 0x00);
        assert_eq!(RegisterSet::boot(ColorMode::CgbCompat, &rom).hl(), 0x007C);
    }

    /// The two cartridges whose palette entry carries SameBoy's `$80` flag get the DMG boot
    /// tilemap loaded, which leaves `HL` pointing into VRAM rather than at `0x007C`. Pan Docs
    /// states the rule on `B`; these are checksums `0x43` and `0x58`.
    #[test]
    fn the_dmg_tilemap_cartridges_hand_over_hl_pointing_into_vram() {
        use crate::model::ColorMode;

        // Synthesise a Nintendo cartridge whose 16 title bytes sum to the target checksum.
        fn cart_with_checksum(checksum: u8) -> Vec<u8> {
            let mut rom = vec![0u8; 0x150];
            rom[0x134] = checksum;
            rom[0x14B] = 0x01; // Nintendo, old licensee code
            rom
        }

        for checksum in [0x43u8, 0x58] {
            let registers = RegisterSet::boot(ColorMode::CgbCompat, &cart_with_checksum(checksum));
            assert_eq!(registers.b, checksum);
            assert_eq!(registers.hl(), 0x991A, "checksum {checksum:#04X}");
        }
        // A neighbouring checksum takes the ordinary path.
        assert_eq!(
            RegisterSet::boot(ColorMode::CgbCompat, &cart_with_checksum(0x44)).hl(),
            0x007C
        );
    }

    /// ...and it reaches the machine, not just the constructor — including across a reset, which
    /// rebuilds the file from the cartridge.
    #[test]
    fn a_cgb_boots_reporting_itself() {
        use crate::game_boy::GameBoy;

        assert_eq!(GameBoy::cgb(crate::roms::cgb_acid::ROM).core().registers().a, 0x11);
        assert_eq!(GameBoy::dmg(crate::pokemon::roms::POKERED).core().registers().a, 0x01);

        let mut compat = GameBoy::cgb(crate::pokemon::roms::POKERED);
        assert_eq!(compat.core().registers().a, 0x11, "compatibility mode is still CGB hardware");
        assert_eq!(compat.core().registers().b, 0x14, "and B carries the title checksum");

        compat.run(crate::cycles::MachineCycles::from_m(100_000));
        compat.reset();
        assert_eq!(compat.core().registers().b, 0x14, "a reset rebuilds it from the cartridge");
    }

    #[test]
    fn register_set_decrement_hl() {
        let mut registers = RegisterSet::dmg();
        registers.set_hl(0x1234);
        let value = registers.hl_decrement();
        assert_eq!(value, 0x1234);
        assert_eq!(registers.hl(), 0x1233);

        // Test high underflow
        registers.set_hl(0x0100);
        let value = registers.hl_decrement();
        assert_eq!(value, 0x0100);
        assert_eq!(registers.hl(), 0x00FF);

        // Test low underflow
        registers.set_hl(0x0000);
        let value = registers.hl_decrement();
        assert_eq!(value, 0x0000);
        assert_eq!(registers.hl(), 0xFFFF); // Should wrap around to 0xFFFF
    }
}