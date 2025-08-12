use std::time::Duration;
use crate::core::CoreMode;
use crate::header::CartHeader;
use crate::interrupt::{InterruptFlags, InterruptType};
use crate::joypad::JoypadRegister;

const RAM_BANK_SIZE: usize = 0x2000; // 8KB
const ROM_BANK_SIZE: usize = 0x4000; // 16KB

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MMU {
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
    pub fn from_rom(data: &[u8]) -> Result<Self, String> {
        let header = CartHeader::parse(data)?;
        if header.rom_banks() > 0x20 {
            // TODO support mode 1 of MBC1 https://gbdev.io/pandocs/MBC1.html
            return Err("game not supported with greater than 0x20 rom banks".to_string());
        }

        let ram_banks = Vec::from_iter((0..header.ram_banks()).map(|_| [0; RAM_BANK_SIZE]));
        Ok(Self {
            data: data.to_vec(),
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

    pub fn header(&self) -> &CartHeader {
        &self.header
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn joypad_mut(&mut self) -> &mut JoypadRegister {
        &mut self.joypad_register
    }

    /// update internal state of the MMU, should be called every CPU cycle
    pub fn update(&mut self, delta: Duration) {
        // update interrupt register
        for interrupt in InterruptType::all() {
            let interrupt_required = match interrupt {
                InterruptType::Joypad => self.joypad_register.interrupt_required(),
                _ => false // TODO
            };
            if interrupt_required {
                self.interrupt_register.set_interrupt(interrupt);
            }
        }
    }

    pub fn check_interrupts(&mut self, core_mode: CoreMode) -> Option<InterruptType> {
        // check if enabled interrupts in order of priority
        let interrupts_to_check = match core_mode {
            CoreMode::Stop => {
                // In STOP mode, interrupts are not processed, but we still check for JOYPAD interrupts
                vec![InterruptType::Joypad]
            }
            CoreMode::Crash => {
                // In CRASH mode, no interrupts are processed
                vec![]
            }
            _ => InterruptType::all().collect()
        };

        for interrupt in interrupts_to_check {
            if self.interrupt_enable.is_set(interrupt) && self.interrupt_register.is_set(interrupt) {
                self.interrupt_register.clear_interrupt(interrupt);
                return Some(interrupt);
            }
        }
        None
    }

    pub fn read(&self, address: u16) -> u8 {
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
            0xA000..=0xBFFF if self.ram_enabled && self.header.ram_banks() > 0 => {
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

    pub fn read_u16_le(&self, address: u16) -> u16 {
        let low = self.read(address);
        let high = self.read(address + 1);
        u16::from_le_bytes([low, high])
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1FFF => {
                // https://gbdev.io/pandocs/MBC1.html#00001fff--ram-enable-write-only
                self.ram_enabled = value & 0xF == 0xA;
            }
            0x2000..=0x3FFF if self.header.rom_banks() > 2 => {
                // https://gbdev.io/pandocs/MBC1.html#20003fff--rom-bank-number-write-only
                self.rom_bank_register = ((value & 0x1F) as usize)
                    .min(self.header.rom_banks() - 1)
                    .max(1);
            }
            0x4000..=0x5FFF if self.header.ram_banks() > 0 => {
                // https://gbdev.io/pandocs/MBC1.html#40005fff--ram-bank-number--or--upper-bits-of-rom-bank-number-write-only
                self.ram_bank_register = ((value & 0x03) as usize).min(self.header.ram_banks() - 1);
            }
            0xA000..=0xBFFF if self.ram_enabled && self.header.ram_banks() > 0 => {
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

    pub fn write_u16_le(&mut self, address: u16, value: u16) {
        let low = (value & 0xFF) as u8;
        let high = (value >> 8) as u8;
        self.write(address, low);
        self.write(address + 1, high);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmu_enable_ram() {
        let mut mmu = MMU::from_rom(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0x0000, 0xA); // Enable RAM
        assert!(mmu.ram_enabled);
    }

    #[test]
    fn mmu_rom_banks() {
        let mut mmu = MMU::from_rom(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
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
        let mut mmu = MMU::from_rom(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xC000, 0x42); // Write to work RAM
        assert_eq!(mmu.read(0xC000), 0x42);
        mmu.write(0xE000, 0x24); // Write to echo RAM
        assert_eq!(mmu.read(0xE000), 0x24);
        assert_eq!(mmu.read(0xC000), 0x24); // Echo RAM mirrors work RAM
    }

    #[test]
    fn mmu_high_ram() {
        let mut mmu = MMU::from_rom(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xFF80, 0xAB); // Write to high RAM
        assert_eq!(mmu.read(0xFF80), 0xAB);
        mmu.write(0xFFFE, 0xCD); // Write to high RAM
        assert_eq!(mmu.read(0xFFFE), 0xCD);
    }

    #[test]
    fn mmu_interrupt_flags() {
        let mut mmu = MMU::from_rom(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xFF0F, 0x1F); // Set all interrupt flags
        assert_eq!(mmu.interrupt_register.get(), 0x1F);
        mmu.write(0xFF0F, 0x00); // Clear all interrupt flags
        assert_eq!(mmu.interrupt_register.get(), 0x00);
    }

    #[test]
    fn interrupt_enable() {
        let mut mmu = MMU::from_rom(crate::roms::blarg::CPU_INSTRUCTIONS).unwrap();
        mmu.write(0xFFFF, 0x1F); // Enable all interrupts
        assert_eq!(mmu.interrupt_enable.get(), 0x1F);
        mmu.write(0xFFFF, 0x00); // Disable all interrupts
        assert_eq!(mmu.interrupt_enable.get(), 0x00);
    }
}