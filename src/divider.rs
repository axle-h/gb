use crate::cycles::MachineCycles;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    enabled: bool,
    value: u8,
    cycles_since_tick: MachineCycles,
}

impl Default for Divider {
    fn default() -> Self {
        Self {
            enabled: true,
            value: 0,
            cycles_since_tick: MachineCycles::ZERO,
        }
    }
}

impl Divider {
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.value = 0;
        self.enabled = false;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn reset(&mut self) {
        self.value = 0;
    }

    pub fn value(&self) -> u8 {
        self.value
    }

    pub fn update(&mut self, cycles: MachineCycles) -> DividerClocks {
        let mut result = DividerClocks { initial_value: self.value, count: 0 };
        if !self.enabled {
            return result;
        }
        self.cycles_since_tick += cycles;
        while self.cycles_since_tick >= MachineCycles::PER_DIVIDER_TICK {
            result.count += 1;
            self.cycles_since_tick -= MachineCycles::PER_DIVIDER_TICK;
            self.value = self.value.wrapping_add(1);
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DividerClocks {
    pub initial_value: u8,
    pub count: usize
}

impl DividerClocks {
    pub const ZERO: Self = Self { initial_value: 0, count: 0 };

    /// Checks if the specified bit transitions from 1 to 0 at any point during the clock iterations.
    /// # Arguments
    /// * `bit` - The bit position to check (0-7 for u8)
    pub fn bit_fall_edge(&self, bit: u8) -> usize {
        debug_assert!(bit < 8, "Bit position must be between 0 and 7");

        let bit_mask = 1u8 << bit;
        let mut prev_bit_set = (self.initial_value & bit_mask) != 0;
        let mut result = 0;
        for delta in 1..=self.count {
            let current_value = self.initial_value.wrapping_add(delta as u8);
            let current_bit_set = (current_value & bit_mask) != 0;
            if prev_bit_set && !current_bit_set {
                // 1 -> 0 transition
                result += 1;
            }
            prev_bit_set = current_bit_set;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state() {
        let divider = Divider::default();
        assert!(divider.is_enabled());
        assert_eq!(divider.value(), 0);
    }

    #[test]
    fn enable_disable() {
        let mut divider = Divider::default();
        assert!(divider.is_enabled());

        divider.disable();
        assert!(!divider.is_enabled());
        assert_eq!(
            divider.update(MachineCycles::PER_DIVIDER_TICK),
            DividerClocks { initial_value: 0, count: 0 }
        );
        assert_eq!(divider.value(), 0);

        divider.enable();
        assert!(divider.is_enabled());
        assert_eq!(
            divider.update(MachineCycles::PER_DIVIDER_TICK),
            DividerClocks { initial_value: 0, count: 1 }
        );
        assert_eq!(divider.value(), 1);
    }

    #[test]
    fn wraps() {
        let mut divider = Divider::default();
        for i in 0..0xff {
            let clocks = divider.update(MachineCycles::PER_DIVIDER_TICK);
            assert_eq!(clocks, DividerClocks { initial_value: i, count: 1 });
            assert_eq!(divider.value(), i + 1);
        }
        let clocks = divider.update(MachineCycles::PER_DIVIDER_TICK);
        assert_eq!(clocks, DividerClocks { initial_value: 0xFF, count: 1 });
        assert_eq!(divider.value(), 0);
    }


    #[test]
    fn bit_fall_edge() {
        let mut count = 0;
        for i in 0..=0xff {
            let clocks = DividerClocks { initial_value: i, count: 1 };
            count += clocks.bit_fall_edge(4);
        }
        // There are 8 transitions from 1 to 0 for bit 4 in a full cycle of u8
        // this is used by the audio frame sequencer derive a 512hz clock
        assert_eq!(count, 8);
    }
}
