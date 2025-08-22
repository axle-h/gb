use crate::activation::Activation;

/// NRx3: period low
/// NRx4: period high & control
#[derive(Debug, Clone, Default)]
pub struct PeriodAndControlRegisters {
    period: u16, // 11 bits
    trigger: bool, // bit 7 of high byte
    length_enable: bool, // bit 6 of high byte
    pending_activation: bool,
}

impl PeriodAndControlRegisters {
    pub fn get_low(&self) -> u8 {
        (self.period & 0xFF) as u8 // Get the lower 8 bits
    }

    pub fn set_low(&mut self, value: u8) {
        self.period = (self.period & 0xFF00) | value as u16; // Set the lower 8 bits
    }

    pub fn get_high(&self) -> u8 {
        ((self.period >> 8) & 0x07) as u8
            | if self.trigger { 0x80 } else { 0 }
            | if self.length_enable { 0x40 } else { 0 }
    }

    pub fn set_high(&mut self, value: u8) {
        self.period = (self.period & 0x00FF) | ((value as u16 & 0x07) << 8); // Set the upper 3 bits
        self.trigger = (value & 0x80) != 0; // bit 7
        self.length_enable = (value & 0x40) != 0; // bit 6

        if self.trigger {
            self.pending_activation = true; // Set pending activation if trigger is set
        }
    }

    pub fn period(&self) -> u16 {
        self.period
    }

    pub fn set_period(&mut self, period: u16) {
        self.period = period & 0x07FF; // Only 11 bits are valid
    }

    pub fn length_enable(&self) -> bool {
        self.length_enable
    }
}

impl Activation for PeriodAndControlRegisters {
    fn is_activation_pending(&self) -> bool {
        self.pending_activation
    }

    fn clear_activation(&mut self) {
        self.pending_activation = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let registers = PeriodAndControlRegisters::default();
        assert_eq!(registers.get_low(), 0);
        assert_eq!(registers.get_high(), 0);
        assert!(!registers.is_activation_pending());
        assert!(!registers.length_enable());
        assert_eq!(registers.period(), 0);
    }

    #[test]
    fn get_and_set() {
        let mut registers = PeriodAndControlRegisters::default();
        registers.set_high(0xFF);
        assert!(registers.is_activation_pending()); // Bit 7 is set
        assert!(registers.length_enable()); // Bit 6 is set
        assert_eq!(registers.get_high(), 0xC7);
        assert_eq!(registers.period(), 0x0700);

        registers.set_low(0xFF);
        assert_eq!(registers.get_low(), 0xFF);
        assert_eq!(registers.period(), 0x7FF);
    }
}