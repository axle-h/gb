use bincode::{Decode, Encode};

/// FF10 — NR10: Channel 1 sweep
#[derive(Debug, Clone, Default, Eq, PartialEq, Decode, Encode)]
pub struct Sweep {
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

    // internal state
    enabled: bool,
    shadow_period: u16,
    sweep_timer: u8,
    calculated_with_negate_since_trigger: bool,
}

impl Sweep {
    pub fn nr10(&self) -> u8 {
        let mut byte = 0x80; // Bit 7: Unused, always 1
        byte |= (self.sweep_period & 0x07) << 4; // Bits 4-6: Pace
        if self.subtraction {
            byte |= 0x08; // Bit 3: Direction (1 = Subtraction)
        }
        byte |= self.individual_step & 0x07; // Bits 0-2: Step
        byte
    }

    pub fn set_nr10(&mut self, value: u8, channel_active: &mut bool) {
        self.sweep_period = (value >> 4) & 0x07; // Bits 4-6
        self.subtraction = (value & 0x08) != 0; // Bit 3
        self.individual_step = value & 0x07; // Bits 0-2

        if self.sweep_timer == 0 {
            self.reset_sweep_timer();
        }

        if self.calculated_with_negate_since_trigger && !self.subtraction {
            // If the negate flag is cleared after frequency was calculated with it set at least
            // once, the channel is immediately disabled
            *channel_active = false;
        }
    }

    fn reset_sweep_timer(&mut self) {
        self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
    }

    pub fn trigger(&mut self, period: u16) -> SweepResult {
        self.calculated_with_negate_since_trigger = false;
        self.shadow_period = period;
        self.reset_sweep_timer();
        self.enabled = self.sweep_period != 0 || self.individual_step != 0;

        // If the individual step is non-zero, frequency calculation and overflow check are performed immediately.
        if self.individual_step > 0 {
            self.calculate_period()
        } else {
            SweepResult::new(self.shadow_period) // No change
        }
    }

    pub fn clock(&mut self) -> Option<SweepResult> {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }

        if self.sweep_timer != 0 {
            return None;
        }

        self.reset_sweep_timer();

        if !self.enabled || self.sweep_period == 0 {
            return None;
        }

        let mut next_period = self.calculate_period();

        if !next_period.overflows && self.individual_step > 0 {
            self.shadow_period = next_period.value;
            // 2nd overflow check
            next_period.overflows = self.calculate_period().overflows;
        }

        Some(next_period)
    }

    fn calculate_period(&mut self) -> SweepResult {
        let next_period = self.shadow_period >> self.individual_step;
        let result = SweepResult::new(
            if self.subtraction {
                self.calculated_with_negate_since_trigger = true;
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
    pub value: u16,
    pub overflows: bool,
}

impl SweepResult {
    pub fn new(value: u16) -> Self {
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
        let register = Sweep::default();
        assert_eq!(register.sweep_period, 0);
        assert_eq!(register.subtraction, false);
        assert_eq!(register.individual_step, 0);
        assert_eq!(register.nr10(), 0x80); // Bit 7 is always set
    }

    #[test]
    fn bit_masking() {
        let mut register = Sweep::default();

        // Test that unused bits are ignored/masked
        register.set_nr10(0b11111111, &mut false); // All bits set, including unused bit 7
        assert_eq!(register.sweep_period, 7); // Only bits 4-6 used
        assert_eq!(register.subtraction, true); // Only bit 3 used
        assert_eq!(register.individual_step, 7); // Only bits 0-2 used
        assert_eq!(register.nr10(), 0b11111111); // Bit 7 is always set
    }

    #[test]
    fn individual_field_isolation() {
        let mut register = Sweep::default();

        // Set pace only
        register.set_nr10(0b01110000, &mut false); // pace: 7, direction: 0, step: 0
        assert_eq!(register.sweep_period, 7);
        assert_eq!(register.subtraction, false);
        assert_eq!(register.individual_step, 0);

        // Set direction only
        register.set_nr10(0b00001000, &mut false); // pace: 0, direction: 1, step: 0
        assert_eq!(register.sweep_period, 0);
        assert_eq!(register.subtraction, true);
        assert_eq!(register.individual_step, 0);

        // Set step only
        register.set_nr10(0b00000111, &mut false); // pace: 0, direction: 0, step: 7
        assert_eq!(register.sweep_period, 0);
        assert_eq!(register.subtraction, false);
        assert_eq!(register.individual_step, 7);
    }

    #[test]
    fn round_trip_consistency() {
        let mut register = Sweep::default();

        // Test that set followed by get returns the same value for all valid combinations
        for value in 0..=0b01111111 {
            register.set_nr10(value, &mut false);
            assert_eq!(register.nr10(), value | 0x80, "Round trip failed for value: {:#08b}", value);
        }
    }

}
