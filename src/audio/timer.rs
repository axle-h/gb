use bincode::{Decode, Encode};
use crate::cycles::MachineCycles;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Decode, Encode)]
pub struct PhaseTimer<const MAX_PHASE: u8, const SPEED_MULTIPLIER: usize> {
    phase: u8,
    frequency: u16,
    counter: u16,
    period: u16,
}

impl<const MAX_PHASE: u8, const SPEED_MULTIPLIER: usize> Default for PhaseTimer<MAX_PHASE, SPEED_MULTIPLIER> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const MAX_PHASE: u8, const SPEED_MULTIPLIER: usize> PhaseTimer<MAX_PHASE, SPEED_MULTIPLIER> {
    pub fn new() -> Self {
        // Sanity check that (MAX_PHASE + 1) is a power of 2
        assert_eq!(MAX_PHASE.trailing_ones() + MAX_PHASE.leading_zeros(), u8::BITS);

        Self { phase: 0, counter: 2048, period: 2048, frequency: 0 }
    }

    pub fn just_reloaded(self) -> bool {
        self.counter == self.period
    }

    pub fn frequency(self) -> u16 {
        self.frequency
    }

    pub fn set_frequency(&mut self, value: u16) {
        self.frequency = value;
        self.period = 2048 - value;
    }

    pub fn trigger(&mut self) {
        // TODO When triggering Ch1 and Ch2, the low two bits of the frequency timer are NOT modified.
        self.phase = 0;
        self.counter = self.period;
    }

    pub fn phase(&self) -> u8 {
        self.phase
    }

    pub fn update(&mut self, machine_cycles: MachineCycles) -> bool {
        let mut clocked = false;

        let ticks = machine_cycles.m_cycles() * SPEED_MULTIPLIER;
        for _ in 0..ticks {
            self.counter -= 1;
            if self.counter == 0 {
                self.counter = self.period;
                self.phase = (self.phase + 1) & MAX_PHASE;
                clocked = true;
            }
        }

        clocked
    }
}

pub type PulseTimer = PhaseTimer<7, 1>;
pub type WavetableTimer = PhaseTimer<31, 2>;