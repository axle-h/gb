use std::ops::{Deref, DerefMut};
use crate::memory::MemoryInterface;

const RAM_BANK_SIZE: usize = 0x2000; // 8KB
const ROM_BANK_SIZE: usize = 0x4000; // 16KB

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
}

/// https://gbdev.io/pandocs/Interrupts.html#ffff--ie-interrupt-enable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InterruptFlags {
    joypad: bool,
    serial: bool,
    timer: bool,
    lcd_stat: bool,
    v_blank: bool,
}

impl InterruptFlags {
    pub fn new() -> Self {
        Self {
            joypad: false,
            serial: false,
            timer: false,
            lcd_stat: false,
            v_blank: false,
        }
    }

    pub fn set(&mut self, value: u8) {
        self.joypad = (value & 0x10) != 0;
        self.serial = (value & 0x08) != 0;
        self.timer = (value & 0x04) != 0;
        self.lcd_stat = (value & 0x02) != 0;
        self.v_blank = (value & 0x01) != 0;
    }

    pub fn get(&self) -> u8 {
        let mut value = 0;
        if self.joypad { value |= 0x10; }
        if self.serial { value |= 0x08; }
        if self.timer { value |= 0x04; }
        if self.lcd_stat { value |= 0x02; }
        if self.v_blank { value |= 0x01; }
        value
    }
}

/// https://gbdev.io/pandocs/Joypad_Input.html#ff00--p1joyp-joypad
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JoypadRegister {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    a: bool,
    b: bool,
    select: bool,
    start: bool,
    select_buttons: bool,
    select_directions: bool,
}

impl JoypadRegister {
    pub fn new() -> Self {
        Self {
            up: false,
            down: false,
            left: false,
            right: false,
            a: false,
            b: false,
            select: false,
            start: false,
            select_buttons: false,
            select_directions: false,
        }
    }

    pub fn set(&mut self, value: u8) {
        self.select_buttons = (value & 0x20) != 0;
        self.select_directions = (value & 0x10) != 0;
    }

