use crate::activation::Activation;
/// https://gbdev.io/pandocs/STAT.html
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LcdStatus {
    ly: u8,   // Current line (read only)
    lyc: u8,  // LY Compare (read-write)
    mode: LcdMode, // bit 0-1: LCD Mode (read only)
    hblank_interrupt: bool, // bit 3: Mode 0 interrupt (HBlank)
    vblank_interrupt: bool, // bit 4: Mode 1 interrupt (VBlank)
    oam_interrupt: bool, // bit 5: Mode 2 interrupt (OAM)
    lyc_interrupt: bool, // bit 6: LYC=LY interrupt
    interrupt_pending: bool, // Indicates if any interrupt is pending
}

impl LcdStatus {
    pub fn ly(&self) -> u8 {
        self.ly
    }

    pub fn increment_ly(&mut self) -> u8 {
        // this should only be called by the PPU during rendering
        self.ly += 1;
        if self.ly > 153 {
            self.ly = 0; // wrap around after VBlank
        }
        self.check_lyc_interrupt();
        self.ly
    }

    pub fn lyc(&self) -> u8 {
        self.lyc
    }

    pub fn set_lyc(&mut self, value: u8) {
        self.lyc = value;
        self.check_lyc_interrupt();
    }

    pub fn mode(&self) -> LcdMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: LcdMode) {
        if self.mode == mode {
            return; // no change
        }
        self.mode = mode;

        // check interrupt
        // TODO emulate STAT blocking
        self.interrupt_pending |= match mode {
            LcdMode::HBlank => self.hblank_interrupt,
            LcdMode::VBlank => self.vblank_interrupt,
            LcdMode::OAM => self.oam_interrupt,
            LcdMode::Drawing => false
        };
    }

    pub fn stat(&self) -> u8 {
        (self.mode as u8) & 0x03 // bits 0-1 for mode
            | ((self.lyc == self.ly) as u8) << 2 // bit 2: LYC=LY flag
            | (self.hblank_interrupt as u8) << 3 // bit 3: HBlank interrupt
            | (self.vblank_interrupt as u8) << 4 // bit 4: VBlank interrupt
            | (self.oam_interrupt as u8) << 5 // bit 5: OAM interrupt
            | (self.lyc_interrupt as u8) << 6 // bit 6: LYC=LY interrupt
    }

    pub fn set_stat(&mut self, value: u8) {
        // only the interrupt flags, bits 3-6, are writable
        self.hblank_interrupt = (value & 0x08) != 0;
        self.vblank_interrupt = (value & 0x10) != 0;
        self.oam_interrupt = (value & 0x20) != 0;
        self.lyc_interrupt = (value & 0x40) != 0;
    }

    fn check_lyc_interrupt(&mut self) {
        self.interrupt_pending |= self.lyc_interrupt && self.lyc == self.ly;
    }
}

impl Activation for LcdStatus {
    fn is_activation_pending(&self) -> bool {
        self.interrupt_pending
    }

    fn clear_activation(&mut self) {
        self.interrupt_pending = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, strum_macros::FromRepr)]
#[repr(u8)]
pub enum LcdMode {
    #[default]
    HBlank = 0,
    VBlank = 1,
    OAM = 2,
    Drawing = 3,
}

impl LcdMode {
    pub fn vram_accessible(self) -> bool {
        self != LcdMode::Drawing
    }

    pub fn oam_accessible(self) -> bool {
        self == LcdMode::HBlank || self == LcdMode::VBlank
    }
}