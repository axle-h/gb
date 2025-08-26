/// NRX1 - Length Timer and Duty Cycle Register
/// Channel 1 & 2 only
#[derive(Debug, Clone, Default)]
pub struct LengthTimerAndDutyCycleRegister {
    /// bits 6-7 Duty cycle
    /// Controls the output waveform as follows:
    /// - 00: 12.5% duty cycle
    /// - 01: 25% duty cycle
    /// - 10: 50% duty cycle
    /// - 11: 75% duty cycle
    wave_duty_cycle: u8,

    /// bits 0-5 Initial length timer
    /// The higher this field is, the shorter the time before the channel is cut.
    initial_length_timer: u8,
}

impl LengthTimerAndDutyCycleRegister {
    pub fn get(&self) -> u8 {
        // write only bits 0-5 always read as 1
        0x3F | ((self.wave_duty_cycle & 0x03) << 6) // Bits 6-7: Wave duty cycle
    }

    pub fn set(&mut self, value: u8) {
        self.wave_duty_cycle = (value >> 6) & 0x03; // Bits 6-7
        self.initial_length_timer = value & 0x3F; // Bits 0-5
    }

    pub fn wave_duty_cycle(&self) -> u8 {
        self.wave_duty_cycle
    }

    pub fn initial_length_timer(&self) -> u8 {
        self.initial_length_timer
    }

    pub fn waveform(&self) -> u8 {
        match self.wave_duty_cycle {
            0 => 0b00000001, // 12.5% duty cycle
            1 => 0b00000011, // 25% duty cycle
            2 => 0b00001111, // 50% duty cycle
            3 => 0b11111100, // 75% duty cycle
            _ => unreachable!(), // Should never happen
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LengthTimer {
    offset: u16,
    value: u16
}

impl LengthTimer {
    pub fn square_channel(register: &LengthTimerAndDutyCycleRegister) -> Self {
        Self::new(64, register.initial_length_timer)
    }

    pub fn wave_channel(initial_length_timer: u8) -> Self {
        Self::new(256, initial_length_timer)
    }

    fn new(offset: u16, initial_length_timer: u8) -> Self {
        Self { value: offset - initial_length_timer as u16, offset }
    }

    pub fn reset(&mut self, initial_length_timer: u8) {
        // If length timer expired it is reset.
        if self.value == 0 {
            self.value = self.offset - initial_length_timer as u16;
        }
    }

    pub fn step(&mut self) -> bool {
        if self.value > 0 {
            self.value -= 1;
        }
        self.value == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let register = LengthTimerAndDutyCycleRegister::default();
        assert_eq!(register.get(), 0x3F);
        assert_eq!(register.wave_duty_cycle(), 0);
        assert_eq!(register.initial_length_timer(), 0);
    }

    #[test]
    fn set_and_get_wave_duty_cycle() {
        let mut register = LengthTimerAndDutyCycleRegister::default();
        register.set(0b11000000); // Set duty cycle to 11 (75%)
        assert_eq!(register.get(), 0xFF); // Bits 0-5 are always set
        assert_eq!(register.wave_duty_cycle(), 3); // 11 in binary
    }

    #[test]
    fn set_and_get_initial_length_timer() {
        let mut register = LengthTimerAndDutyCycleRegister::default();
        register.set(0x3F);
        assert_eq!(register.get(), 0x3F);
        assert_eq!(register.initial_length_timer(), 0x3F);
    }


    #[test]
    fn set_and_get_all() {
        let mut register = LengthTimerAndDutyCycleRegister::default();
        register.set(0xFF);
        assert_eq!(register.get(), 0xFF);
        assert_eq!(register.wave_duty_cycle(), 3);
        assert_eq!(register.initial_length_timer(), 0x3F);
    }
}