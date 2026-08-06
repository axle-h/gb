//! Memory bank controllers — the cartridge hardware that decides which ROM bank sits at
//! `0x4000..=0x7FFF` and what `0xA000..=0xBFFF` addresses.
//!
//! **D2.** Before this module `gb` had no MBC abstraction at all: [`crate::header::CartType`] was
//! parsed and never dispatched on, and one hardcoded pseudo-mapper — MBC1's register layout with
//! MBC3's 7-bit width — served every cartridge. It worked only because Pokémon Red is
//! MBC3-no-RTC under 128 banks.
//!
//! # Why an enum and not `Box<dyn Mbc>`
//!
//! The plan specifies `Box<dyn Mbc>`. [`Mapper`] is an enum instead, and the reason is
//! [`crate::mmu::MMU`]'s derives: it is `Clone + PartialEq`, and the save state needs
//! `Encode + Decode`. A boxed trait object gives none of those — it would need a hand-written
//! `Encode`/`Decode` (which the plan flags), plus `clone_box` and a snapshot-comparing
//! `PartialEq`, all to buy an open set of mappers that a Game Boy emulator will never have. The
//! enum derives all five. [`Mbc`] survives as the interface each mapper implements, which is what
//! the trait was for.
//!
//! # The one rule worth internalising
//!
//! **Every mapper resolves its register to a physical bank differently, and the differences are
//! not decoration.** Three of the six do something distinct with a bank-0 selection:
//!
//! | Mapper | register → bank | bank 0 reachable at `0x4000`? |
//! |---|---|---|
//! | MBC1 | `adjust(reg) & (n-1)`, `adjust(b) = b & 0x1F ? b : b\|1` | **yes**, by wrapping |
//! | MBC3 | `max(reg & (n-1), 1)` | no — the remap is applied *after* the wrap |
//! | MBC2 / MBC5 / HuC1 | `reg & (n-1)` | yes — no remap at all |
//!
//! ⚠️ MBC1 and MBC3 differ **only in the order of the same two operations**, and the order is
//! observable. MBC1 remaps then wraps, so `4` on a four-bank cartridge is bank 0 — which is how
//! blargg's combined `dmg_sound.gb` reaches the terminator in its bank 0 (plan task D1). MBC3
//! wraps then remaps, so the same write is bank 1. Every one of these is gambatte's
//! `setRombank()` for that mapper, `mem/cartridge.cpp`.

use bincode::{Decode, Encode};

use crate::header::CartType;
use crate::rtc::Rtc;

/// What `0xA000..=0xBFFF` currently addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamTarget {
    /// Cartridge RAM, at this bank. Already wrapped against the banks that exist.
    Bank(usize),
    /// One of MBC3's real-time-clock registers, selected by writing `0x08..=0x0C` to
    /// `0x4000..=0x5FFF`. The clock is [`crate::rtc::Rtc`]; only the two cartridge types that
    /// declare a timer ever report this, so on Pokémon Red it never occurs.
    Rtc(u8),
    /// Nothing is mapped: RAM is disabled, or the cartridge has none.
    None,
}

/// The interface every mapper implements. Deliberately narrow: a mapper sees writes to
/// `0x0000..=0x7FFF` and answers two questions about the memory map. It never sees a read, because
/// reads must stay on [`crate::mmu::MMU`]'s inlined fast path (C6) — the MMU caches the answers
/// and refreshes them after each write.
pub trait Mbc {
    /// A guest write to `0x0000..=0x7FFF`. Address decoding is the mapper's own business:
    /// ⚠️ five of the six decode `address >> 13 & 3`, but **MBC2 decodes `address & 0x6100`**,
    /// because it looks at A8 as well.
    fn rom_write(&mut self, address: u16, value: u8);

    /// The bank mapped at `0x4000..=0x7FFF`, already wrapped to a bank that exists.
    fn rom_bank(&self) -> usize;

    /// What `0xA000..=0xBFFF` addresses right now.
    fn ram_target(&self) -> RamTarget;

    /// Whether the guest has unlocked cartridge RAM by writing `0x?A` to `0x0000..=0x1FFF`.
    fn ram_enabled(&self) -> bool;