    pub fn get(&self) -> u8 {
        let button_bits = if self.select_buttons {
            (self.a as u8) | ((self.b as u8) << 1) | ((self.select as u8) << 2) | ((self.start as u8) << 3)
        } else { 0 };

        let direction_bits = if self.select_directions {
            (self.up as u8) | ((self.down as u8) << 1) | ((self.left as u8) << 2) | ((self.right as u8) << 3)
        } else { 0 };

        let value = button_bits | direction_bits;

        // Button pressed = bit is 0, so invert the lower 4 bits
        (!value & 0xF) | (self.select_buttons as u8) << 5 | (self.select_directions as u8) << 4
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MMU {
    data: Vec<u8>,
    header: CartHeader,
    ram_banks: Vec<[u8; RAM_BANK_SIZE]>,
    ram_enabled: bool,
    rom_bank_register: usize,
    ram_bank_register: usize,
    work_ram: [u8; 0x2000], // 8KB of work RAM (DMG mode only)
    high_ram: [u8; 0x7F], // 128 bytes of high RAM
    interrupt_enable: InterruptFlags,
    interrupt_register: InterruptFlags,
    joypad_register: JoypadRegister,
}

impl MMU {
    pub fn new(data: Vec<u8>) -> Result<Self, String> {
        let header = CartHeader::parse(&data)?;
        if header.rom_banks > 0x20 {
            // TODO support mode 1 of MBC1 https://gbdev.io/pandocs/MBC1.html
            return Err("game not supported with greater than 0x20 rom banks".to_string());
        }

        let ram_banks = Vec::from_iter((0..header.ram_banks).map(|_| [0; RAM_BANK_SIZE]));
        Ok(Self {
            data,
            header,
            ram_banks,
            ram_enabled: false,
            rom_bank_register: 1,
            ram_bank_register: 0,
            work_ram: [0; 0x2000],
            high_ram: [0; 0x7F],
            interrupt_enable: InterruptFlags::new(),
            interrupt_register: InterruptFlags::new(),
            joypad_register: JoypadRegister::new(),
        })
    }

    pub fn from_slice(data: &[u8]) -> Result<Self, String> {
        Self::new(data.to_vec())
    }

    pub fn header(&self) -> &CartHeader {
        &self.header
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn read(&self, address: u16) -> u8 {
        // https://gbdev.io/pandocs/Memory_Map.html
        match address {
            // rom bank 0
            0x0000..=0x3FFF => {
                // https://gbdev.io/pandocs/MBC1.html#00003fff--rom-bank-x0-read-only
                self.data[address as usize]
            }
            // rom bank 1-n
            0x4000..=0x7FFF => {
                // https://gbdev.io/pandocs/MBC1.html#40007fff--rom-bank-01-7f-read-only
                let bank_offset = self.rom_bank_register * ROM_BANK_SIZE;
                self.data[bank_offset + (address - 0x4000) as usize]
            }
            // external ram
            0xA000..=0xBFFF if self.ram_enabled && self.header.ram_banks > 0 => {
                // https://gbdev.io/pandocs/MBC1.html#a000bfff--ram-bank-0003-if-any
                let ram_bank = &self.ram_banks[self.ram_bank_register];
                ram_bank[(address - 0xA000) as usize]
            }
            0xC000..=0xDFFF => self.work_ram[(address - 0xC000) as usize], // work ram
            0xE000..=0xFDFF => self.work_ram[(address - 0xE000) as usize], // echo ram
            0xFF00 => self.joypad_register.get(), // joypad register
            0xFF0F => self.interrupt_register.get(), // interrupt flags
            0xFF80..=0xFFFE => self.high_ram[(address - 0xFF80) as usize], // high ram
            0xFFFF => self.interrupt_enable.get(),
            _ => {
                // ignore
                0
            }
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1FFF => {
                // https://gbdev.io/pandocs/MBC1.html#00001fff--ram-enable-write-only
                self.ram_enabled = value & 0xF == 0xA;
            }
            0x2000..=0x3FFF if self.header.rom_banks > 2 => {
                // https://gbdev.io/pandocs/MBC1.html#20003fff--rom-bank-number-write-only
                self.rom_bank_register = ((value & 0x1F) as usize)
                    .min(self.header.rom_banks - 1)
                    .max(1);
            }
            0x4000..=0x5FFF if self.header.ram_banks > 0 => {
                // https://gbdev.io/pandocs/MBC1.html#40005fff--ram-bank-number--or--upper-bits-of-rom-bank-number-write-only
                self.ram_bank_register = ((value & 0x03) as usize).min(self.header.ram_banks - 1);
            }
            0xA000..=0xBFFF if self.ram_enabled && self.header.ram_banks > 0 => {
                let ram_bank = &mut self.ram_banks[self.ram_bank_register];
                ram_bank[(address - 0xA000) as usize] = value;
            }
            0xC000..=0xDFFF => self.work_ram[(address - 0xC000) as usize] = value, // work ram
            0xE000..=0xFDFF => self.work_ram[(address - 0xE000) as usize] = value, // echo ram
            0xFF00 => self.joypad_register.set(value),
            0xFF0F => self.interrupt_register.set(value),
            0xFF80..=0xFFFE => self.high_ram[(address - 0xFF80) as usize] = value, // high ram
            0xFFFF => self.interrupt_enable.set(value),
            _ => {
                // ignore
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cart_header_parse() {
        let header = CartHeader::parse(crate::roms::blarg::CPU_INSTRUCTIONS)
            .expect("Failed to parse CPU_INSTRS header");
        assert_eq!(header.title, "CPU_INSTRS");
        assert_eq!(header.cgb_mode, CGBMode::Enhanced);
        assert_eq!(header.cart_type, CartType::MBC1);
        assert_eq!(header.rom_banks, 4); // 64KB ROM
        assert_eq!(header.ram_banks, 0); // No RAM

        let header = CartHeader::parse(crate::roms::commercial::TETRIS)
            .expect("Failed to parse TETRIS header");
        assert_eq!(header.title, "TETRIS");
        assert_eq!(header.cgb_mode, CGBMode::None);
        assert_eq!(header.cart_type, CartType::RomOnly);
        assert_eq!(header.rom_banks, 2); // 32KB ROM
        assert_eq!(header.ram_banks, 0); // No RAM
    }

    #[test]
    fn mmu_enable_ram() {
        let mut mmu = MMU::from_slice(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0x0000, 0xA); // Enable RAM
        assert!(mmu.ram_enabled);
    }

    #[test]
    fn mmu_rom_banks() {
        let mut mmu = MMU::from_slice(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        assert_eq!(mmu.read(0x0101), 0xC3); // Read from ROM bank 0, should be a JP instruction
        mmu.write(0x2000, 0x01);
        assert_eq!(mmu.rom_bank_register, 1);
        mmu.write(0x2000, 0x00); // ROM bank 1 cannot be mapped to ROM bank 0
        assert_eq!(mmu.rom_bank_register, 1);
        assert_eq!(mmu.read(0x4244), 0x5D); // read from ROM bank 1
        mmu.write(0x2000, 0x02); // switch to ROM bank 2
        assert_eq!(mmu.rom_bank_register, 2);
        assert_eq!(mmu.read(0x4244), 0xBE); // read from ROM bank 2, different to rom bank 1
    }

    #[test]
    fn mmu_work_ram() {
        let mut mmu = MMU::from_slice(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xC000, 0x42); // Write to work RAM
        assert_eq!(mmu.read(0xC000), 0x42);
        mmu.write(0xE000, 0x24); // Write to echo RAM
        assert_eq!(mmu.read(0xE000), 0x24);
        assert_eq!(mmu.read(0xC000), 0x24); // Echo RAM mirrors work RAM
    }

    #[test]
    fn mmu_high_ram() {
        let mut mmu = MMU::from_slice(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xFF80, 0xAB); // Write to high RAM
        assert_eq!(mmu.read(0xFF80), 0xAB);
        mmu.write(0xFFFE, 0xCD); // Write to high RAM
        assert_eq!(mmu.read(0xFFFE), 0xCD);
    }

    #[test]
    fn mmu_interrupt_flags() {
        let mut mmu = MMU::from_slice(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xFF0F, 0x1F); // Set all interrupt flags
        assert_eq!(mmu.interrupt_register.get(), 0x1F);
        mmu.write(0xFF0F, 0x00); // Clear all interrupt flags
        assert_eq!(mmu.interrupt_register.get(), 0x00);
    }

    #[test]
    fn interrupt_enable() {
        let mut mmu = MMU::from_slice(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xFFFF, 0x1F); // Enable all interrupts
        assert_eq!(mmu.interrupt_enable.get(), 0x1F);
        mmu.write(0xFFFF, 0x00); // Disable all interrupts
        assert_eq!(mmu.interrupt_enable.get(), 0x00);
    }

    #[test]
    fn interrupt_flags() {
        let mut flags = InterruptFlags::new();
        assert_eq!(flags.get(), 0x00); // No flags set
        flags.set(0x10);
        assert!(flags.joypad);
        flags.set(0x08);
        assert!(flags.serial);
        flags.set(0x04);
        assert!(flags.timer);
        flags.set(0x02);
        assert!(flags.lcd_stat);
        flags.set(0x01);
        assert!(flags.v_blank);
        flags.set(0x1F);
        assert_eq!(flags.get(), 0x1F); // All flags set
    }

    #[test]
    fn joypad_register() {
        let mut joypad = JoypadRegister::new();
        assert_eq!(joypad.get(), 0xF); // All buttons released
        joypad.set(0x20); // Select buttons
        assert_eq!(joypad.get(), 0x2F);
        joypad.a = true;
        joypad.b = true;
        joypad.select = true;
        joypad.start = true;
        assert_eq!(joypad.get(), 0x20);

        joypad.set(0x10); // Select directions
        assert_eq!(joypad.get(), 0x1F);
        joypad.up = true;
        joypad.down = true;
        joypad.left = true;
        joypad.right = true;
        assert_eq!(joypad.get(), 0x10); // All directions pressed
    }
}