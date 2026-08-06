use bincode::{Decode, Encode};
use crate::activation::Activation;
use crate::audio::Audio;
use crate::core::CoreMode;
use crate::cycles::MachineCycles;
use crate::divider::Divider;
use crate::header::CartHeader;
use crate::hdma::{Hdma, HdmaRequest};
use crate::interrupt::{InterruptFlags, InterruptType};
use crate::joypad::JoypadRegister;
use crate::model::{ColorMode, Model};
use crate::pokemon::symbols::{DmgBank, DmgPointer};
use crate::ppu::{CGB_SECTION_VERSION, PPU};
use crate::ram::{RAM, ROM};
use crate::savestate::{labels, SectionReader, SectionWriter};
use crate::serial::Serial;
use crate::timer::Timer;

pub const RAM_BANK_SIZE: usize = 0x2000; // 8KB
pub const ROM_BANK_SIZE: usize = 0x4000; // 16KB
/// Work RAM is banked in 4 KB units: bank 0 is fixed at `0xC000`, `SVBK` selects the bank at
/// `0xD000`.
pub const WRAM_BANK_SIZE: usize = 0x1000;
/// A CGB has 8 work-RAM banks, a DMG 2. All eight are allocated either way — gambatte does the
/// same, and it keeps the addressing identical across models.
pub const WRAM_BANKS: usize = 8;
/// The `0xC000..=0xDFFF` window: bank 0 plus one switchable bank. On DMG this is *all* of work
/// RAM, which is why [`MMU::work_ram`] can still hand it out as one flat slice.
pub const WRAM_WINDOW: usize = WRAM_BANK_SIZE * 2;

/// `0xFEA0..=0xFEFF` on CGB is three 8-byte blocks of RAM, each mirrored four times across its
/// own 32-byte span. Unlike DMG — where the whole region is write-protected and reads `0x00` —
/// it holds what is written to it.
pub const UNUSABLE_BLOCK: usize = 8;
pub const UNUSABLE_BLOCKS: usize = 3;

/// Power-on contents of that region, taken byte for byte from gambatte's committed hardware dump
/// `test/hwtests/fexx_ffxx_dumper_cgb.bin`, whose `0xA0..=0xFF` is exactly these three patterns
/// repeated four times each. The DMG dump beside it is all zeroes, which is where the `0x00`
/// A13 settled on came from.
pub const UNUSABLE_CGB: [u8; UNUSABLE_BLOCK * UNUSABLE_BLOCKS] = [
    0x08, 0x01, 0xEF, 0xDE, 0x06, 0x4A, 0xCD, 0xBD, // 0xFEA0..=0xFEBF
    0x00, 0x90, 0xF7, 0x7F, 0xC0, 0xB1, 0xBC, 0xFB, // 0xFEC0..=0xFEDF
    0x24, 0x13, 0xFD, 0x3A, 0x10, 0x10, 0xAD, 0x45, // 0xFEE0..=0xFEFF
];

/// Where `0xFEA0..=0xFEFF` lands in [`MMU::unusable`]: one 8-byte block per 32-byte span,
/// mirrored four times inside it.
fn unusable_offset(address: u16) -> usize {
    let offset = (address - 0xFEA0) as usize;
    (offset / 32) * UNUSABLE_BLOCK + (offset % UNUSABLE_BLOCK)
}

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
    /// Eight 4 KB banks. A DMG only ever addresses the first two, so `work_ram[..0x2000]` is
    /// exactly the old flat array — see [`MMU::work_ram`].
    work_ram: [u8; WRAM_BANK_SIZE * WRAM_BANKS],
    /// `SVBK` (`FF70`), 1..=7. Never anything but 1 on DMG or in CGB compatibility mode.
    work_ram_bank: usize,
    high_ram: [u8; 0x7F], // 128 bytes of high RAM
    /// Which console this is. A construction-time property; it is not guest state and is not
    /// serialised (a save state carries its `cgb` section instead).
    model: Model,
    color_mode: ColorMode,
    /// `KEY1` (`FF4D`) bit 0: the guest has asked for a speed switch on the next `STOP`.
    speed_switch_armed: bool,
    /// `KEY1` bit 7: the CPU is running at 8 MHz. See [`MMU::update`] for what that means for the
    /// peripherals.
    double_speed: bool,
    /// Odd M-cycle carried over when halving the CPU clock for the peripherals in double speed.
    /// Without it, a run of single-cycle instructions would round every one of them down to zero.
    double_speed_carry: bool,
    hdma: Hdma,
    /// `FF72`, `FF73`, `FF74`, `FF75` — undocumented CGB scratch registers with no known effect.
    undocumented: [u8; 4],
    /// `SC` bit 1: the CGB's 32x serial clock. Held here rather than inside [`Serial`] so the
    /// already-shipped `timer` save-state section keeps its shape.
    serial_fast: bool,
    /// The three 8-byte blocks behind `0xFEA0..=0xFEFF` on CGB, where the region is ordinary
    /// mirrored RAM rather than the write-protected zeroes a DMG returns. See [`UNUSABLE_CGB`].
    unusable: [u8; UNUSABLE_BLOCK * UNUSABLE_BLOCKS],
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