    /// Adopt the effective bank/enable state of a save state written before the `mbc` section
    /// existed. See [`Mapper::restore_effective`].
    fn restore_effective(&mut self, rom_bank: usize, ram_bank: usize, ram_enabled: bool);

    /// The cartridge's real-time clock, if it has one. Only MBC3's two timer types do (D5).
    fn rtc(&self) -> Option<&Rtc> {
        None
    }

    fn rtc_mut(&mut self) -> Option<&mut Rtc> {
        None
    }
}

/// How many banks a mapper wraps against. Always a power of two of at least 1, so the wrap is a
/// mask — [`crate::mmu::pad_rom`] guarantees it for ROM, and every legal value of header byte
/// `0x149` gives one for RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct BankCounts {
    pub rom: usize,
    pub ram: usize,
}

impl BankCounts {
    fn wrap_rom(&self, bank: usize) -> usize {
        bank & (self.rom.max(1) - 1)
    }

    /// `None` when the cartridge has no RAM at all, which is not the same as having one bank.
    fn wrap_ram(&self, bank: usize) -> Option<usize> {
        match self.ram {
            0 => None,
            banks if banks.is_power_of_two() => Some(bank & (banks - 1)),
            // Unreachable for any legal header, but D8 will start defaulting unknown sizes and an
            // out-of-bounds bank index here would be a panic in the memory path.
            banks => Some(bank % banks),
        }
    }
}

/// Every mapper `gb` implements. See the module documentation for why this is an enum.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub enum Mapper {
    RomOnly(RomOnly),
    Mbc1(Mbc1),
    Mbc2(Mbc2),
    Mbc3(Mbc3),
    Mbc5(Mbc5),
    HuC1(HuC1),
}

impl Mapper {
    /// Select the mapper a cartridge header asks for.
    ///
    /// Cartridge types `gb` cannot emulate never reach here — [`CartHeader::parse`] rejects them
    /// with [`crate::header::LoadError::UnsupportedMbc`] (D7/D8), rather than running them as
    /// something else and looking like it worked.
    pub fn new(cart_type: CartType, banks: BankCounts) -> Self {
        use CartType::*;
        match cart_type {
            RomOnly => Self::RomOnly(self::RomOnly::new(banks)),
            MBC2 | MBC2Battery => Self::Mbc2(self::Mbc2::new(banks)),
            NBC3TimerBattery | MBC3TimerRamBattery | MBC3 | MBC3Ram | MBC3RamBattery => {
                Self::Mbc3(self::Mbc3::new(banks, cart_type.has_rtc()))
            }
            MBC5 | MBC5Ram | MBC5RamBattery | MBC5Rumble | MBC5RumbleRam | MBC5RumbleRamBattery => {
                Self::Mbc5(self::Mbc5::new(banks))
            }
            HuC1RamBattery => Self::HuC1(self::HuC1::new(banks)),
            _ => Self::Mbc1(self::Mbc1::new(banks)),
        }
    }

    fn as_mbc(&self) -> &dyn Mbc {
        match self {
            Self::RomOnly(m) => m,
            Self::Mbc1(m) => m,
            Self::Mbc2(m) => m,
            Self::Mbc3(m) => m,
            Self::Mbc5(m) => m,
            Self::HuC1(m) => m,
        }
    }

    fn as_mbc_mut(&mut self) -> &mut dyn Mbc {
        match self {
            Self::RomOnly(m) => m,
            Self::Mbc1(m) => m,
            Self::Mbc2(m) => m,
            Self::Mbc3(m) => m,
            Self::Mbc5(m) => m,
            Self::HuC1(m) => m,
        }
    }

    /// Rebuild the mapper's registers from a save state that predates the `mbc` section — which is
    /// all 91 committed fixtures.
    ///
    /// The `cart` section has always carried the **effective** bank numbers and the RAM-enable
    /// flag, so this is exact for the mapper that matters (Pokémon Red's MBC3, whose register *is*
    /// its effective bank below 64). It cannot recover an MBC1 mode bit or MBC5's ninth bank bit,
    /// and it does not need to: no such state was ever written.
    pub fn restore_effective(&mut self, rom_bank: usize, ram_bank: usize, ram_enabled: bool) {
        self.as_mbc_mut().restore_effective(rom_bank, ram_bank, ram_enabled);
    }
}

