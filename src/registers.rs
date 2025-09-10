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