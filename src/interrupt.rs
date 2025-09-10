use bincode::{Decode, Encode};
use strum::IntoEnumIterator;

/// https://gbdev.io/pandocs/Interrupts.html#ffff--ie-interrupt-enable
#[derive(Debug, Clone, Copy, PartialEq, Eq, Decode, Encode)]
pub struct InterruptFlags {
    joypad: bool,
    serial: bool,
    timer: bool,
    lcd_stat: bool,
    v_blank: bool,
}

impl Default for InterruptFlags {
    fn default() -> Self {
        Self {
            joypad: false,
            serial: false,
            timer: false,
            lcd_stat: false,
            v_blank: false,
        }
    }
}

impl InterruptFlags {
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

    pub fn is_set(&self, interrupt: InterruptType) -> bool {
        match interrupt {
            InterruptType::VBlank => self.v_blank,
            InterruptType::LcdStatus => self.lcd_stat,
            InterruptType::Timer => self.timer,
            InterruptType::Serial => self.serial,
            InterruptType::Joypad => self.joypad,
        }
    }

    pub fn clear_interrupt(&mut self, interrupt: InterruptType) {
        match interrupt {
            InterruptType::VBlank => self.v_blank = false,
            InterruptType::LcdStatus => self.lcd_stat = false,
            InterruptType::Timer => self.timer = false,
            InterruptType::Serial => self.serial = false,
            InterruptType::Joypad => self.joypad = false,
        }
    }

    pub fn set_interrupt(&mut self, interrupt: InterruptType) {
        match interrupt {
            InterruptType::VBlank => self.v_blank = true,
            InterruptType::LcdStatus => self.lcd_stat = true,
            InterruptType::Timer => self.timer = true,
            InterruptType::Serial => self.serial = true,
            InterruptType::Joypad => self.joypad = true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::EnumIter)]
pub enum InterruptType {
    VBlank,
    LcdStatus,
    Timer,
    Serial,
    Joypad,
}

impl InterruptType {
    pub fn all() -> InterruptTypeIter {
        Self::iter()
    }

    pub fn address(self) -> u16 {
        match self {
            InterruptType::VBlank => 0x0040,
            InterruptType::LcdStatus => 0x0048,
            InterruptType::Timer => 0x0050,
            InterruptType::Serial => 0x0058,
            InterruptType::Joypad => 0x0060,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupt_flags() {
        let mut flags = InterruptFlags::default();
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
}