impl Mbc for Mapper {
    fn rom_write(&mut self, address: u16, value: u8) {
        self.as_mbc_mut().rom_write(address, value)
    }

    fn rom_bank(&self) -> usize {
        self.as_mbc().rom_bank()
    }

    fn ram_target(&self) -> RamTarget {
        self.as_mbc().ram_target()
    }

    fn ram_enabled(&self) -> bool {
        self.as_mbc().ram_enabled()
    }

    fn restore_effective(&mut self, rom_bank: usize, ram_bank: usize, ram_enabled: bool) {
        self.as_mbc_mut().restore_effective(rom_bank, ram_bank, ram_enabled)
    }

    fn rtc(&self) -> Option<&Rtc> {
        self.as_mbc().rtc()
    }

    fn rtc_mut(&mut self) -> Option<&mut Rtc> {
        self.as_mbc_mut().rtc_mut()
    }
}

/// The four-way decode five of the six mappers share: `0x0000`, `0x2000`, `0x4000`, `0x6000`.
/// Gambatte writes it `p >> 13 & 3` (`cartridge.cpp`), and so does this.
fn region(address: u16) -> u8 {
    (address >> 13 & 3) as u8
}

/// Whether cartridge RAM is unlocked. Every mapper agrees on this one: the low nibble must be
/// `0xA`, so `0x0A` and `0x1A` both unlock and `0x00` locks.
fn unlocks_ram(value: u8) -> bool {
    value & 0x0F == 0x0A
}

/// No mapper at all: 32 KB of ROM, bank 1 permanently at `0x4000`.
///
/// Gambatte's `Mbc0` still honours the RAM-enable register, because a few `0x00` cartridges do
/// carry RAM, so this does too.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct RomOnly {
    banks: BankCounts,
    ram_enabled: bool,
}

impl RomOnly {
    fn new(banks: BankCounts) -> Self {
        Self { banks, ram_enabled: false }
    }
}

impl Mbc for RomOnly {
    fn rom_write(&mut self, address: u16, value: u8) {
        if address < 0x2000 {
            self.ram_enabled = unlocks_ram(value);
        }
    }

    fn rom_bank(&self) -> usize {
        1
    }

    fn ram_target(&self) -> RamTarget {
        match (self.ram_enabled, self.banks.wrap_ram(0)) {
            (true, Some(bank)) => RamTarget::Bank(bank),
            _ => RamTarget::None,
        }
    }

    fn ram_enabled(&self) -> bool {
        self.ram_enabled
    }

    fn restore_effective(&mut self, _rom_bank: usize, _ram_bank: usize, ram_enabled: bool) {
        self.ram_enabled = ram_enabled;
    }
}

/// **D3.** MBC1: a 5-bit ROM-bank register plus a 2-bit register that is *either* the top two ROM
/// bank bits or the RAM bank, depending on the mode bit at `0x6000..=0x7FFF`.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Mbc1 {
    banks: BankCounts,
    /// The assembled 7-bit bank: low five bits from `0x2000`, bits 5-6 from `0x4000` in mode 0.
    rom_bank: usize,
    ram_bank: usize,
    ram_enabled: bool,
    /// The `0x6000..=0x7FFF` mode bit. `false` routes the 2-bit register to ROM, `true` to RAM.
    ram_bank_mode: bool,
}

impl Mbc1 {
    fn new(banks: BankCounts) -> Self {
        Self { banks, rom_bank: 1, ram_bank: 0, ram_enabled: false, ram_bank_mode: false }
    }

    /// ⚠️ **The aliasing tests the low five bits, not the whole register**, so `0x20` becomes
    /// `0x21` and not `0x01` — the classic MBC1 hole where banks `0x00/0x20/0x40/0x60` are
    /// unreachable at `0x4000`. Getting this wrong silently swaps a 1 MB cartridge's banks.
    fn adjusted(bank: usize) -> usize {
        if bank & 0x1F != 0 { bank } else { bank | 1 }
    }
}

