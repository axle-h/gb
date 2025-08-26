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

const RAM_BANK_SIZE: usize = 0x2000; // 8KB
const ROM_BANK_SIZE: usize = 0x4000; // 16KB

#[derive(Debug, Clone)]
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

            0xFF10..=0xFF3F => {
                let value = match address {
                    0xFF10 => self.audio.channel1().sweep_register().get(), // NR10: Channel 1 sweep register
                    0xFF11 => self.audio.channel1().length_duty_register().get(), // NR11: Channel 1 length and duty register
                    0xFF12 => self.audio.channel1().volume_envelope_register().get(), // NR12: Channel 1 volume and envelope register
                    0xFF13 => self.audio.channel1().period_control_register().nrx3(), // NR13: Channel 1 period low byte
                    0xFF14 => self.audio.channel1().period_control_register().nrx4(), // NR14: Channel 1 period high byte and control
                    0xFF16 => self.audio.channel2().length_duty_register().get(), // NR21: Channel 2 length and duty register
                    0xFF17 => self.audio.channel2().volume_envelope_register().get(), // NR22: Channel 2 volume and envelope register
                    0xFF18 => self.audio.channel2().period_control_register().nrx3(), // NR23: Channel 2 period low byte
                    0xFF19 => self.audio.channel2().period_control_register().nrx4(), // NR24: Channel 2 period high byte and control
                    0xFF1A => self.audio.channel3().nr30(), // NR30: Channel 3 DAC power
                    0xFF1B => self.audio.channel3().nr31(), // NR31: Channel 3 length timer
                    0xFF1C => self.audio.channel3().nr32(), // NR32: Channel 3 output level
                    0xFF1D => self.audio.channel3().nr33(), // NR33: Channel 3 frequency low
                    0xFF1E => self.audio.channel3().nr34(), // NR34: Channel 3 frequency high and control
                    0xFF24 => self.audio.master_volume().get(), // NR50: Sound volume register
                    0xFF25 => self.audio.panning().get(), // NR51: Sound panning register
                    0xFF26 => self.audio.control().get(), // NR52: Sound control register
                    0xFF30..=0xFF3F => self.audio.channel3().wave_ram((address - 0xFF30) as usize), // Wave RAM (0xFF30-0xFF3F)
                    _ => {
                        // ignore other audio registers for now
                        0xFF
                    }
                };

                println!("Read from audio register: {:04X} = {:02X}", address, value);
                value
            }

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
                // TODO MBC1 should mask to 0x1F
                self.rom_bank_register = ((value & 0x7F) as usize)
                    .min(self.header.rom_banks() - 1)
                    .max(1);
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

            0xFF10..=0xFF3F => {
                println!("Write to audio register: {:04X} = {:02X}", address, value);
                match address {
                    0xFF10 => self.audio.channel1_mut().sweep_register_mut().set(value), // NR10: Channel 1 sweep register
                    0xFF11 => self.audio.channel1_mut().length_duty_register_mut().set(value), // NR11: Channel 1 length and duty register
                    0xFF12 => self.audio.channel1_mut().volume_envelope_register_mut().set(value), // NR12: Channel 1 volume and envelope register
                    0xFF13 => self.audio.channel1_mut().period_control_register_mut().set_nrx3(value), // NR13: Channel 1 period low byte
                    0xFF14 => self.audio.channel1_mut().period_control_register_mut().set_nrx4(value), // NR14: Channel 1 period high byte and control
                    0xFF16 => self.audio.channel2_mut().length_duty_register_mut().set(value), // NR21: Channel 2 length and duty register
                    0xFF17 => self.audio.channel2_mut().volume_envelope_register_mut().set(value), // NR22: Channel 2 volume and envelope register
                    0xFF18 => self.audio.channel2_mut().period_control_register_mut().set_nrx3(value), // NR23: Channel 2 period low byte
                    0xFF19 => self.audio.channel2_mut().period_control_register_mut().set_nrx4(value), // NR24: Channel 2 period high byte and control
                    0xFF1A => self.audio.channel3_mut().set_nr30(value), // NR30: Channel 3 DAC power
                    0xFF1B => self.audio.channel3_mut().set_nr31(value), // NR31: Channel 3 length timer
                    0xFF1C => self.audio.channel3_mut().set_nr32(value), // NR32: Channel 3 output level
                    0xFF1D => self.audio.channel3_mut().set_nr33(value), // NR33: Channel 3 frequency low
                    0xFF1E => self.audio.channel3_mut().set_nr34(value), // NR34: Channel 3 frequency high and control
                    0xFF24 => self.audio.master_volume_mut().set(value), // NR50: Sound volume register
                    0xFF25 => self.audio.panning_mut().set(value), // NR51: Sound panning register
                    0xFF26 => self.audio.control_mut().set(value), // NR52: Sound control register
                    0xFF30..=0xFF3F => self.audio.channel3_mut().set_wave_ram((address - 0xFF30) as usize, value), // Wave RAM (0xFF30-0xFF3F)
                    _ => {
                        // ignore other audio registers for now
                    }
                }
            }
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
        assert_eq!(mmu.interrupt_request.get(), 0x1F);
        mmu.write(0xFF0F, 0x00); // Clear all interrupt flags
        assert_eq!(mmu.interrupt_request.get(), 0x00);
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