//! A banked address: which of the cartridge's or the console's banks, and the address inside it.

use std::fmt::Display;
use std::ops::{Add, AddAssign, Sub, SubAssign};

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