impl Mbc for Mbc1 {
    fn rom_write(&mut self, address: u16, value: u8) {
        match region(address) {
            0 => self.ram_enabled = unlocks_ram(value),
            // In mode 1 the write replaces the whole register; in mode 0 it keeps bits 5-6.
            1 => {
                let low = value as usize & 0x1F;
                self.rom_bank = if self.ram_bank_mode { low } else { (self.rom_bank & 0x60) | low };
            }
            2 => {
                if self.ram_bank_mode {
                    self.ram_bank = value as usize & 0x03;
                } else {
                    self.rom_bank = ((value as usize) << 5 & 0x60) | (self.rom_bank & 0x1F);
                }
            }
            _ => self.ram_bank_mode = value & 1 != 0,
        }
    }

    fn rom_bank(&self) -> usize {
        // Remap **then** wrap — the order that lets a wrap reach bank 0. See the module docs.
        self.banks.wrap_rom(Self::adjusted(self.rom_bank))
    }

    fn ram_target(&self) -> RamTarget {
        if !self.ram_enabled {
            return RamTarget::None;
        }
        // The 2-bit register only reaches RAM in mode 1; in mode 0 RAM is stuck on bank 0.
        //
        // ⚠️ **Divergence from gambatte, on purpose.** Its `setRambank` uses `rambank_`
        // regardless of mode and simply never *writes* it outside mode 1, so a cartridge that
        // selects RAM bank 2 in mode 1 and then returns to mode 0 keeps bank 2 there. On hardware
        // the mode bit routes the register, so mode 0 is bank 0. Observable only across a mode
        // switch, which is why it has survived in gambatte; Phase D is scored against mooneye,
        // which tests the hardware, so this follows Pan Docs.
        let bank = if self.ram_bank_mode { self.ram_bank } else { 0 };
        match self.banks.wrap_ram(bank) {
            Some(bank) => RamTarget::Bank(bank),
            None => RamTarget::None,
        }
    }

    fn ram_enabled(&self) -> bool {
        self.ram_enabled
    }

    fn restore_effective(&mut self, rom_bank: usize, ram_bank: usize, ram_enabled: bool) {
        self.rom_bank = rom_bank;
        self.ram_bank = ram_bank;
        self.ram_enabled = ram_enabled;
    }
}

/// **D4.** MBC2: a 4-bit ROM-bank register and 512 nibbles of built-in RAM.
///
/// ⚠️ **Two things here are unlike every other mapper.** The register select is `address & 0x6100`
/// — A8 takes part, so `0x2100` selects the bank register while `0x2000` does nothing at all. And
/// the RAM is on the chip: the header says zero banks, so [`crate::mmu::MMU`] allocates one
/// regardless, and only the low nibble of each byte is real.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Mbc2 {
    banks: BankCounts,
    rom_bank: usize,
    ram_enabled: bool,
}

impl Mbc2 {
    fn new(banks: BankCounts) -> Self {
        Self { banks, rom_bank: 1, ram_enabled: false }
    }
}

impl Mbc for Mbc2 {
    fn rom_write(&mut self, address: u16, value: u8) {
        match address & 0x6100 {
            0x0000 => self.ram_enabled = unlocks_ram(value),
            0x2100 => self.rom_bank = value as usize & 0x0F,
            _ => {}
        }
    }

    fn rom_bank(&self) -> usize {
        // ⚠️ **Divergence from gambatte, on purpose.** Its MBC2 `setRombank` is a bare
        // `rombank_ & (rombanks - 1)` with no bank-0 remap at all, so a zero selection maps bank 0
        // at `0x4000`. Pan Docs is explicit that MBC2 treats 0 as 1, and mooneye's `mbc2` ROMs —
        // Phase D's exit criterion — test the hardware, not gambatte.
        self.banks.wrap_rom(self.rom_bank.max(1))
    }

    fn ram_target(&self) -> RamTarget {
        match (self.ram_enabled, self.banks.wrap_ram(0)) {
            (true, Some(bank)) => RamTarget::Bank(bank),
            _ => RamTarget::None,
        }
    }

