use std::time::Duration;
use crate::cycles::MachineCycles;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DelayContext {
    cycles: MachineCycles,
}

const SHORT_DELAY_CYCLES: MachineCycles = MachineCycles::from_duration(Duration::from_millis(50));
const DEFAULT_DELAY_CYCLES: MachineCycles = MachineCycles::from_duration(Duration::from_millis(500));
const LONG_DELAY_CYCLES: MachineCycles = MachineCycles::from_duration(Duration::from_millis(1000));

impl Default for DelayContext {
    fn default() -> Self {
        Self {
            cycles: DEFAULT_DELAY_CYCLES,
        }
    }
}

impl DelayContext {
    pub const fn new(cycles: MachineCycles) -> Self {
        Self { cycles }
    }

    pub const ZERO: Self = Self::new(MachineCycles::ZERO);

    pub fn short() -> Self {
        Self { cycles: SHORT_DELAY_CYCLES }
    }

    pub fn long() -> Self {
        Self { cycles: LONG_DELAY_CYCLES }
    }

    pub fn tick(&mut self, cycles: MachineCycles) -> bool {
        if self.cycles == MachineCycles::ZERO {
            true
        } else if self.cycles > cycles {
            self.cycles = self.cycles - cycles;
            false
        } else {
            self.cycles = MachineCycles::ZERO;
            true
        }
    }

    pub fn is_exhausted(&self) -> bool {
        self.cycles == MachineCycles::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> MachineCycles {
        MachineCycles::from_duration(Duration::from_millis(n))
    }

    #[test]
    fn zero_delay_fires_immediately() {
        let mut d = DelayContext::new(MachineCycles::ZERO);
        assert!(d.tick(ms(1)));
    }

    #[test]
    fn zero_delay_keeps_firing() {
        let mut d = DelayContext::new(MachineCycles::ZERO);
        for _ in 0..5 {
            assert!(d.tick(ms(16)));
        }
    }

    #[test]
    fn default_delay_does_not_fire_on_first_small_tick() {
        let mut d = DelayContext::default();
        assert!(!d.tick(ms(1)));
    }

    #[test]
    fn default_delay_fires_after_exact_duration() {
        let mut d = DelayContext::default(); // 250 ms
        assert!(d.tick(ms(250)));
    }

    #[test]
    fn default_delay_fires_after_accumulated_ticks() {
        let mut d = DelayContext::default(); // 250 ms
        assert!(!d.tick(ms(100)));
        assert!(!d.tick(ms(100)));
        assert!(d.tick(ms(100))); // 50 ms over — still fires
    }

    #[test]
    fn large_tick_completes_delay_immediately() {
        let mut d = DelayContext::default();
        assert!(d.tick(ms(1000)));
    }

    #[test]
    fn delay_keeps_firing_after_completion() {
        let mut d = DelayContext::default();
        assert!(d.tick(ms(250)));
        assert!(d.tick(ms(16)));
        assert!(d.tick(ms(16)));
    }

    #[test]
    fn custom_delay_counts_down_correctly() {
        // Use a total smaller than 2×tick so the third tick definitely overshoots.
        let mut d = DelayContext::new(ms(50));
        assert!(!d.tick(ms(20))); // ~30 ms left
        assert!(!d.tick(ms(20))); // ~10 ms left
        assert!(d.tick(ms(20)));  // overshoots → fires
    }
}