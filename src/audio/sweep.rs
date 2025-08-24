/// FF10 — NR10: Channel 1 sweep
#[derive(Debug, Clone, Default)]
pub struct SweepRegister {
    /// Pace: This dictates how often sweep “iterations” happen, in units of 128 Hz ticks5 (7.8 ms).
    /// A value of 0 disables the sweep.
    /// bits 4-6
    sweep_period: u8,
    /// Direction: 0 = Addition (period increases); 1 = Subtraction (period decreases)
    /// bit 3
    subtraction: bool,
    /// Individual step: On each iteration, the new period Lt+1 is computed from the current one Lt as follows:
    /// L[t+1] = L[t] + L[t] / 2^step if direction is 0 (addition)
    /// L[t+1] = L[t] - L[t] / 2^step if direction is 1 (subtraction)
    /// bits 0-2
    individual_step: u8,
}

impl SweepRegister {
    pub fn get(&self) -> u8 {
        let mut byte = 0;
        byte |= (self.sweep_period & 0x07) << 4; // Bits 4-6: Pace
        if self.subtraction {
            byte |= 0x08; // Bit 3: Direction (1 = Subtraction)
        }
        byte |= self.individual_step & 0x07; // Bits 0-2: Step
        byte
    }

    pub fn set(&mut self, value: u8) {
        self.sweep_period = (value >> 4) & 0x07; // Bits 4-6
        self.subtraction = (value & 0x08) != 0; // Bit 3
        self.individual_step = value & 0x07; // Bits 0-2
    }

    pub fn sweep_period(&self) -> u8 {
        if self.sweep_period == 0 { 8 } else { self.sweep_period }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sweep {
    register: SweepRegister,
    enabled: bool,
    shadow_period: usize,
    sweep_timer: u8,
}

impl Sweep {
    pub fn register(&self) -> &SweepRegister {
        &self.register
    }

    pub fn register_mut(&mut self) -> &mut SweepRegister {
        &mut self.register
    }

    pub fn reset(&mut self, period: usize) -> SweepResult {
        self.shadow_period = period;
        self.sweep_timer = self.register.sweep_period();
        self.enabled = self.register.sweep_period != 0 || self.register.individual_step != 0;

        // If the individual step is non-zero, frequency calculation and overflow check are performed immediately.
        if self.register.individual_step > 0 {
            self.calculate_period()
        } else {
            SweepResult::new(self.shadow_period) // No change
        }
    }

    pub fn step(&mut self) -> Option<SweepResult> {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }

        if self.sweep_timer != 0 {
            return None;
        }

        self.sweep_timer = self.register.sweep_period();

        if !self.enabled || self.register.sweep_period == 0 {
            return None;
        }

        let mut next_period = self.calculate_period();

        if !next_period.overflows && self.register.individual_step > 0 {
            self.shadow_period = next_period.value;
            // 2nd overflow check
            next_period.overflows = self.calculate_period().overflows;
        }

        Some(next_period)
    }

    fn calculate_period(&mut self) -> SweepResult {
        let next_period = self.shadow_period >> self.register.individual_step;
        let result = SweepResult::new(
            if self.register.subtraction {
                self.shadow_period.wrapping_sub(next_period)
            } else {
                self.shadow_period.wrapping_add(next_period)
            }
        );
        if result.overflows {
            self.enabled = false;
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepResult {
    pub value: usize,
    pub overflows: bool,
}

impl SweepResult {
    pub fn new(value: usize) -> Self {
        Self {
            value,
            overflows: value > 0x7FF,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default() {
        let register = SweepRegister::default();
        assert_eq!(register.sweep_period, 0);
        assert_eq!(register.subtraction, false);
        assert_eq!(register.individual_step, 0);
        assert_eq!(register.get(), 0);
    }

    #[test]
    fn bit_masking() {
        let mut register = SweepRegister::default();

        // Test that unused bits are ignored/masked
        register.set(0b11111111); // All bits set, including unused bit 7
        assert_eq!(register.sweep_period, 7); // Only bits 4-6 used
        assert_eq!(register.subtraction, true); // Only bit 3 used
        assert_eq!(register.individual_step, 7); // Only bits 0-2 used
        assert_eq!(register.get(), 0b01111111); // Bit 7 should not be set in output
    }

    #[test]
    fn individual_field_isolation() {
        let mut register = SweepRegister::default();

        // Set pace only
        register.set(0b01110000); // pace: 7, direction: 0, step: 0
        assert_eq!(register.sweep_period, 7);
        assert_eq!(register.subtraction, false);
        assert_eq!(register.individual_step, 0);

        // Set direction only
        register.set(0b00001000); // pace: 0, direction: 1, step: 0
        assert_eq!(register.sweep_period, 0);
        assert_eq!(register.subtraction, true);
        assert_eq!(register.individual_step, 0);

        // Set step only
        register.set(0b00000111); // pace: 0, direction: 0, step: 7
        assert_eq!(register.sweep_period, 0);
        assert_eq!(register.subtraction, false);
        assert_eq!(register.individual_step, 7);
    }

    #[test]
    fn round_trip_consistency() {
        let mut register = SweepRegister::default();

        // Test that set followed by get returns the same value for all valid combinations
        for value in 0..=0b01111111 { // Only test valid 7-bit values
            register.set(value);
            assert_eq!(register.get(), value, "Round trip failed for value: {:#08b}", value);
        }
    }

}