    fn ram_enabled(&self) -> bool {
        self.ram_enabled
    }

    fn restore_effective(&mut self, rom_bank: usize, _ram_bank: usize, ram_enabled: bool) {
        self.rom_bank = rom_bank;
        self.ram_enabled = ram_enabled;
    }
}

/// **D5.** MBC3: a 7-bit ROM-bank register, four RAM banks, and — on the `0x0F`/`0x10` cartridge
/// types — a real-time clock whose five registers replace RAM at `0xA000` when `0x08..=0x0C` is
/// selected. The clock itself lives in [`crate::rtc::Rtc`].
///
/// ⭐ **This is Pokémon Red's mapper and therefore the live path.** `pokered.gbc` is `0x13`
/// (MBC3+RAM+battery, *no* timer), 64 banks, 4 RAM banks — so `rtc` is `None` for it and none of
/// the clock code runs.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Mbc3 {
    banks: BankCounts,
    rom_bank: usize,
    /// The raw `0x4000..=0x5FFF` register. Kept whole rather than masked, because `0x08..=0x0C`
    /// selects a clock register rather than a RAM bank.
    ram_bank: usize,
    ram_enabled: bool,
    /// `Some` only for the two cartridge types that declare a timer.
    rtc: Option<Rtc>,
}

impl Mbc3 {
    fn new(banks: BankCounts, has_rtc: bool) -> Self {
        Self {
            banks,
            rom_bank: 1,
            ram_bank: 0,
            ram_enabled: false,
            rtc: has_rtc.then(Rtc::default),
        }
    }
}

impl Mbc for Mbc3 {
    fn rom_write(&mut self, address: u16, value: u8) {
        match region(address) {
            0 => self.ram_enabled = unlocks_ram(value),
            1 => self.rom_bank = value as usize & 0x7F,
            2 => self.ram_bank = value as usize,
            _ => {
                if let Some(rtc) = self.rtc.as_mut() {
                    rtc.write_latch(value);
                }
            }
        }
    }

    fn rom_bank(&self) -> usize {
        // ⚠️ Wrap **then** remap — the opposite order to MBC1, so a wrap can never land on bank 0.
        self.banks.wrap_rom(self.rom_bank).max(1)
    }

    fn ram_target(&self) -> RamTarget {
        if !self.ram_enabled {
            return RamTarget::None;
        }
        if self.rtc.is_some() && (0x08..=0x0C).contains(&self.ram_bank) {
            return RamTarget::Rtc(self.ram_bank as u8);
        }
        match self.banks.wrap_ram(self.ram_bank) {
            Some(bank) => RamTarget::Bank(bank),
            None => RamTarget::None,
        }
    }

    fn ram_enabled(&self) -> bool {
        self.ram_enabled
    }

    fn restore_effective(&mut self, rom_bank: usize, ram_bank: usize, ram_enabled: bool) {
        self.rom_bank = rom_bank;
        self.ram_bank = ram_bank;
        self.ram_enabled = ram_enabled;
    }

    fn rtc(&self) -> Option<&Rtc> {
        self.rtc.as_ref()
    }

    fn rtc_mut(&mut self) -> Option<&mut Rtc> {
        self.rtc.as_mut()
    }
}

/// **D6.** MBC5: a **9-bit** ROM-bank register split across two halves of the `0x2000` range, and
/// a 4-bit RAM-bank register.
///
/// ⚠️ **MBC5 does not remap bank 0**, so a game may legitimately map bank 0 at `0x4000` and see
/// the same 16 KB twice. Every other mapper forces bank 1.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Mbc5 {
    banks: BankCounts,
    rom_bank: usize,
    ram_bank: usize,
    ram_enabled: bool,
}

impl Mbc5 {
    fn new(banks: BankCounts) -> Self {
        Self { banks, rom_bank: 1, ram_bank: 0, ram_enabled: false }
    }
}

