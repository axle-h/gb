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

    /// Ticks remaining until the next phase advance. One tick is `1 / SPEED_MULTIPLIER` of an
    /// M-cycle, so for [`WavetableTimer`] it is 2 T-cycles — the resolution the DMG wave-RAM
    /// access aperture is defined at (see [`crate::audio::wave_channel::WaveChannel::wave_ram`]).
    pub fn counter(&self) -> u16 {
        self.counter
    }

    /// Ticks between phase advances.
    pub fn period(&self) -> u16 {
        self.period
    }

    /// Trigger, but hold the first phase advance back by `delay` extra ticks. The wave channel
    /// needs this: on DMG its first sample fetch after a trigger comes `period + 3` ticks later,
    /// not `period` (gambatte `channel3.cpp:69`).
    pub fn trigger_after(&mut self, delay: u16) {
        self.trigger();
        self.counter = self.period + delay;
    }

    pub fn phase(&self) -> u8 {
        self.phase
    }

    /// M-cycles until the next phase advance. One tick is `1 / SPEED_MULTIPLIER` of an M-cycle, so
    /// this rounds *up*: waking a fraction of a cycle early is harmless, waking late is not.
    pub fn machine_cycles_to_next_phase(&self) -> u64 {
        u64::from(self.counter).div_ceil(SPEED_MULTIPLIER as u64)
    }

    /// Advance by `machine_cycles`, returning whether the phase moved.
    ///
    /// **C3: closed form, no loop** — the same shape as gambatte's `DutyUnit::updatePos`
    /// (`sound/duty_unit.cpp:51-58`). The old `for _ in 0..ticks` ran once per M-cycle per channel
    /// and was measured at 7.7% of the whole emulator; four channels each stepping one tick at a
    /// time is most of what made the APU 37%.
    pub fn update(&mut self, machine_cycles: MachineCycles) -> bool {
        let ticks = machine_cycles.m_cycles() as u32 * SPEED_MULTIPLIER as u32;
        if ticks < u32::from(self.counter) {
            // The common case by far: no phase advance in this window.
            self.counter -= ticks as u16;
            return false;
        }

        // `counter` ticks to the *first* advance, then one every `period` after it.
        debug_assert!(self.period > 0, "a zero period would never reload");
        let period = u32::from(self.period);
        let past_first = ticks - u32::from(self.counter);
        if past_first < period {
            // ⚠️ Not merely a shortcut — **this is the case that matters**. The emulator drives the
            // APU one M-cycle at a time, so a window almost never spans two advances, and the
            // general form below costs two `u32` divisions (~20 cycles each) where the old
            // one-iteration loop cost about two. Measured: without this, `cpu_instrs` lost 13%.
            self.counter = self.period - past_first as u16;
            self.phase = (self.phase + 1) & MAX_PHASE;
        } else {
            let advances = 1 + past_first / period;
            self.counter = self.period - (past_first % period) as u16;
            self.phase = (self.phase + (advances % (u32::from(MAX_PHASE) + 1)) as u8) & MAX_PHASE;
        }
        true
    }
}

pub type PulseTimer = PhaseTimer<7, 1>;
pub type WavetableTimer = PhaseTimer<31, 2>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-C3 implementation, kept as the oracle. The closed form has to agree with it on
    /// every field, not just the phase — `just_reloaded()` reads `counter` and the DMG wave-RAM
    /// aperture is one tick wide, so being one off is audible.
    fn stepped<const MAX_PHASE: u8, const SPEED: usize>(
        timer: &mut PhaseTimer<MAX_PHASE, SPEED>,
        machine_cycles: MachineCycles,
    ) -> bool {
        let mut clocked = false;
        for _ in 0..machine_cycles.m_cycles() as usize * SPEED {
            timer.counter -= 1;
            if timer.counter == 0 {
                timer.counter = timer.period;
                timer.phase = (timer.phase + 1) & MAX_PHASE;
                clocked = true;
            }
        }
        clocked
    }

    fn check<const MAX_PHASE: u8, const SPEED: usize>(frequency: u16, delta: u64) {
        let mut closed = PhaseTimer::<MAX_PHASE, SPEED>::new();
        closed.set_frequency(frequency);
        closed.trigger();
        let mut oracle = closed;

        // Several windows in a row: the interesting disagreements are in the carried remainder,
        // and a single call from a freshly triggered timer would not expose them.
        for step in 0..6 {
            let cycles = MachineCycles::from_m(delta + step);
            assert_eq!(closed.update(cycles), stepped(&mut oracle, cycles),
                "clocked, freq {frequency} delta {delta} step {step}");
            assert_eq!(closed, oracle, "state, freq {frequency} delta {delta} step {step}");
        }
    }

    #[test]
    fn the_closed_form_matches_the_old_loop() {
        // Periods from the shortest a channel can select to the longest, and windows from one
        // M-cycle (how the emulator actually drives it) up past a whole period.
        for frequency in [0, 1, 1024, 2040, 2046, 2047] {
            for delta in [1, 2, 3, 4, 7, 16, 63, 64, 1024, 2047, 2048, 5000] {
                check::<7, 1>(frequency, delta); // PulseTimer
                check::<31, 2>(frequency, delta); // WavetableTimer
            }
        }
    }

    /// `trigger_after` starts the counter *above* the period, which is the one state where the
    /// first window is longer than every later one.
    #[test]
    fn the_closed_form_matches_the_old_loop_after_a_delayed_trigger() {
        for frequency in [0, 2000, 2047] {
            for delay in [1, 3, 100] {
                for delta in [1, 5, 2048] {
                    let mut closed = WavetableTimer::new();
                    closed.set_frequency(frequency);
                    closed.trigger_after(delay);
                    let mut oracle = closed;
                    for _ in 0..4 {
                        let cycles = MachineCycles::from_m(delta);
                        assert_eq!(closed.update(cycles), stepped(&mut oracle, cycles));
                        assert_eq!(closed, oracle, "freq {frequency} delay {delay} delta {delta}");
                    }
                }
            }
        }
    }
}