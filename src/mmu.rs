use bincode::{Decode, Encode};
use crate::activation::Activation;
use crate::audio::Audio;
use crate::core::CoreMode;
use crate::cycles::MachineCycles;
use crate::divider::Divider;
use crate::header::CartHeader;
use crate::interrupt::{InterruptFlags, InterruptType};
use crate::joypad::JoypadRegister;
use crate::pokemon::symbols::{DmgBank, DmgPointer};
use crate::ppu::PPU;
use crate::ram::{RAM, ROM};
use crate::savestate::{labels, SectionReader, SectionWriter};
use crate::serial::Serial;
use crate::timer::Timer;

pub const RAM_BANK_SIZE: usize = 0x2000; // 8KB
pub const ROM_BANK_SIZE: usize = 0x4000; // 16KB

/// Round a ROM image up to a whole power-of-two number of 16 KB banks, filling with `0xFF` — the
/// value an unmapped bus reads back. Gambatte does the same (`cartridge.cpp:638-652`) and for the
/// same reason: it makes every in-range bank index land on real memory, so a cartridge whose
/// header over-states its size can no longer index past the buffer.
fn pad_rom(data: &[u8]) -> Vec<u8> {
    let banks = (data.len().div_ceil(ROM_BANK_SIZE)).max(2).next_power_of_two();
    let mut padded = Vec::with_capacity(banks * ROM_BANK_SIZE);
    padded.extend_from_slice(data);
    padded.resize(banks * ROM_BANK_SIZE, 0xFF);
    padded
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MMU {
    data: Vec<u8>,
    header: CartHeader,
    ram_banks: Vec<[u8; RAM_BANK_SIZE]>,
    ram_enabled: bool,
    rom_bank_register: usize,
    ram_bank_register: usize,
    work_ram: [u8; 0x2000], // 8KB of work RAM (DMG mode only)
    high_ram: [u8; 0x7F], // 128 bytes of high RAM
    ppu: PPU,
    serial: Serial,
    divider: Divider,
    timer: Timer,
    interrupt_enable: InterruptFlags,
    /// Bits 5-7 of IE. Not wired to any interrupt, but hardware still stores and returns them.
    interrupt_enable_upper: u8,
    interrupt_request: InterruptFlags,
    joypad_register: JoypadRegister,
    audio: Audio,
}

/// Contents of the `cart` save-state section: everything that describes the cartridge and its
/// mapper state. The ROM image itself is deliberately absent — it is supplied by the loader.
#[derive(Debug, Clone, Decode, Encode)]
pub struct CartSection {
    pub header: CartHeader,
    pub ram_banks: Vec<[u8; RAM_BANK_SIZE]>,
    pub ram_enabled: bool,
    pub rom_bank_register: usize,
    pub ram_bank_register: usize,
}

/// Contents of the `irq` save-state section.
#[derive(Debug, Clone, Decode, Encode)]
pub struct IrqSection {
    pub interrupt_enable: InterruptFlags,
    pub interrupt_request: InterruptFlags,
}

/// Contents of the `timer` save-state section: everything clocked off the divider, plus serial,
/// which shares its clock domain.
#[derive(Debug, Clone, Decode, Encode)]
pub struct TimerSection {
    pub divider: Divider,
    pub timer: Timer,
    pub serial: Serial,
}

pub const CART_SECTION_VERSION: u16 = 1;
pub const WRAM_SECTION_VERSION: u16 = 1;
pub const HRAM_SECTION_VERSION: u16 = 1;
/// Bumped to 2 by A13, which appended IE's upper three bits.
pub const IRQ_SECTION_VERSION: u16 = 2;
pub const TIMER_SECTION_VERSION: u16 = 1;
pub const JOYP_SECTION_VERSION: u16 = 1;

impl MMU {
    pub(crate) fn write_sections(&self, writer: &mut SectionWriter) -> Result<(), String> {
        writer.write(labels::CART, CART_SECTION_VERSION, &CartSection {
            header: self.header.clone(),
            ram_banks: self.ram_banks.clone(),
            ram_enabled: self.ram_enabled,
            rom_bank_register: self.rom_bank_register,
            ram_bank_register: self.ram_bank_register,
        })?;
        writer.write(labels::WRAM, WRAM_SECTION_VERSION, &self.work_ram)?;
        writer.write(labels::HRAM, HRAM_SECTION_VERSION, &self.high_ram)?;
        writer.write(labels::TIMER, TIMER_SECTION_VERSION, &TimerSection {
            divider: self.divider,
            timer: self.timer.clone(),
            serial: self.serial.clone(),
        })?;
        writer.write_fields(labels::IRQ, IRQ_SECTION_VERSION, |fields| {
            fields.field(&IrqSection {
                interrupt_enable: self.interrupt_enable,
                interrupt_request: self.interrupt_request,
            })?;
            fields.field(&self.interrupt_enable_upper) // appended in v2
        })?;
        writer.write(labels::JOYP, JOYP_SECTION_VERSION, &self.joypad_register)?;
        self.ppu.write_sections(writer)?;
        self.audio.write_sections(writer)
    }

    pub(crate) fn read_sections(&mut self, reader: &SectionReader) -> Result<(), String> {
        if let Some((_version, section)) = reader.read::<CartSection>(labels::CART)? {
            self.header = section.header;
            self.ram_banks = section.ram_banks;
            self.ram_enabled = section.ram_enabled;
            self.rom_bank_register = section.rom_bank_register;
            self.ram_bank_register = section.ram_bank_register;
        }
        if let Some((_version, work_ram)) = reader.read::<[u8; 0x2000]>(labels::WRAM)? {
            self.work_ram = work_ram;
        }
        if let Some((_version, high_ram)) = reader.read::<[u8; 0x7F]>(labels::HRAM)? {
            self.high_ram = high_ram;
        }
        if let Some((_version, section)) = reader.read::<TimerSection>(labels::TIMER)? {
            self.divider = section.divider;
            self.timer = section.timer;
            self.serial = section.serial;
        }
        if let Some(mut fields) = reader.section(labels::IRQ)? {
            if let Some(section) = fields.field::<IrqSection>()? {
                self.interrupt_enable = section.interrupt_enable;
                self.interrupt_request = section.interrupt_request;
            }
            // Absent in v1 payloads, which is not an error — see src/savestate/mod.rs.
            if let Some(upper) = fields.field::<u8>()? {
                self.interrupt_enable_upper = upper;
            }
        }
        if let Some((_version, joypad_register)) = reader.read::<JoypadRegister>(labels::JOYP)? {
            self.joypad_register = joypad_register;
        }
        self.ppu.read_sections(reader)?;
        self.audio.read_sections(reader)
    }

    pub fn high_ram(&self) -> &[u8] {
        &self.high_ram
    }
}

impl MMU {
    pub fn from_rom(data: &[u8]) -> Result<Self, String> {
        let header = CartHeader::parse(data)?;

        println!("{:?}", header);

        let ram_banks = Vec::from_iter((0..header.ram_banks()).map(|_| [0; RAM_BANK_SIZE]));
        Ok(Self {
            data: pad_rom(data),
            header,
            ram_banks,
            ram_enabled: false,
            rom_bank_register: 1,
            ram_bank_register: 0,
            work_ram: [0; 0x2000],
            high_ram: [0; 0x7F],
            ppu: PPU::default(),
            interrupt_enable: InterruptFlags::default(),
            interrupt_enable_upper: 0,
            interrupt_request: InterruptFlags::default(),
            joypad_register: JoypadRegister::default(),
            serial: Serial::default(),
            divider: Divider::default(),
            timer: Timer::default(),
            audio: Audio::default(),
        })
    }

    /// Return to power-on state, **preserving the cartridge and its battery-backed RAM** — the
    /// same contract as gambatte's `GB::reset` (`gambatte.cpp:79-89`). Everything reset here must
    /// match what [`MMU::from_rom`] constructs, or `Core::reset` will not produce a machine equal
    /// to a fresh one.
    pub fn reset(&mut self) {
        self.ram_enabled = false;
        self.rom_bank_register = 1;
        self.ram_bank_register = 0;
        self.work_ram = [0; 0x2000];
        self.high_ram = [0; 0x7F];
        self.ppu = PPU::default();
        self.serial = Serial::default();
        self.divider = Divider::default();
        self.timer = Timer::default();
        self.interrupt_enable = InterruptFlags::default();
        self.interrupt_enable_upper = 0;
        self.interrupt_request = InterruptFlags::default();
        self.joypad_register = JoypadRegister::default();
        self.audio = Audio::default();
        // `data`, `header` and `ram_banks` are deliberately untouched: the cartridge does not
        // leave the slot, and its RAM is battery-backed.
    }

    pub fn header(&self) -> &CartHeader {
        &self.header
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn rom_bank_register(&self) -> usize {
        self.rom_bank_register
    }

    pub fn set_rom_bank_register(&mut self, value: usize) {
        // TODO MBC1 should mask to 0x1F
        // Clamp against the ROM actually loaded, not against header byte 0x148 — a cartridge that
        // claims more banks than its file contains would otherwise index past the buffer and
        // panic on the first high-bank read. D1 replaces this clamp with hardware's wrapping mask.
        self.rom_bank_register = (value & 0x7F)
            .min(self.rom_bank_count() - 1)
            .max(1);
    }

    /// Banks actually backed by data. Derived from the loaded image rather than from header byte
    /// `0x148`, which cartridges are free to lie about. [`MMU::set_data`] keeps the image padded
    /// to a whole power-of-two number of banks, so this is always exact and at least 2.
    pub fn rom_bank_count(&self) -> usize {
        (self.data.len() / ROM_BANK_SIZE).max(2)
    }

    pub fn rom_data<L: Into<Option<usize>>>(&self, bank: usize, index: usize, length: L) -> &[u8] {
        let start = bank * ROM_BANK_SIZE + index;
        if let Some(length) = length.into() {
            let end = start + length;
            self.data.get(start..end)
                .unwrap_or_else(|| panic!("ROM slice out of bounds: bank={} index={} length={}", bank, index, length))
        } else {
            self.data.get(start..).unwrap_or_else(|| panic!("ROM slice out of bounds: bank={} index={}", bank, index))
        }
    }

    pub fn rom_data_from_rom_pointer<L: Into<Option<usize>>>(&self, pointer: &DmgPointer, length: L) -> &[u8] {
        if let DmgPointer { bank: DmgBank::ROM { bank }, address } = pointer {
            self.rom_data_from_pointer(*bank as usize, *address, length)
        } else {
            panic!("Pointer {} is not a ROM pointer", pointer)
        }
    }

    pub fn rom_data_from_pointer<L: Into<Option<usize>>>(&self, bank: usize, pointer: u16, length: L) -> &[u8] {
        if bank == 0 || pointer < ROM_BANK_SIZE as u16 {
            assert!(
                pointer < ROM_BANK_SIZE as u16,
                "Pointer {:04X} is invalid for bank {}", pointer, bank
            );
            // bank 0 or a raw offset into rom bank > 0
            self.rom_data(bank, pointer as usize, length)
        } else if pointer >= ROM_BANK_SIZE as u16 && pointer < ROM_BANK_SIZE as u16 * 2 {
            // correct for raw MMU address
            self.rom_data(bank, pointer as usize - ROM_BANK_SIZE, length)
        } else {
            panic!("Pointer {:04X} is invalid for bank {}", pointer, bank)
        }
    }

    pub fn read_vram_slice<L : Into<Option<usize>>>(&self, address: u16, length: L) -> Result<&[u8], String> {
        if address >= 0x8000 && address <= 0x9FFF {
            let offset = (address - 0x8000) as usize;
            let vram = self.ppu.vram();
            if let Some(length) = length.into() {
                // The base address is range-checked above, but the *end* was not — a long read
                // near 0x9FFF sliced past the array and panicked.
                vram.get(offset..(offset + length))
                    .ok_or_else(|| format!("VRAM read of {length} bytes at {address:04X} runs past the end of VRAM"))
            } else {
                Ok(&vram[offset..])
            }
        } else {
            Err(format!("Address {:04X} is invalid for VRAM", address))
        }
    }

    pub fn read_wram_slice<L : Into<Option<usize>>>(&self, address: u16, length: L) -> Result<&[u8], String> {
        if address >= 0xC000 && address <= 0xDFFF {
            let offset = (address - 0xC000) as usize;
            if let Some(length) = length.into() {
                self.work_ram.get(offset..(offset + length))
                    .ok_or_else(|| format!("WRAM read of {length} bytes at {address:04X} runs past the end of work RAM"))
            } else {
                Ok(&self.work_ram[offset..])
            }
        } else {
            Err(format!("Address {:04X} is invalid for WRAM", address))
        }
    }

    pub fn dump_sram(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.ram_banks.len() * RAM_BANK_SIZE);
        for bank in &self.ram_banks {
            data.extend_from_slice(bank);
        }
        data
    }

    pub fn restore_sram(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != self.ram_banks.len() * RAM_BANK_SIZE {
            Err(format!("Cannot restore SRAM, expected {} bytes, got {}", self.ram_banks.len() * RAM_BANK_SIZE, data.len()))
        } else {
            for (bank, chunk) in self.ram_banks.iter_mut().zip(data.chunks_exact(RAM_BANK_SIZE)) {
                bank.copy_from_slice(chunk);
            }
            Ok(())
        }
    }

    pub fn work_ram(&self) -> &[u8] {
        &self.work_ram
    }

    /// replace rom data, only intended for reloading save states without rom data
    pub fn set_data(&mut self, data: &[u8]) {
        self.data = pad_rom(data);
    }

    pub fn joypad(&self) -> &JoypadRegister {
        &self.joypad_register
    }

    pub fn joypad_mut(&mut self) -> &mut JoypadRegister {
        &mut self.joypad_register
    }

    pub fn ppu(&self) -> &PPU {
        &self.ppu
    }

    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    pub fn audio_mut(&mut self) -> &mut Audio {
        &mut self.audio
    }

    pub fn divider(&self) -> &Divider {
        &self.divider
    }

    pub fn serial(&self) -> &Serial {
        &self.serial
    }

    pub fn serial_mut(&mut self) -> &mut Serial {
        &mut self.serial
    }

    pub fn stop(&mut self) {
        self.divider.disable();
        self.timer.disable();
    }

    pub fn restart(&mut self) {
        self.divider.enable();
        self.timer.enable();
    }

    /// update internal state of the MMU, should be called every CPU cycle
    pub fn update(&mut self, delta_machine_cycles: MachineCycles) {
        if delta_machine_cycles == MachineCycles::ZERO {
            return; // no cycles to update
        }

        if let Some(transfer) = self.ppu.dma_mut().update(delta_machine_cycles) {
            // The DMA controller owns the OAM bus, so it writes through the privileged path
            // rather than the CPU-facing, mode-gated one.
            for (source, offset) in transfer.bytes() {
                let value = self.read(source);
                self.ppu.write_oam_dma(offset, value);
            }
        }


        self.serial.update(delta_machine_cycles);
        let div_clocks = self.divider.update(delta_machine_cycles);
        self.timer.update(delta_machine_cycles);
        self.ppu.update(delta_machine_cycles);
        self.audio.update(delta_machine_cycles, div_clocks);

        // consume pending, an interrupt is triggered on a rising edge
        for interrupt in InterruptType::all() {
            let interrupt_pending = match interrupt {
                InterruptType::Joypad => self.joypad_register.consume_pending_activation(),
                InterruptType::LcdStatus => self.ppu.lcd_status_mut().consume_pending_activation(),
                InterruptType::VBlank => self.ppu.consume_pending_activation(),
                InterruptType::Serial => self.serial.consume_pending_activation(),
                InterruptType::Timer => self.timer.consume_pending_activation(),
            };
            if interrupt_pending {
                self.interrupt_request.set_interrupt(interrupt);
            }
        }
    }

    pub fn interrupt_pending(&self) -> Option<InterruptType> {
        for interrupt in InterruptType::all() {
            if self.interrupt_enable.is_set(interrupt) && self.interrupt_request.is_set(interrupt) {
                return Some(interrupt);
            }
        }
        None
    }

    pub fn clear_interrupt_request(&mut self, interrupt: InterruptType) {
        self.interrupt_request.clear_interrupt(interrupt);
    }

    pub fn check_interrupts(&mut self, interrupt_master_enable: bool, core_mode: CoreMode) -> Option<InterruptType> {
        if !interrupt_master_enable || core_mode == CoreMode::Crash {
            return None;
        }

        // check if enabled interrupts in order of priority
        for interrupt in InterruptType::all() {
            if core_mode == CoreMode::Stop && interrupt != InterruptType::Joypad {
                continue; // In STOP mode, only JOYPAD interrupts are checked
            }

            if self.interrupt_enable.is_set(interrupt) && self.interrupt_request.is_set(interrupt) {
                self.interrupt_request.clear_interrupt(interrupt);
                return Some(interrupt);
            }
        }
        None
    }
}

impl ROM for MMU {
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
            // vram
            0x8000..=0x9FFF => self.ppu.read_vram(address - 0x8000),
            // external ram
            0xA000..=0xBFFF if self.ram_enabled && self.header.ram_banks() > 0 => {
                // https://gbdev.io/pandocs/MBC1.html#a000bfff--ram-bank-0003-if-any
                let ram_bank = &self.ram_banks[self.ram_bank_register];
                ram_bank[(address - 0xA000) as usize]
            }
            0xC000..=0xDFFF => self.work_ram[(address - 0xC000) as usize], // work ram
            0xE000..=0xFDFF => self.work_ram[(address - 0xE000) as usize], // echo ram
            0xFE00..=0xFE9F => self.ppu.read_oam(address - 0xFE00), // OAM (Object Attribute Memory)
            // The unusable region. DMG returns 0x00 here, not 0xFF — settled by gambatte's
            // committed hardware dump `test/hwtests/fexx_ffxx_dumper_dmg08.bin`, which is all
            // zeros at offsets 0xA0..0xFF. CGB differs; revisit in B9.
            0xFEA0..=0xFEFF => 0x00,
            0xFF00 => 0xC0 | self.joypad_register.get(), // joypad register — bits 6-7 unused, read 1
            0xFF01 => self.serial.get_data(), // serial data register
            0xFF02 => 0x7E | self.serial.control(), // serial control register — bits 1-6 read 1
            0xFF04 => self.divider.value(), // DIV register
            0xFF05 => self.timer.value(), // TIMA register
            0xFF06 => self.timer.modulo(), // TMA register
            0xFF07 => 0xF8 | self.timer.control(), // TAC register — bits 3-7 read 1
            0xFF0F => 0xE0 | self.interrupt_request.get(), // IF register — bits 5-7 read 1
            0xFF10..=0xFF3F => self.audio.read(address),
            0xFF40 => self.ppu.lcd_control().get(), // LCD control register
            0xFF41 => 0x80 | self.ppu.lcd_status().stat(), // LCD status register — bit 7 always 1
            0xFF42 => self.ppu.scroll().y, // SCY register
            0xFF43 => self.ppu.scroll().x, // SCX register
            0xFF44 => self.ppu.lcd_status().ly(), // LY register (read-only)
            0xFF45 => self.ppu.lcd_status().lyc(), // LYC register
            0xFF46 => self.ppu.dma().register(), // DMA register reads back the last value written
            0xFF47 => self.ppu.palette().background().to_byte(), // BGP register
            0xFF48 => self.ppu.palette().object0().to_byte(), // OBP0 register
            0xFF49 => self.ppu.palette().object1().to_byte(), // OBP1 register
            0xFF4A => self.ppu.window_position().y, // WY register
            0xFF4B => self.ppu.window_position().x, // WX register
            0xFF80..=0xFFFE => self.high_ram[(address - 0xFF80) as usize], // high ram
            // IE is fully readable and writable, including its top three bits, which are not
            // wired to any interrupt but still hold what was written.
            0xFFFF => self.interrupt_enable.get() | self.interrupt_enable_upper,
            _ => {
                // ignore
                0xFF
            }
        }
    }
}