impl Mbc for Mbc5 {
    fn rom_write(&mut self, address: u16, value: u8) {
        match region(address) {
            0 => self.ram_enabled = unlocks_ram(value),
            // `0x2000-0x2FFF` writes the low eight bits, `0x3000-0x3FFF` the ninth.
            1 => {
                self.rom_bank = if address < 0x3000 {
                    (self.rom_bank & 0x100) | value as usize
                } else {
                    ((value as usize) << 8 & 0x100) | (self.rom_bank & 0xFF)
                };
            }
            2 => self.ram_bank = value as usize & 0x0F,
            _ => {}
        }
    }

    fn rom_bank(&self) -> usize {
        self.banks.wrap_rom(self.rom_bank)
    }

    fn ram_target(&self) -> RamTarget {
        if !self.ram_enabled {
            return RamTarget::None;
        }
        match self.banks.wrap_ram(self.ram_bank) {
            Some(bank) => RamTarget::Bank(bank),
            None => RamTarget::None,
        }
    }

    fn ram_enabled(&self) -> bool {
        self.ram_enabled
    }

    fn restore_effective(&mut self, rom_bank: usize, ram_bank: usize, ram_enabled: bool) {
        self.rom_bank = rom_bank;
        self.ram_bank = ram_bank;
        self.ram_enabled = ram_enabled;
    }
}

/// **D7.** HuC1: MBC1's shape with an infrared port where the RAM-enable register would be.
///
/// ⚠️ In mode 0 the 2-bit register is shifted **six** places into the ROM bank and *also* kept in
/// the low bits (`bank << 6 | bank`), which is genuinely what gambatte does (`cartridge.cpp`
/// `HuC1::setRombank`) — not a transcription slip. HuC1 cartridges are rare and untested here.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct HuC1 {
    banks: BankCounts,
    rom_bank: usize,
    ram_bank: usize,
    ram_enabled: bool,
    ram_bank_mode: bool,
}

impl HuC1 {
    fn new(banks: BankCounts) -> Self {
        Self { banks, rom_bank: 1, ram_bank: 0, ram_enabled: false, ram_bank_mode: false }
    }
}

impl Mbc for HuC1 {
    fn rom_write(&mut self, address: u16, value: u8) {
        match region(address) {
            0 => self.ram_enabled = unlocks_ram(value),
            1 => self.rom_bank = value as usize & 0x3F,
            2 => self.ram_bank = value as usize & 0x03,
            _ => self.ram_bank_mode = value & 1 != 0,
        }
    }

    fn rom_bank(&self) -> usize {
        let bank = if self.ram_bank_mode {
            self.rom_bank
        } else {
            self.ram_bank << 6 | self.rom_bank
        };
        self.banks.wrap_rom(bank)
    }

    fn ram_target(&self) -> RamTarget {
        if !self.ram_enabled {
            return RamTarget::None;
        }
        let bank = if self.ram_bank_mode { self.ram_bank } else { 0 };
        match self.banks.wrap_ram(bank) {
            Some(bank) => RamTarget::Bank(bank),
            None => RamTarget::None,
        }
    }

    /// ⚠️ **Simplified, and the simplification is a known gap.** On a HuC1 the `0x0000..=0x1FFF`
    /// register switches the *infrared port* in rather than switching RAM out, so gambatte keeps
    /// reads enabled unconditionally and gates only writes. [`Mbc::ram_enabled`] is a single flag
    /// that the MMU applies to both, so it cannot express that; this reports the real flag, which
    /// keeps writes right and makes disabled *reads* return `0xFF` where hardware would return
    /// data. No HuC1 cartridge is committed and nothing exercises it.
    fn ram_enabled(&self) -> bool {
        self.ram_enabled
    }