/// Contents of the MMU's half of the `cgb` save-state section — the PPU writes its own fields
/// first, see [`crate::ppu::CgbVideoSection`].
#[derive(Debug, Clone, Decode, Encode)]
pub struct CgbSection {
    pub work_ram_bank: usize,
    pub speed_switch_armed: bool,
    pub double_speed: bool,
    pub double_speed_carry: bool,
    pub hdma: Hdma,
    pub undocumented: [u8; 4],
    pub serial_fast: bool,
    pub unusable: [u8; UNUSABLE_BLOCK * UNUSABLE_BLOCKS],
}

pub const CART_SECTION_VERSION: u16 = 1;
/// Bumped to 2 by B2, which appended work-RAM banks 2-7. Field 1 keeps its v1 shape — banks 0
/// and 1, all a DMG has — so states written before CGB support still decode untouched.
pub const WRAM_SECTION_VERSION: u16 = 2;
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
        let mut window = [0u8; WRAM_WINDOW];
        window.copy_from_slice(&self.work_ram[..WRAM_WINDOW]);
        let mut upper = [0u8; WRAM_BANK_SIZE * WRAM_BANKS - WRAM_WINDOW];
        upper.copy_from_slice(&self.work_ram[WRAM_WINDOW..]);
        writer.write_fields(labels::WRAM, WRAM_SECTION_VERSION, |fields| {
            fields.field(&window)?;
            fields.field(&upper) // appended in v2
        })?;
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
        writer.write_fields(labels::CGB, CGB_SECTION_VERSION, |fields| {
            self.ppu.write_cgb_fields(fields)?;
            fields.field(&CgbSection {
                work_ram_bank: self.work_ram_bank,
                speed_switch_armed: self.speed_switch_armed,
                double_speed: self.double_speed,
                double_speed_carry: self.double_speed_carry,
                hdma: self.hdma,
                undocumented: self.undocumented,
                serial_fast: self.serial_fast,
                unusable: self.unusable,
            })
        })?;
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
        if let Some(mut fields) = reader.section(labels::WRAM)? {
            if let Some(window) = fields.field::<[u8; WRAM_WINDOW]>()? {
                self.work_ram[..WRAM_WINDOW].copy_from_slice(&window);
                // Absent in v1 payloads, which is not an error: a DMG state has no banks 2-7.
                let upper = fields.field::<[u8; WRAM_BANK_SIZE * WRAM_BANKS - WRAM_WINDOW]>()?
                    .unwrap_or([0; WRAM_BANK_SIZE * WRAM_BANKS - WRAM_WINDOW]);
                self.work_ram[WRAM_WINDOW..].copy_from_slice(&upper);
            }
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
        // Absent from every state written before Phase B, in which case the machine keeps the
        // CGB defaults it was constructed with — all of which are "this is a DMG".
        if let Some(mut fields) = reader.section(labels::CGB)? {
            self.ppu.read_cgb_fields(&mut fields)?;
            if let Some(section) = fields.field::<CgbSection>()? {
                self.work_ram_bank = section.work_ram_bank.clamp(1, WRAM_BANKS - 1);
                self.speed_switch_armed = section.speed_switch_armed;
                self.double_speed = section.double_speed;
                self.double_speed_carry = section.double_speed_carry;
                self.hdma = section.hdma;
                self.undocumented = section.undocumented;
                self.serial_fast = section.serial_fast;
                self.unusable = section.unusable;
            }
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
        Self::new(data, Model::Dmg)
    }

    pub fn new(data: &[u8], model: Model) -> Result<Self, String> {
        let header = CartHeader::parse(data)?;

        println!("{:?}", header);

        let color_mode = ColorMode::of(model, &header);
        let ram_banks = Vec::from_iter((0..header.ram_banks()).map(|_| [0; RAM_BANK_SIZE]));
        let mut ppu = PPU::default();
        ppu.set_color_mode(color_mode);

        let mut mmu = Self {
            data: pad_rom(data),
            header,
            ram_banks,
            ram_enabled: false,
            rom_bank_register: 1,
            ram_bank_register: 0,
            work_ram: [0; WRAM_BANK_SIZE * WRAM_BANKS],
            work_ram_bank: 1,
            high_ram: [0; 0x7F],
            model,
            color_mode,
            speed_switch_armed: false,
            double_speed: false,
            double_speed_carry: false,
            hdma: Hdma::default(),
            undocumented: [0; 4],
            serial_fast: false,
            unusable: if model.is_cgb() { UNUSABLE_CGB } else { [0; UNUSABLE_BLOCK * UNUSABLE_BLOCKS] },
            ppu,
            interrupt_enable: InterruptFlags::default(),
            interrupt_enable_upper: 0,
            interrupt_request: InterruptFlags::default(),
            joypad_register: JoypadRegister::default(),
            serial: Serial::default(),
            divider: Divider::default(),
            timer: Timer::default(),
            audio: Audio::default(),
        };
        mmu.apply_boot_state();
        Ok(mmu)
    }

    /// The state the boot ROM leaves behind. `gb` starts the cartridge directly rather than
    /// executing a boot ROM, so anything the boot ROM would have installed has to be applied here.
    ///
    /// Today that is exactly one thing, and it is the point of Phase B: in **CGB compatibility
    /// mode** the boot ROM writes a palette chosen from the cartridge title into CGB palette RAM
    /// and sets `OPRI` for DMG sprite priority (SameBoy `cgb_boot.asm`, `EmulateDMG`). Everything
    /// else the boot ROM does — the logo check, the intro animation, the register block — either
    /// has no observable effect here or is already covered by [`crate::registers::RegisterSet`].
    fn apply_boot_state(&mut self) {
        if self.color_mode != ColorMode::CgbCompat {
            return;
        }
        let palettes = crate::boot_palette::for_cartridge(&self.data);
        self.ppu.cgb_background_palettes_mut().set_palette(0, palettes.background);
        self.ppu.cgb_object_palettes_mut().set_palette(0, palettes.object0);
        self.ppu.cgb_object_palettes_mut().set_palette(1, palettes.object1);
        self.ppu.set_object_priority_register(0x01);
    }

    pub fn model(&self) -> Model {
        self.model
    }

    pub fn color_mode(&self) -> ColorMode {
        self.color_mode
    }

    pub fn is_double_speed(&self) -> bool {
        self.double_speed
    }

    /// Return to power-on state, **preserving the cartridge and its battery-backed RAM** — the
    /// same contract as gambatte's `GB::reset` (`gambatte.cpp:79-89`). Everything reset here must
    /// match what [`MMU::from_rom`] constructs, or `Core::reset` will not produce a machine equal
    /// to a fresh one.
    pub fn reset(&mut self) {
        self.ram_enabled = false;
        self.rom_bank_register = 1;
        self.ram_bank_register = 0;
        self.work_ram = [0; WRAM_BANK_SIZE * WRAM_BANKS];
        self.work_ram_bank = 1;
        self.high_ram = [0; 0x7F];
        self.speed_switch_armed = false;
        self.double_speed = false;
        self.double_speed_carry = false;
        self.hdma = Hdma::default();
        self.undocumented = [0; 4];
        self.serial_fast = false;
        self.unusable = if self.model.is_cgb() { UNUSABLE_CGB } else { [0; UNUSABLE_BLOCK * UNUSABLE_BLOCKS] };
        self.ppu = PPU::default();
        self.ppu.set_color_mode(self.color_mode);
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
        self.apply_boot_state();
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

    /// The `0xC000..=0xDFFF` window as one flat slice. On DMG that is all of work RAM and this is
    /// byte-for-byte the array it always was; on CGB it is bank 0 followed by bank **1**, not by
    /// whatever `SVBK` currently selects. The Pokémon layer reads DMG game state through this.
    pub fn work_ram(&self) -> &[u8] {
        &self.work_ram[..WRAM_WINDOW]
    }

    /// Where an address in `0xC000..=0xDFFF` lands in the flat array, following `SVBK`.
    ///
    /// The trailing mask is not defensive — it is what lets the compiler prove the index is in
    /// range and drop the bounds check. `work_ram_bank` is a plain `usize` field, so without it
    /// every work-RAM access in the interpreter pays for a branch it can never take. Measured:
    /// this and the matching masks in the PPU are worth ~10% of core throughput.
    #[inline]
    fn work_ram_offset(&self, address: u16) -> usize {
        let offset = (address & 0x1FFF) as usize;
        // Bank 0 is fixed at 0xC000; SVBK selects what sits at 0xD000.
        let bank = if offset < WRAM_BANK_SIZE { 0 } else { self.work_ram_bank };
        (bank * WRAM_BANK_SIZE + (offset & (WRAM_BANK_SIZE - 1))) & (WRAM_BANK_SIZE * WRAM_BANKS - 1)
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

    /// A `STOP` on a CGB with `KEY1` bit 0 set switches the CPU between 4 MHz and 8 MHz instead of
    /// stopping, and resets `DIV` as it does so. Returns whether the switch happened, i.e. whether
    /// the caller should *not* enter stop mode.
    pub fn try_speed_switch(&mut self) -> bool {
        if !self.color_mode.cgb_features() || !self.speed_switch_armed {
            return false;
        }
        self.double_speed = !self.double_speed;
        self.speed_switch_armed = false;
        self.double_speed_carry = false;
        self.divider.reset();
        true
    }

    /// Record how long the instruction now executing is. Forwarded to the APU, which is the only
    /// peripheral modelled finely enough to care where inside it the bus access falls — see
    /// [`Audio::set_instruction_length`].
    pub fn set_instruction_length(&mut self, machine_cycles: u8) {
        self.audio.set_instruction_length(machine_cycles);
    }

    /// Source reads for a VRAM DMA. Hardware cannot copy *out* of VRAM or out of the OAM/IO
    /// region; both read back as `0xFF`.
    fn vram_dma_source(&self, address: u16) -> u8 {
        match address {
            0x8000..=0x9FFF | 0xE000..=0xFFFF => 0xFF,
            _ => self.read(address),
        }
    }

    /// GDMA: the whole block at once. See [`Hdma`] for what this does *not* model — the CPU
    /// stall.
    ///
    /// Out of line, like every other DMA path here: `MMU::update` runs once per instruction, and
    /// letting these inline into it grew it by 60% and cost ~4% of core throughput on its own.
    #[cold]
    #[inline(never)]
    fn run_general_dma(&mut self, blocks: u8) {
        let copies: Vec<(u16, u16)> = self.hdma.general_blocks(blocks)
            .flat_map(|block| block.bytes().collect::<Vec<_>>())
            .collect();
        for (source, destination) in copies {
            let value = self.vram_dma_source(source);
            self.ppu.write_vram_dma(destination, value);
        }
        self.hdma.advance(blocks);
    }

    /// HDMA: one `0x10`-byte block, at the start of each HBlank. Out of line — see
    /// [`MMU::run_general_dma`].
    #[cold]
    #[inline(never)]
    fn run_hblank_dma(&mut self) {
        let Some(block) = self.hdma.next_block() else { return };
        for (source, destination) in block.bytes().collect::<Vec<_>>() {
            let value = self.vram_dma_source(source);
            self.ppu.write_vram_dma(destination, value);
        }
    }

    /// The CGB register block, `FF4C`-`FF7F`, behind one unguarded match arm.
    ///
    /// **Kept out of line deliberately.** Spreading fifteen `if self.color_mode.cgb_features()`
    /// guards through `MMU::read`'s address match stops LLVM building a jump table for it, and
    /// every memory access in the machine pays for that. Measured at ~10% of core throughput on
    /// the CPU-bound workloads. Guards are the cost, not the branches — the same conditions
    /// inside a function body are free.
    #[cold]
    fn read_cgb_register(&self, address: u16) -> u8 {
        if !self.color_mode.cgb_features() {
            // In compatibility mode the boot ROM has locked the cartridge into the DMG register
            // set, so all of these read as unmapped — except the undocumented block, which is
            // CGB *hardware* rather than a CGB *feature*.
            return match address {
                0xFF72..=0xFF74 if self.model.is_cgb() => self.undocumented[(address - 0xFF72) as usize],
                0xFF75 if self.model.is_cgb() => 0x8F | self.undocumented[3],
                _ => 0xFF,
            };
        }
        match address {
            // KEY1: bit 7 = current speed, bit 0 = switch armed, the rest read 1.
            0xFF4D => 0x7E | if self.double_speed { 0x80 } else { 0 } | u8::from(self.speed_switch_armed),
            0xFF4F => self.ppu.vram_bank_register(),
            0xFF51..=0xFF54 => 0xFF, // HDMA1-4 are write-only
            0xFF55 => self.hdma.status(),
            0xFF68 => self.ppu.cgb_background_palettes().index(),
            0xFF69 => self.ppu.cgb_background_palettes().read(),
            0xFF6A => self.ppu.cgb_object_palettes().index(),
            0xFF6B => self.ppu.cgb_object_palettes().read(),
            0xFF6C => self.ppu.object_priority_register(),
            0xFF70 => 0xF8 | self.work_ram_bank as u8,
            0xFF72..=0xFF74 => self.undocumented[(address - 0xFF72) as usize],
            0xFF75 => 0x8F | self.undocumented[3],
            _ => 0xFF,
        }
    }

    /// The write half of [`MMU::read_cgb_register`], and out of line for the same reason.
    #[cold]
    fn write_cgb_register(&mut self, address: u16, value: u8) {
        if !self.color_mode.cgb_features() {
            if self.model.is_cgb() {
                match address {
                    0xFF72..=0xFF74 => self.undocumented[(address - 0xFF72) as usize] = value,
                    0xFF75 => self.undocumented[3] = value & 0x70,
                    _ => {}
                }
            }
            return;
        }
        match address {
            0xFF4D => self.speed_switch_armed = value & 0x01 != 0,
            0xFF4F => self.ppu.set_vram_bank_register(value),
            0xFF51 => self.hdma.set_source_high(value),
            0xFF52 => self.hdma.set_source_low(value),
            0xFF53 => self.hdma.set_destination_high(value),
            0xFF54 => self.hdma.set_destination_low(value),
            0xFF55 => {
                if let HdmaRequest::General(blocks) = self.hdma.request(value) {
                    self.run_general_dma(blocks);
                }
            }
            0xFF68 => self.ppu.cgb_background_palettes_mut().set_index(value),
            0xFF69 => self.ppu.cgb_background_palettes_mut().write(value),
            0xFF6A => self.ppu.cgb_object_palettes_mut().set_index(value),
            0xFF6B => self.ppu.cgb_object_palettes_mut().write(value),
            0xFF6C => self.ppu.set_object_priority_register(value),
            // Bank 0 selects bank 1 — hardware has no way to map bank 0 twice. gambatte applies
            // this fixup in two places (`memory.cpp:1074-1079`, `memptrs.cpp:146-150`).
            0xFF70 => self.work_ram_bank = ((value & 0x07) as usize).max(1),
            0xFF72..=0xFF74 => self.undocumented[(address - 0xFF72) as usize] = value,
            0xFF75 => self.undocumented[3] = value & 0x70,
            _ => {}
        }
    }

    /// The OAM DMA controller owns the OAM bus, so it writes through the privileged path rather
    /// than the CPU-facing, mode-gated one. Out of line — see [`MMU::run_general_dma`].
    #[cold]
    #[inline(never)]
    fn run_oam_dma(&mut self, transfer: crate::lcd_dma::DmaTransfer) {
        for (source, offset) in transfer.bytes() {
            let value = self.read(source);
            self.ppu.write_oam_dma(offset, value);
        }
    }

    /// update internal state of the MMU, should be called every CPU cycle
    pub fn update(&mut self, delta_machine_cycles: MachineCycles) {
        if delta_machine_cycles == MachineCycles::ZERO {
            return; // no cycles to update
        }

        if let Some(transfer) = self.ppu.dma_mut().update(delta_machine_cycles) {
            self.run_oam_dma(transfer);
        }

        // In double speed the CPU runs at 8 MHz while the video and audio hardware does not, so
        // they see half the elapsed M-cycles. DIV and the timer are *not* divided: they are
        // clocked from the CPU, so they keep pace with it and the APU frame sequencer, which
        // hangs off DIV, stays at 512 Hz in real time exactly as hardware does. The carry bit
        // makes the halving exact rather than rounding every odd cycle away.
        let video_cycles = if self.double_speed {
            let total = delta_machine_cycles.m_cycles() + usize::from(self.double_speed_carry);
            self.double_speed_carry = total % 2 == 1;
            MachineCycles::from_m(total / 2)
        } else {
            delta_machine_cycles
        };

        self.serial.update(delta_machine_cycles, self.serial_fast);
        let div_clocks = self.divider.update(delta_machine_cycles);
        self.timer.update(delta_machine_cycles);
        self.ppu.update(video_cycles);
        if self.ppu.consume_hblank_started() && self.hdma.is_active() {
            self.run_hblank_dma();
        }
        self.audio.update(video_cycles, div_clocks);

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
            // Work RAM, and its echo — which mirrors the *banked* window, SVBK included.
            0xC000..=0xDFFF | 0xE000..=0xFDFF => self.work_ram[self.work_ram_offset(address)],
            0xFE00..=0xFE9F => self.ppu.read_oam(address - 0xFE00), // OAM (Object Attribute Memory)
            // The unusable region. DMG returns 0x00 here, not 0xFF — settled by gambatte's
            // committed hardware dump `test/hwtests/fexx_ffxx_dumper_dmg08.bin`, which is all
            // zeros at offsets 0xA0..0xFF. On CGB the same dump shows ordinary mirrored RAM.
            0xFEA0..=0xFEFF => {
                if self.model.is_cgb() { self.unusable[unusable_offset(address)] } else { 0x00 }
            }
            0xFF00 => 0xC0 | self.joypad_register.get(), // joypad register — bits 6-7 unused, read 1
            0xFF01 => self.serial.get_data(), // serial data register
            // SC: bits 1-6 read 1 on DMG. On CGB bit 1 is the 32x clock-speed select and is
            // real, so only bits 2-6 are stuck high there.
            0xFF02 => {
                if self.model.is_cgb() {
                    0x7C | self.serial.control() | (u8::from(self.serial_fast) << 1)
                } else {
                    0x7E | self.serial.control()
                }
            }
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
            // The CGB register block, behind a single unguarded arm — see `read_cgb_register`.
            0xFF4C..=0xFF7F => self.read_cgb_register(address),
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
            0xC000..=0xDFFF | 0xE000..=0xFDFF => {
                let offset = self.work_ram_offset(address);
                self.work_ram[offset] = value;
            }
            0xFE00..=0xFE9F => self.ppu.write_oam(address - 0xFE00, value), // OAM (Object Attribute Memory)
            // Write-protected on DMG; ordinary mirrored RAM on CGB.
            0xFEA0..=0xFEFF => {
                if self.model.is_cgb() {
                    self.unusable[unusable_offset(address)] = value;
                }
            }
            0xFF00 => self.joypad_register.set(value),
            0xFF01 => self.serial.set_data(value), // serial data register
            0xFF02 => {
                if self.model.is_cgb() {
                    self.serial_fast = value & 0x02 != 0;
                }
                self.serial.set_control(value);
            }
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
            0xFF4C..=0xFF7F => self.write_cgb_register(address, value),
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

    /// Phase B. `cgb_acid::ROM` has `0x143 = 0xC0` (CGB exclusive) so it gets the full register
    /// set; `POKERED` has `0x143 = 0x00` and therefore runs in compatibility mode, where the boot
    /// ROM has locked the cartridge out of every CGB register.
    mod cgb {
        use super::*;
        use crate::ppu::VRAM_BANK_SIZE;

        fn cgb() -> MMU {
            MMU::new(crate::roms::cgb_acid::ROM, Model::Cgb).unwrap()
        }

        fn compat() -> MMU {
            MMU::new(crate::pokemon::roms::POKERED, Model::Cgb).unwrap()
        }

        #[test]
        fn color_mode_follows_the_cartridge_header() {
            assert_eq!(cgb().color_mode(), ColorMode::Cgb);
            assert_eq!(compat().color_mode(), ColorMode::CgbCompat);
            assert_eq!(MMU::from_rom(crate::pokemon::roms::POKERED).unwrap().color_mode(), ColorMode::Dmg);
            assert_eq!(MMU::new(crate::roms::cgb_acid::ROM, Model::Dmg).unwrap().color_mode(), ColorMode::Dmg);
        }

        /// B2. `SVBK = 0` selects bank **1** — hardware cannot map bank 0 into the switchable
        /// slot. gambatte applies the same fixup in two places (`memory.cpp:1074-1079`).
        #[test]
        fn svbk_bank_zero_selects_bank_one() {
            let mut mmu = cgb();
            assert_eq!(mmu.read(0xFF70) & 0x07, 1, "power-on bank");

            mmu.write(0xFF70, 0x00);
            assert_eq!(mmu.read(0xFF70), 0xF8 | 1, "bank 0 maps to 1; bits 3-7 read 1");
            mmu.write(0xFF70, 0x07);
            assert_eq!(mmu.read(0xFF70), 0xF8 | 7);
        }

        /// Each work-RAM bank is distinct, `0xC000` is always bank 0 whatever `SVBK` says, and
        /// echo RAM mirrors the *banked* window rather than a fixed one.
        #[test]
        fn work_ram_banks_are_independent_and_echoed() {
            let mut mmu = cgb();

            for bank in 1..WRAM_BANKS {
                mmu.write(0xFF70, bank as u8);
                mmu.write(0xD000, 0xB0 | bank as u8);
            }
            mmu.write(0xC000, 0xAA); // bank 0, which no SVBK value can displace

            for bank in 1..WRAM_BANKS {
                mmu.write(0xFF70, bank as u8);
                assert_eq!(mmu.read(0xD000), 0xB0 | bank as u8, "bank {bank}");
                assert_eq!(mmu.read(0xC000), 0xAA, "bank 0 is fixed");
                // Echo RAM: 0xF000 mirrors 0xD000, following SVBK with it.
                assert_eq!(mmu.read(0xF000), 0xB0 | bank as u8, "echo of bank {bank}");
                assert_eq!(mmu.read(0xE000), 0xAA, "echo of bank 0");
            }
        }

        /// `MMU::work_ram` is what the Pokémon layer reads DMG game state through. It must stay
        /// the flat `0xC000..=0xDFFF` window — banks 0 and 1 — no matter what `SVBK` selects.
        #[test]
        fn the_flat_work_ram_view_ignores_svbk() {
            let mut mmu = cgb();
            mmu.write(0xFF70, 1);
            mmu.write(0xD000, 0x11);
            mmu.write(0xFF70, 5);
            mmu.write(0xD000, 0x55);

            assert_eq!(mmu.work_ram().len(), WRAM_WINDOW);
            assert_eq!(mmu.work_ram()[WRAM_BANK_SIZE], 0x11, "the view is bank 0 then bank 1");
            assert_eq!(mmu.read_wram_slice(0xD000, 1).unwrap(), &[0x11]);
        }

        /// B2. Two VRAM banks, and `PPU::vram` still means bank 0.
        #[test]
        fn vram_banks_are_independent() {
            let mut mmu = cgb();
            assert_eq!(mmu.read(0xFF4F), 0xFE, "VBK: only bit 0 exists");

            // VRAM is only writable outside mode 3.
            mmu.ppu.lcd_status_mut().set_mode(crate::lcd_status::LcdMode::VBlank);
            mmu.write(0xFF4F, 0x00);
            mmu.write(0x8000, 0x11);
            mmu.write(0xFF4F, 0x01);
            assert_eq!(mmu.read(0xFF4F), 0xFF);
            mmu.write(0x8000, 0x22);

            assert_eq!(mmu.read(0x8000), 0x22, "bank 1 is selected");
            mmu.write(0xFF4F, 0x00);
            assert_eq!(mmu.read(0x8000), 0x11, "bank 0 is untouched");
            assert_eq!(mmu.ppu().vram()[0], 0x11, "PPU::vram is bank 0 regardless");
            assert_eq!(mmu.ppu().vram_banked(1)[0], 0x22);
        }

        /// B3. Palette RAM through the bus, including auto-increment and the read-back masks.
        #[test]
        fn palette_ram_round_trips_through_the_registers() {
            let mut mmu = cgb();
            mmu.write(0xFF68, 0x80); // BCPS: address 0, auto-increment
            for i in 0..64u8 {
                mmu.write(0xFF69, i.wrapping_mul(3));
            }
            for i in 0..64u8 {
                mmu.write(0xFF68, i);
                assert_eq!(mmu.read(0xFF68), i | 0x40, "BCPS bit 6 reads 1");
                assert_eq!(mmu.read(0xFF69), i.wrapping_mul(3), "BG byte {i}");
            }

            mmu.write(0xFF6A, 0x80 | 0x04); // OCPS
            mmu.write(0xFF6B, 0xCD);
            mmu.write(0xFF6A, 0x04);
            assert_eq!(mmu.read(0xFF6B), 0xCD);
            assert_eq!(mmu.read(0xFF6C), 0xFE, "OPRI: only bit 0 exists");
        }

        /// Every CGB-only register must be invisible both to a DMG and to a cartridge running in
        /// compatibility mode, where the boot ROM has locked the DMG register set in.
        #[test]
        fn cgb_registers_are_unmapped_without_cgb_features() {
            const CGB_ONLY: [u16; 12] =
                [0xFF4D, 0xFF4F, 0xFF51, 0xFF52, 0xFF53, 0xFF54, 0xFF55, 0xFF68, 0xFF69, 0xFF6A, 0xFF6B, 0xFF70];

            for mut mmu in [MMU::from_rom(crate::pokemon::roms::POKERED).unwrap(), compat()] {
                let mode = mmu.color_mode();
                for address in CGB_ONLY {
                    mmu.write(address, 0x55);
                    assert_eq!(mmu.read(address), 0xFF, "{address:04X} in {mode:?}");
                }
                // ...and the banking they control never moves.
                mmu.write(0xD000, 0x42);
                mmu.write(0xFF70, 0x05);
                assert_eq!(mmu.read(0xD000), 0x42, "work RAM must not bank in {mode:?}");
            }
        }

        /// ⭐ B5's other half: in compatibility mode the cartridge cannot reach palette RAM, but
        /// the boot ROM has already filled it, and that is what colours the screen.
        #[test]
        fn compatibility_mode_gets_the_boot_palette_it_cannot_write() {
            let mut mmu = compat();
            let expected = crate::boot_palette::for_cartridge(crate::pokemon::roms::POKERED);
            assert_eq!(&mmu.ppu().cgb_background_palettes().data()[..8], &expected.background);
            assert_eq!(&mmu.ppu().cgb_object_palettes().data()[..8], &expected.object0);
            assert_eq!(&mmu.ppu().cgb_object_palettes().data()[8..16], &expected.object1);
            assert_eq!(mmu.ppu().object_priority_register() & 0x01, 1, "boot ROM sets DMG sprite priority");

            // The cartridge writing BCPD must not disturb it.
            mmu.write(0xFF68, 0x80);
            mmu.write(0xFF69, 0x00);
            assert_eq!(&mmu.ppu().cgb_background_palettes().data()[..8], &expected.background);
        }

        /// A DMG never gets a boot palette, whatever its cartridge title says.
        #[test]
        fn a_dmg_gets_no_boot_palette() {
            let mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
            assert_eq!(mmu.ppu().cgb_background_palettes(), &crate::cgb_palette::PaletteBank::default());
        }

        /// B7. `KEY1` arms the switch; `STOP` performs it and resets `DIV`.
        #[test]
        fn key1_switches_speed_on_stop() {
            let mut mmu = cgb();
            assert_eq!(mmu.read(0xFF4D), 0x7E, "single speed, unarmed");
            assert!(!mmu.try_speed_switch(), "an unarmed STOP really stops");

            mmu.write(0xFF4D, 0x01);
            assert_eq!(mmu.read(0xFF4D), 0x7F, "armed");
            mmu.update(MachineCycles::from_m(1000)); // let DIV move off zero
            assert_ne!(mmu.divider().value(), 0);

            assert!(mmu.try_speed_switch());
            assert!(mmu.is_double_speed());
            assert_eq!(mmu.read(0xFF4D), 0xFE, "bit 7 set, arm bit cleared");
            assert_eq!(mmu.divider().value(), 0, "the switch resets DIV");

            mmu.write(0xFF4D, 0x01);
            assert!(mmu.try_speed_switch());
            assert!(!mmu.is_double_speed(), "and back again");
        }

        /// The point of double speed: the CPU runs twice as fast *relative to the PPU*. DIV is
        /// clocked from the CPU and so is not divided — which is what keeps the APU frame
        /// sequencer at 512 Hz in real time.
        #[test]
        fn double_speed_halves_the_video_clock_but_not_div() {
            /// Scanlines advanced, unwrapped past `LY`'s 154-line wrap so the two runs are
            /// directly comparable.
            fn advance(double: bool, m_cycles: usize) -> (usize, u8) {
                let mut mmu = cgb();
                mmu.ppu.lcd_control_mut().set(0x80); // LCD on
                if double {
                    mmu.write(0xFF4D, 0x01);
                    assert!(mmu.try_speed_switch());
                }
                let mut scanlines = 0usize;
                let mut previous = mmu.ppu().lcd_status().ly();
                // One M-cycle at a time, so the odd-cycle carry is actually exercised.
                for _ in 0..m_cycles {
                    mmu.update(MachineCycles::ONE);
                    let ly = mmu.ppu().lcd_status().ly();
                    if ly != previous {
                        scanlines += 1;
                        previous = ly;
                    }
                }
                (scanlines, mmu.divider().value())
            }

            // Twice the CPU cycles at double speed must render exactly as much as half as many
            // at single speed. Stating it as a ratio rather than an absolute count keeps the
            // assertion independent of where in a scanline the PPU happens to start.
            let (single, single_div) = advance(false, 20_000);
            let (double, double_div) = advance(true, 40_000);
            assert!(single > 100, "test is vacuous unless the PPU actually ran ({single} scanlines)");
            assert_eq!(single, double, "double speed must halve the video clock exactly");

            // DIV is clocked from the CPU, so it is *not* divided — which is what keeps the APU
            // frame sequencer at 512 Hz in real time, as on hardware.
            let (_, half_div) = advance(true, 20_000);
            assert_eq!(single_div, half_div, "DIV tracks the CPU, not the PPU");
            assert_ne!(single_div, double_div, "...so twice the CPU cycles is twice the DIV");
        }

        /// B8. GDMA copies the whole block the moment `FF55` is written.
        #[test]
        fn general_purpose_dma_copies_immediately() {
            let mut mmu = cgb();
            mmu.ppu.lcd_status_mut().set_mode(crate::lcd_status::LcdMode::VBlank);
            for i in 0..0x40u16 {
                mmu.write(0xC000 + i, (i as u8).wrapping_mul(5).wrapping_add(2));
            }

            mmu.write(0xFF51, 0xC0);
            mmu.write(0xFF52, 0x00);
            mmu.write(0xFF53, 0x00);
            mmu.write(0xFF54, 0x00); // -> 0x8000
            mmu.write(0xFF55, 0x03); // bit 7 clear: 4 blocks, right now

            assert_eq!(mmu.read(0xFF55), 0xFF, "the transfer is already over");
            for i in 0..0x40u16 {
                assert_eq!(mmu.read(0x8000 + i), (i as u8).wrapping_mul(5).wrapping_add(2), "byte {i}");
            }
        }

        /// B8. HDMA copies exactly one block per HBlank, and no more.
        #[test]
        fn hblank_dma_copies_one_block_per_scanline() {
            let mut mmu = cgb();
            mmu.ppu.lcd_control_mut().set(0x80); // LCD on
            for i in 0..0x40u16 {
                mmu.write(0xC000 + i, 0x40 + i as u8);
            }

            mmu.write(0xFF51, 0xC0);
            mmu.write(0xFF52, 0x00);
            mmu.write(0xFF53, 0x00);
            mmu.write(0xFF54, 0x00);
            mmu.write(0xFF55, 0x80 | 0x03); // 4 blocks, HBlank-paced
            assert_eq!(mmu.read(0xFF55) & 0x80, 0, "bit 7 clear while running");

            let mut blocks_seen = 0;
            for scanline in 0..4 {
                // Run a whole scanline; exactly one HBlank happens in it.
                for _ in 0..114 {
                    mmu.update(MachineCycles::ONE);
                }
                let copied = (0..0x40u16)
                    .filter(|&i| mmu.ppu().vram()[i as usize] == 0x40 + i as u8)
                    .count();
                assert!(copied >= 0x10 * (scanline + 1), "scanline {scanline}: only {copied} bytes in");
                blocks_seen = copied / 0x10;
            }
            assert_eq!(blocks_seen, 4);
            assert_eq!(mmu.read(0xFF55), 0xFF, "done");
        }

        /// A VRAM DMA cannot source from VRAM itself, nor from the OAM/IO region.
        #[test]
        fn vram_dma_sources_read_back_as_ff() {
            let mut mmu = cgb();
            mmu.ppu.lcd_status_mut().set_mode(crate::lcd_status::LcdMode::VBlank);
            mmu.write(0xFF4F, 1);
            for i in 0..0x10u16 { mmu.write(0x9000 + i, 0x77); }
            mmu.write(0xFF4F, 0);

            mmu.write(0xFF51, 0x90);
            mmu.write(0xFF52, 0x00); // source 0x9000 — inside VRAM
            mmu.write(0xFF53, 0x00);
            mmu.write(0xFF54, 0x00);
            mmu.write(0xFF55, 0x00); // one block

            for i in 0..0x10u16 {
                assert_eq!(mmu.read(0x8000 + i), 0xFF, "byte {i}");
            }
        }

        /// B9. On CGB the unusable region is ordinary RAM, mirrored every 8 bytes within each
        /// 32-byte span; its power-on contents come from gambatte's hardware dump.
        #[test]
        fn the_unusable_region_is_mirrored_ram_on_cgb() {
            let mut mmu = cgb();
            for offset in 0..0x60usize {
                let expected = UNUSABLE_CGB[(offset / 32) * UNUSABLE_BLOCK + offset % UNUSABLE_BLOCK];
                assert_eq!(mmu.read(0xFEA0 + offset as u16), expected, "power-on byte {offset:02X}");
            }

            mmu.write(0xFEA0, 0x5A);
            assert_eq!(mmu.read(0xFEA0), 0x5A);
            assert_eq!(mmu.read(0xFEA8), 0x5A, "mirrored every 8 bytes...");
            assert_eq!(mmu.read(0xFEB8), 0x5A);
            assert_ne!(mmu.read(0xFEC0), 0x5A, "...but not across the 32-byte boundary");

            // DMG is unchanged: write-protected zeroes (A13).
            let mut dmg = MMU::from_rom(crate::roms::cgb_acid::ROM).unwrap();
            dmg.write(0xFEA0, 0x5A);
            assert_eq!(dmg.read(0xFEA0), 0x00);
        }

        /// B9. The undocumented CGB scratch registers, and their read-back masks.
        #[test]
        fn undocumented_registers_hold_what_is_written() {
            let mut mmu = cgb();
            for address in 0xFF72..=0xFF74u16 {
                mmu.write(address, 0xA5);
                assert_eq!(mmu.read(address), 0xA5, "{address:04X}");
            }
            mmu.write(0xFF75, 0xFF);
            assert_eq!(mmu.read(0xFF75), 0xFF, "FF75: only bits 4-6 are writable, the rest read 1");
            mmu.write(0xFF75, 0x00);
            assert_eq!(mmu.read(0xFF75), 0x8F);

            // None of them exist on a DMG.
            let mut dmg = MMU::from_rom(crate::roms::cgb_acid::ROM).unwrap();
            for address in 0xFF72..=0xFF75u16 {
                dmg.write(address, 0xA5);
                assert_eq!(dmg.read(address), 0xFF, "{address:04X} on DMG");
            }
        }

        /// B9. `SC` bit 1 selects the CGB's 32x shift clock, and only exists there.
        #[test]
        fn serial_runs_32x_faster_when_asked() {
            fn transfer_cycles(mut mmu: MMU, control: u8) -> usize {
                mmu.serial_mut().enable_buffer();
                mmu.write(0xFF01, 0x42);
                mmu.write(0xFF02, control);
                let mut cycles = 0;
                while mmu.serial().buffered_bytes().unwrap().is_empty() && cycles < 2000 {
                    mmu.update(MachineCycles::ONE);
                    cycles += 1;
                }
                cycles
            }

            let slow = transfer_cycles(cgb(), 0x81);
            let fast = transfer_cycles(cgb(), 0x83);
            assert_eq!(slow, MachineCycles::PER_SERIAL_BYTE_TRANSFER.m_cycles());
            assert_eq!(fast, slow / 32);

            // On a DMG bit 1 is not wired up: the transfer takes the same time either way.
            assert_eq!(
                transfer_cycles(MMU::from_rom(crate::roms::cgb_acid::ROM).unwrap(), 0x83),
                slow
            );
            assert_eq!(cgb().read(0xFF02) & 0x7C, 0x7C, "SC bits 2-6 read 1 on CGB");
        }
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