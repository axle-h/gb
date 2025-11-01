use bincode::{BorrowDecode, Decode, Encode};
use bincode::de::{BorrowDecoder, Decoder};
use bincode::enc::Encoder;
use crate::activation::Activation;
use crate::audio::Audio;
use crate::core::CoreMode;
use crate::cycles::MachineCycles;
use crate::divider::Divider;
use crate::header::CartHeader;
use crate::interrupt::{InterruptFlags, InterruptType};
use crate::joypad::JoypadRegister;
use crate::ppu::PPU;
use crate::serial::Serial;
use crate::timer::Timer;

pub const RAM_BANK_SIZE: usize = 0x2000; // 8KB
pub const ROM_BANK_SIZE: usize = 0x4000; // 16KB

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
    interrupt_request: InterruptFlags,
    joypad_register: JoypadRegister,
    audio: Audio,
}

impl MMU {
    pub fn from_rom(data: &[u8]) -> Result<Self, String> {
        let header = CartHeader::parse(data)?;

        println!("{:?}", header);

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
            ppu: PPU::default(),
            interrupt_enable: InterruptFlags::default(),
            interrupt_request: InterruptFlags::default(),
            joypad_register: JoypadRegister::default(),
            serial: Serial::default(),
            divider: Divider::default(),
            timer: Timer::default(),
            audio: Audio::default(),
        })
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
        self.rom_bank_register = (value & 0x7F)
            .min(self.header.rom_banks() - 1)
            .max(1);
    }

    pub fn rom_data(&self, bank: usize, index: usize, length: usize) -> &[u8] {
        let start = bank * ROM_BANK_SIZE + index;
        let end = start + length;
        self.data.get(start..end)
            .unwrap_or_else(|| panic!("ROM slice out of bounds: bank={} index={} length={}", bank, index, length))
    }

    pub fn rom_data_from_pointer(&self, bank: usize, pointer: u16, length: usize) -> &[u8] {
        if bank == 0 {
            assert!(
                pointer < ROM_BANK_SIZE as u16,
                "Pointer {:04X} is invalid for bank {}", pointer, bank
            );
            self.rom_data(bank, pointer as usize, length)
        } else {
            assert!(
                pointer >= ROM_BANK_SIZE as u16 && pointer < ROM_BANK_SIZE as u16 * 2,
                "Pointer {:04X} is invalid for bank {}", pointer, bank
            );
            self.rom_data(bank, pointer as usize - ROM_BANK_SIZE, length)
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

    /// replace rom data, only intended for reloading save states without rom data
    pub fn set_data(&mut self, data: &[u8]) {
        self.data = data.to_vec();
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
            // DMA transfer is in progress, we need to copy data from ROM to OAM
            for i in 0 .. 0xA0 {
                let value = self.read(transfer.address + i);
                self.ppu.write_oam(i, value);
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
            0xFF00 => self.joypad_register.get(), // joypad register
            0xFF01 => self.serial.get_data(), // serial data register
            0xFF02 => self.serial.control(), // serial control register
            0xFF04 => self.divider.value(), // DIV register
            0xFF05 => self.timer.value(), // TIMA register
            0xFF06 => self.timer.modulo(), // TMA register
            0xFF07 => self.timer.control(), // TAC register
            0xFF0F => self.interrupt_request.get(), // IF register (interrupt request flags)
            0xFF10..=0xFF3F => self.audio.read(address),
            0xFF40 => self.ppu.lcd_control().get(), // LCD control register
            0xFF41 => self.ppu.lcd_status().stat(), // LCD status register
            0xFF42 => self.ppu.scroll().y, // SCY register
            0xFF43 => self.ppu.scroll().x, // SCX register
            0xFF44 => self.ppu.lcd_status().ly(), // LY register (read-only)
            0xFF45 => self.ppu.lcd_status().lyc(), // LYC register
            0xFF46 => 0, // DMA register (write-only, returns 0 when read)
            0xFF47 => self.ppu.palette().background().to_byte(), // BGP register
            0xFF48 => self.ppu.palette().object0().to_byte(), // OBP0 register
            0xFF49 => self.ppu.palette().object1().to_byte(), // OBP1 register
            0xFF4A => self.ppu.window_position().y, // WY register
            0xFF4B => self.ppu.window_position().x, // WX register
            0xFF80..=0xFFFE => self.high_ram[(address - 0xFF80) as usize], // high ram
            0xFFFF => self.interrupt_enable.get(),
            _ => {
                // ignore
                0xFF
            }
        }
    }

    pub fn read_u16_le(&self, address: u16) -> u16 {
        u16::from_le_bytes([self.read(address), self.read(address + 1)])
    }

    pub fn read_u16_be(&self, address: u16) -> u16 {
        u16::from_be_bytes([self.read(address), self.read(address + 1)])
    }

    pub fn read_u32_be(&self, address: u16) -> u32 {
        u32::from_be_bytes([
            self.read(address),
            self.read(address + 1),
            self.read(address + 2),
            self.read(address + 3)
        ])
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1FFF => {
                // https://gbdev.io/pandocs/MBC1.html#00001fff--ram-enable-write-only
                self.ram_enabled = value & 0xF == 0xA;
            }
            0x2000..=0x3FFF if self.header.rom_banks() > 2 => {
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
            0xFFFF => self.interrupt_enable.set(value),
            _ => {
                // ignore
            }
        }
    }

    pub fn write_u16_le(&mut self, address: u16, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write(address, low);
        self.write(address + 1, high);
    }

    pub fn write_u16_be(&mut self, address: u16, value: u16) {
        let [low, high] = value.to_be_bytes();
        self.write(address, low);
        self.write(address + 1, high);
    }

    pub fn write_u32_be(&mut self, address: u16, value: u32) {
        let bytes = value.to_be_bytes();
        for i in 0..bytes.len() {
            self.write(address + i as u16, bytes[i]);
        }
    }
}

impl Encode for MMU {
    fn encode<__E: Encoder>(&self, encoder: &mut __E) -> Result<(), bincode::error::EncodeError> {
        // Encode::encode(&self.data, encoder)?; Do not encode the ROM data
        Encode::encode(&self.header, encoder)?;
        Encode::encode(&self.ram_banks, encoder)?;
        Encode::encode(&self.ram_enabled, encoder)?;
        Encode::encode(&self.rom_bank_register, encoder)?;
        Encode::encode(&self.ram_bank_register, encoder)?;
        Encode::encode(&self.work_ram, encoder)?;
        Encode::encode(&self.high_ram, encoder)?;
        Encode::encode(&self.ppu, encoder)?;
        Encode::encode(&self.serial, encoder)?;
        Encode::encode(&self.divider, encoder)?;
        Encode::encode(&self.timer, encoder)?;
        Encode::encode(&self.interrupt_enable, encoder)?;
        Encode::encode(&self.interrupt_request, encoder)?;
        Encode::encode(&self.joypad_register, encoder)?;
        Encode::encode(&self.audio, encoder)?;
        core::result::Result::Ok(())
    }
}

impl<__Context> Decode<__Context> for MMU {
    fn decode<__D: Decoder<Context=__Context>>(decoder: &mut __D) -> Result<Self, ::bincode::error::DecodeError> {
        Ok(Self {
            data: vec![], // temporary empty data, will be filled in from the ROM
            header: Decode::decode(decoder)?,
            ram_banks: Decode::decode(decoder)?,
            ram_enabled: Decode::decode(decoder)?,
            rom_bank_register: Decode::decode(decoder)?,
            ram_bank_register: Decode::decode(decoder)?,
            work_ram: Decode::decode(decoder)?,
            high_ram: Decode::decode(decoder)?,
            ppu: Decode::decode(decoder)?,
            serial: Decode::decode(decoder)?,
            divider: Decode::decode(decoder)?,
            timer: Decode::decode(decoder)?,
            interrupt_enable: Decode::decode(decoder)?,
            interrupt_request: Decode::decode(decoder)?,
            joypad_register: Decode::decode(decoder)?,
            audio: Decode::decode(decoder)?
        })
    }
}
impl<'__de, __Context> BorrowDecode<'__de, __Context> for MMU {
    fn borrow_decode<__D: BorrowDecoder<'__de, Context=__Context>>(decoder: &mut __D) -> Result<Self, ::bincode::error::DecodeError> {
        Ok(Self {
            data: vec![],
            header: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            ram_banks: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            ram_enabled: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            rom_bank_register: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            ram_bank_register: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            work_ram: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            high_ram: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            ppu: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            serial: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            divider: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            timer: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            interrupt_enable: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            interrupt_request: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            joypad_register: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
            audio: BorrowDecode::<'_, __Context>::borrow_decode(decoder)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::roms::blargg_cpu::ROM;
    use super::*;

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