    fn restore_effective(&mut self, rom_bank: usize, ram_bank: usize, ram_enabled: bool) {
        self.rom_bank = rom_bank;
        self.ram_bank = ram_bank;
        self.ram_enabled = ram_enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banks(rom: usize, ram: usize) -> BankCounts {
        BankCounts { rom, ram }
    }

    /// D1's acceptance trace, now owned by the mapper that actually has it. Blargg's combined
    /// `dmg_sound.gb` is MBC1 with four banks, and its runner reaches its terminator by writing
    /// `4` and landing on bank 0.
    #[test]
    fn mbc1_wraps_a_bank_selection_onto_bank_zero() {
        let mut mbc = Mbc1::new(banks(4, 1));
        for (write, expected) in [(1, 1), (2, 2), (3, 3), (4, 0)] {
            mbc.rom_write(0x2000, write);
            assert_eq!(mbc.rom_bank(), expected, "write of {write}");
        }
    }

    /// ⚠️ The MBC1 hole: the aliasing tests the **low five bits**, so `0x20` is `0x21`, not `0x01`.
    #[test]
    fn mbc1_aliases_bank_zero_of_each_thirty_two() {
        let mut mbc = Mbc1::new(banks(128, 0));
        for (bank2, expected) in [(0, 0x01), (1, 0x21), (2, 0x41), (3, 0x61)] {
            mbc.rom_write(0x4000, bank2); // the high two bits, in mode 0
            mbc.rom_write(0x2000, 0x00); // a zero low-five selection
            assert_eq!(mbc.rom_bank(), expected, "high bits {bank2}");
        }
    }

    /// Mode 1 routes the 2-bit register to RAM instead of ROM.
    #[test]
    fn mbc1_mode_switches_the_two_bit_register_between_rom_and_ram() {
        let mut mbc = Mbc1::new(banks(128, 4));
        mbc.rom_write(0x0000, 0x0A); // enable RAM
        mbc.rom_write(0x2000, 0x05);
        mbc.rom_write(0x4000, 0x02);
        assert_eq!(mbc.rom_bank(), 0x45, "mode 0: the register is ROM bank bits 5-6");
        assert_eq!(mbc.ram_target(), RamTarget::Bank(0), "mode 0: RAM is stuck on bank 0");

        mbc.rom_write(0x6000, 0x01); // mode 1
        mbc.rom_write(0x4000, 0x02);
        assert_eq!(mbc.ram_target(), RamTarget::Bank(2), "mode 1: the register is the RAM bank");
    }

    /// ⚠️ MBC3 applies its bank-0 remap **after** the wrap, so — unlike MBC1 — no selection can
    /// reach bank 0. This is the live Pokémon Red path.
    #[test]
    fn mbc3_can_never_select_bank_zero() {
        let mut mbc = Mbc3::new(banks(64, 4), false);
        for write in [0x00, 0x40, 0x80] {
            mbc.rom_write(0x2000, write);
            assert_eq!(mbc.rom_bank(), 1, "write of {write:#04X} wraps to 0 and is remapped to 1");
        }
        mbc.rom_write(0x2000, 0x3F);
        assert_eq!(mbc.rom_bank(), 0x3F, "seven bits reach the register");
    }

    /// The same write is bank 0 on MBC1 and bank 1 on MBC3. If this ever passes by accident, the
    /// two orders have been collapsed into one.
    #[test]
    fn mbc1_and_mbc3_disagree_about_the_same_write() {
        let mut mbc1 = Mbc1::new(banks(4, 0));
        let mut mbc3 = Mbc3::new(banks(4, 0), false);
        mbc1.rom_write(0x2000, 4);
        mbc3.rom_write(0x2000, 4);
        assert_eq!(mbc1.rom_bank(), 0);
        assert_eq!(mbc3.rom_bank(), 1);
    }

    /// Only a cartridge type that declares a timer exposes the clock registers; on `0x13` — which
    /// is pokered — `0x08` is just another RAM-bank selection, and wraps.
    #[test]
    fn mbc3_clock_registers_need_a_timer_cartridge() {
        let mut without = Mbc3::new(banks(64, 4), false);
        without.rom_write(0x0000, 0x0A);
        without.rom_write(0x4000, 0x08);
        assert_eq!(without.ram_target(), RamTarget::Bank(0), "0x08 wraps onto the four banks");

        let mut with = Mbc3::new(banks(64, 4), true);
        with.rom_write(0x0000, 0x0A);
        with.rom_write(0x4000, 0x08);
        assert_eq!(with.ram_target(), RamTarget::Rtc(0x08));
    }

    /// ⚠️ MBC2 decodes A8, so `0x2000` and `0x2100` do completely different things.
    #[test]
    fn mbc2_decodes_a8() {
        let mut mbc = Mbc2::new(banks(16, 1));
        mbc.rom_write(0x2100, 0x05);
        assert_eq!(mbc.rom_bank(), 5);
        mbc.rom_write(0x2000, 0x07);
        assert_eq!(mbc.rom_bank(), 5, "a write without A8 is not the bank register");

        mbc.rom_write(0x0100, 0x0A);
        assert!(!mbc.ram_enabled(), "...and neither is a RAM-enable write with A8 set");
        mbc.rom_write(0x0000, 0x0A);
        assert!(mbc.ram_enabled());
    }

    /// ⚠️ Divergence from gambatte, deliberate: Pan Docs says MBC2 treats a zero selection as 1,
    /// gambatte has no remap at all. Phase D is scored against mooneye, which tests hardware.
    #[test]
    fn mbc2_remaps_bank_zero() {
        let mut mbc = Mbc2::new(banks(16, 1));
        mbc.rom_write(0x2100, 0x00);
        assert_eq!(mbc.rom_bank(), 1);
    }

    /// ⚠️ The other deliberate divergence: MBC1's mode bit *routes* the 2-bit register, so
    /// returning to mode 0 puts RAM back on bank 0. Gambatte leaves it where mode 1 left it.
    #[test]
    fn mbc1_mode_zero_puts_ram_back_on_bank_zero() {
        let mut mbc = Mbc1::new(banks(4, 4));
        mbc.rom_write(0x0000, 0x0A);
        mbc.rom_write(0x6000, 0x01); // mode 1
        mbc.rom_write(0x4000, 0x02);
        assert_eq!(mbc.ram_target(), RamTarget::Bank(2));

        mbc.rom_write(0x6000, 0x00); // back to mode 0
        assert_eq!(mbc.ram_target(), RamTarget::Bank(0), "the register no longer reaches RAM");
    }

    /// MBC5 assembles nine bits from two ranges, and is the only mapper that leaves bank 0 alone.
    #[test]
    fn mbc5_has_a_nine_bit_register_and_no_bank_zero_remap() {
        let mut mbc = Mbc5::new(banks(512, 0));
        mbc.rom_write(0x2000, 0x00);
        assert_eq!(mbc.rom_bank(), 0, "bank 0 is a legal MBC5 selection");

        mbc.rom_write(0x2000, 0xFF);
        assert_eq!(mbc.rom_bank(), 0xFF);
        mbc.rom_write(0x3000, 0x01); // the ninth bit
        assert_eq!(mbc.rom_bank(), 0x1FF);
        mbc.rom_write(0x2000, 0x00); // low byte only — the ninth bit survives
        assert_eq!(mbc.rom_bank(), 0x100);
    }

    /// A `RomOnly` cartridge has bank 1 nailed down whatever the guest writes.
    #[test]
    fn rom_only_never_switches() {
        let mut mbc = RomOnly::new(banks(2, 0));
        mbc.rom_write(0x2000, 0x05);
        assert_eq!(mbc.rom_bank(), 1);
    }

    /// Disabled RAM is not addressable, on every mapper that has an enable register.
    #[test]
    fn disabled_ram_is_not_addressable() {
        for mut mapper in [
            Mapper::Mbc1(Mbc1::new(banks(4, 4))),
            Mapper::Mbc3(Mbc3::new(banks(4, 4), false)),
            Mapper::Mbc5(Mbc5::new(banks(4, 4))),
        ] {
            assert_eq!(mapper.ram_target(), RamTarget::None, "{mapper:?} starts locked");
            mapper.rom_write(0x0000, 0x0A);
            assert_eq!(mapper.ram_target(), RamTarget::Bank(0));
            mapper.rom_write(0x0000, 0x00);
            assert_eq!(mapper.ram_target(), RamTarget::None);
        }
    }

    /// A cartridge with no RAM at all addresses nothing, however unlocked it claims to be.
    #[test]
    fn a_cartridge_without_ram_addresses_nothing() {
        let mut mbc = Mbc3::new(banks(64, 0), false);
        mbc.rom_write(0x0000, 0x0A);
        assert_eq!(mbc.ram_target(), RamTarget::None);
    }
}