impl RAM for MMU {
    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1FFF => {
                // https://gbdev.io/pandocs/MBC1.html#00001fff--ram-enable-write-only
                self.ram_enabled = value & 0xF == 0xA;
            }
            0x2000..=0x3FFF if self.rom_bank_count() > 2 => {
                // https://gbdev.io/pandocs/MBC1.html#20003fff--rom-bank-number-write-only
                self.set_rom_bank_register(value as usize);
            }
            0x4000..=0x5FFF if self.header.ram_banks() > 0 => {
                // https://gbdev.io/pandocs/MBC1.html#40005fff--ram-bank-number--or--upper-bits-of-rom-bank-number-write-only
                self.ram_bank_register = ((value & 0x03) as usize).min(self.header.ram_banks() - 1);
            }
            // vram
            0x8000..=0x9FFF => self.ppu.write_vram(address - 0x8000, value),
            0xA000..=0xBFFF if self.ram_enabled && self.header.ram_banks() > 0 => {
                let ram_bank = &mut self.ram_banks[self.ram_bank_register];
                ram_bank[(address - 0xA000) as usize] = value;
            }
            0xC000..=0xDFFF => self.work_ram[(address - 0xC000) as usize] = value, // work ram
            0xE000..=0xFDFF => self.work_ram[(address - 0xE000) as usize] = value, // echo ram
            0xFE00..=0xFE9F => self.ppu.write_oam(address - 0xFE00, value), // OAM (Object Attribute Memory)
            0xFF00 => self.joypad_register.set(value),
            0xFF01 => self.serial.set_data(value), // serial data register
            0xFF02 => self.serial.set_control(value), // serial control register
            0xFF04 => self.divider.reset(), // DIV register (reset on write)
            0xFF05 => self.timer.set_value(value), // TIMA register
            0xFF06 => self.timer.set_modulo(value), // TMA register
            0xFF07 => self.timer.set_control(value), // TAC register
            0xFF0F => self.interrupt_request.set(value), // IF register (interrupt request flags)
            0xFF10..=0xFF3F => self.audio.write(address, value),
            0xFF40 => self.ppu.lcd_control_mut().set(value), // LCD control register
            0xFF41 => self.ppu.lcd_status_mut().set_stat(value), // LCD status register
            0xFF42 => self.ppu.scroll_mut().y = value, // SCY register
            0xFF43 => self.ppu.scroll_mut().x = value, // SCX register
            0xFF44 => {} // LY register is read-only, writing to it has no effect
            0xFF45 => self.ppu.lcd_status_mut().set_lyc(value), // LYC register
            0xFF46 => self.ppu.dma_mut().set(value), // DMA register (write-only)
            0xFF47 => self.ppu.palette_mut().background_mut().set_from_byte(value), // BGP register
            0xFF48 => self.ppu.palette_mut().object0_mut().set_from_byte(value), // OBP0 register
            0xFF49 => self.ppu.palette_mut().object1_mut().set_from_byte(value), // OBP1 register
            0xFF4A => self.ppu.window_position_mut().y = value, // WY register
            0xFF4B => self.ppu.window_position_mut().x = value, // WX register
            0xFF80..=0xFFFE => self.high_ram[(address - 0xFF80) as usize] = value, // high ram
            0xFFFF => {
                self.interrupt_enable.set(value);
                self.interrupt_enable_upper = value & 0xE0;
            }
            _ => {
                // ignore
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::roms::blargg_cpu::ROM;
    use super::*;

    /// A13: unused register bits read as 1 on hardware. `gb` returned them as 0, so guest code
    /// testing them saw the wrong answer.
    #[test]
    fn unused_io_bits_read_as_one() {
        let mut mmu = MMU::from_rom(crate::roms::blargg_cpu::ROM).unwrap();

        mmu.write(0xFF02, 0x00);
        assert_eq!(mmu.read(0xFF02) & 0x7E, 0x7E, "SC bits 1-6");

        mmu.write(0xFF07, 0x00);
        assert_eq!(mmu.read(0xFF07) & 0xF8, 0xF8, "TAC bits 3-7");

        mmu.write(0xFF0F, 0x00);
        assert_eq!(mmu.read(0xFF0F) & 0xE0, 0xE0, "IF bits 5-7");

        assert_eq!(mmu.read(0xFF41) & 0x80, 0x80, "STAT bit 7");
        assert_eq!(mmu.read(0xFF00) & 0xC0, 0xC0, "P1 bits 6-7");
    }

    /// IE is unusual: all eight bits are readable and writable, even the three that are not wired
    /// to an interrupt.
    #[test]
    fn interrupt_enable_keeps_its_upper_bits() {
        let mut mmu = MMU::from_rom(crate::roms::blargg_cpu::ROM).unwrap();

        mmu.write(0xFFFF, 0xFF);
        assert_eq!(mmu.read(0xFFFF), 0xFF);

        mmu.write(0xFFFF, 0xE0); // only the unwired bits
        assert_eq!(mmu.read(0xFFFF), 0xE0);

        mmu.write(0xFFFF, 0x00);
        assert_eq!(mmu.read(0xFFFF), 0x00);
    }

    /// The unusable region reads 0x00 on DMG, not 0xFF — gambatte's committed hardware dump
    /// `test/hwtests/fexx_ffxx_dumper_dmg08.bin` is all zeros across 0xA0..0xFF.
    #[test]
    fn unusable_region_reads_zero_on_dmg() {
        let mmu = MMU::from_rom(crate::roms::blargg_cpu::ROM).unwrap();
        for address in 0xFEA0..=0xFEFF {
            assert_eq!(mmu.read(address), 0x00, "{address:04X}");
        }
    }

    /// A7: the transfer used to run through the mode-gated `write_oam` using the PPU mode from
    /// the *previous* step, so a DMA started while the PPU was in mode 2 or 3 had all 160 bytes
    /// silently discarded. Pokémon Red only DMAs during VBlank, which is why it never surfaced.
    #[test]
    fn oam_dma_delivers_during_mode_3() {
        use crate::lcd_status::LcdMode;

        let mut mmu = MMU::from_rom(crate::roms::blargg_cpu::ROM).unwrap();

        // Source pattern in work RAM.
        for i in 0..0xA0u16 {
            mmu.write(0xC000 + i, (i as u8).wrapping_mul(3).wrapping_add(1));
        }
        // Put the PPU in the most restrictive mode there is.
        mmu.ppu.lcd_status_mut().set_mode(LcdMode::Drawing);

        mmu.write(0xFF46, 0xC0);
        assert!(mmu.ppu.dma().is_active());
        assert_eq!(mmu.read(0xFF46), 0xC0, "FF46 should read back");

        // OAM is unreadable by the CPU while the controller holds the bus.
        assert_eq!(mmu.read(0xFE00), 0xFF);

        for _ in 0..0xA0 {
            mmu.update(MachineCycles::ONE);
        }

        assert!(!mmu.ppu.dma().is_active(), "the transfer should have finished");
        let expected: Vec<u8> = (0..0xA0u16).map(|i| (i as u8).wrapping_mul(3).wrapping_add(1)).collect();
        assert_eq!(mmu.ppu.oam(), expected.as_slice(), "OAM did not receive the transfer");
    }

    /// The gate on VRAM/OAM used to be `|| dma.is_active()`, which made them *more* accessible
    /// during a transfer — the opposite of hardware.
    #[test]
    fn oam_dma_blocks_cpu_access_rather_than_granting_it() {
        use crate::lcd_status::LcdMode;

        let mut mmu = MMU::from_rom(crate::roms::blargg_cpu::ROM).unwrap();
        mmu.ppu.lcd_status_mut().set_mode(LcdMode::HBlank); // OAM normally accessible here
        assert_ne!(mmu.read(0xFE00), 0xFF, "sanity: OAM is readable outside a transfer");

        mmu.write(0xFF46, 0xC0);
        assert_eq!(mmu.read(0xFE00), 0xFF, "OAM must be blocked during a transfer");

        // ...and CPU writes are dropped rather than corrupting the transfer.
        mmu.write(0xFE00, 0x42);
        assert_eq!(mmu.ppu.oam()[0], 0x00);
    }

    /// A5: `set_rom_bank_register` used to clamp against header byte `0x148`, not against the
    /// bytes actually loaded. A cartridge claiming 64 banks with 32 KB on disk hard-panicked on
    /// the first high-bank read.
    #[test]
    fn truncated_rom_does_not_panic() {
        let full = crate::pokemon::roms::POKERED;
        assert_eq!(full[0x148], 0x05, "pokered's header claims 64 banks");

        // Same header, a thirty-second of the data.
        let truncated = &full[..2 * ROM_BANK_SIZE];
        let mut mmu = MMU::from_rom(truncated).unwrap();
        assert_eq!(mmu.header().rom_banks(), 64, "the header still lies");
        assert_eq!(mmu.rom_bank_count(), 2, "but the bank count follows the data");

        mmu.set_rom_bank_register(63);
        for address in (0x4000..=0x7FFF).step_by(0x400) {
            let _ = mmu.read(address); // must not panic
        }
    }

    /// A ROM that is not a whole power-of-two number of banks is padded with `0xFF`, so every
    /// in-range bank index lands on real memory.
    #[test]
    fn rom_is_padded_to_a_power_of_two() {
        let full = crate::pokemon::roms::POKERED;
        // Two and a half banks -> padded up to four.
        let ragged = &full[..2 * ROM_BANK_SIZE + ROM_BANK_SIZE / 2];
        let mut mmu = MMU::from_rom(ragged).unwrap();
        assert_eq!(mmu.rom_bank_count(), 4);

        // The tail of the half-filled bank, and the two banks beyond it, read as 0xFF.
        mmu.set_rom_bank_register(2);
        assert_eq!(mmu.read(0x4000 + ROM_BANK_SIZE as u16 / 2), 0xFF);
        mmu.set_rom_bank_register(3);
        assert_eq!(mmu.read(0x4000), 0xFF);

        // Real data is untouched.
        assert_eq!(mmu.read(0x0100), full[0x0100]);
    }

    #[test]
    fn mmu_enable_ram() {
        let mut mmu = MMU::from_rom(ROM).unwrap();
        mmu.write(0x0000, 0xA); // Enable RAM
        assert!(mmu.ram_enabled);
    }

    #[test]
    fn mmu_rom_banks() {
        let mut mmu = MMU::from_rom(ROM).unwrap();
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
        let mut mmu = MMU::from_rom(ROM).unwrap();
        mmu.write(0xC000, 0x42); // Write to work RAM
        assert_eq!(mmu.read(0xC000), 0x42);
        mmu.write(0xE000, 0x24); // Write to echo RAM
        assert_eq!(mmu.read(0xE000), 0x24);
        assert_eq!(mmu.read(0xC000), 0x24); // Echo RAM mirrors work RAM
    }

    #[test]
    fn mmu_high_ram() {
        let mut mmu = MMU::from_rom(ROM).unwrap();
        mmu.write(0xFF80, 0xAB); // Write to high RAM
        assert_eq!(mmu.read(0xFF80), 0xAB);
        mmu.write(0xFFFE, 0xCD); // Write to high RAM
        assert_eq!(mmu.read(0xFFFE), 0xCD);
    }

    #[test]
    fn mmu_interrupt_flags() {
        let mut mmu = MMU::from_rom(ROM).unwrap();
        mmu.write(0xFF0F, 0x1F); // Set all interrupt flags
        assert_eq!(mmu.interrupt_request.get(), 0x1F);
        mmu.write(0xFF0F, 0x00); // Clear all interrupt flags
        assert_eq!(mmu.interrupt_request.get(), 0x00);
    }

    #[test]
    fn interrupt_enable() {
        let mut mmu = MMU::from_rom(ROM).unwrap();
        mmu.write(0xFFFF, 0x1F); // Enable all interrupts
        assert_eq!(mmu.interrupt_enable.get(), 0x1F);
        mmu.write(0xFFFF, 0x00); // Disable all interrupts
        assert_eq!(mmu.interrupt_enable.get(), 0x00);
    }
}