use bincode::{Decode, Encode};
use crate::cycles::MachineCycles;
use crate::schedule::DISABLED;

/// One DIV period, in m-cycles: 16384 Hz off a 4 MHz clock.
const PERIOD: i64 = MachineCycles::PER_DIVIDER_TICK.m_cycles() as i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    enabled: bool,
    value: u8,
    /// When DIV next increments — an absolute m-cycle stamp while it runs, the cycles the
    /// interrupted period still owes while `STOP` has it halted. Exactly
    /// [`crate::timer::Timer::next_tick`]'s two origins, and for the same reason.
    next_tick: i64,
}

/// The serialised shape of a [`Divider`]: field-for-field what the `timer` save-state section has
/// always held. See [`crate::timer::TimerSnapshot`] for why C1 needed one.
#[derive(Debug, Clone, Copy, Decode, Encode)]
pub struct DividerSnapshot {
    enabled: bool,
    value: u8,
    cycles_since_tick: MachineCycles,
}

impl Default for Divider {
    fn default() -> Self {
        Self { enabled: true, value: 0, next_tick: PERIOD }
    }
}

impl Divider {
    fn origin(enabled: bool, now: u64) -> i64 {
        if enabled { now as i64 } else { 0 }
    }

    pub fn snapshot(&self, now: u64) -> DividerSnapshot {
        let owed = self.next_tick - Self::origin(self.enabled, now);
        DividerSnapshot {
            enabled: self.enabled,
            value: self.value,
            cycles_since_tick: MachineCycles::from_m((PERIOD - owed).max(0) as u64),
        }
    }

    pub fn restore(&mut self, snapshot: DividerSnapshot, now: u64) {
        self.enabled = snapshot.enabled;
        self.value = snapshot.value;
        self.next_tick = Self::origin(self.enabled, now) + PERIOD
            - snapshot.cycles_since_tick.m_cycles() as i64;
    }

    /// ⚠️ Catch the divider up to `now` before either of these, or the part-period it is in the
    /// middle of would be rebased from a stale deadline.
    pub fn enable(&mut self, now: u64) {
        if !self.enabled {
            self.next_tick += now as i64;
            self.enabled = true;
        }
    }

    pub fn disable(&mut self, now: u64) {
        if self.enabled {
            self.next_tick -= now as i64;
            self.enabled = false;
        }
        self.value = 0;
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

    /// Advance DIV to absolute m-cycle `now`, reporting the ticks that happened on the way — the
    /// APU frame sequencer is clocked off DIV bit 4, so the *edges* matter, not just the value.
    #[inline]
    pub fn catch_up(&mut self, now: u64) -> DividerClocks {
        let mut result = DividerClocks { initial_value: self.value, count: 0 };
        if !self.enabled {
            return result;
        }
        let now = now as i64;
        while self.next_tick <= now {
            self.next_tick += PERIOD;
            result.count += 1;
            self.value = self.value.wrapping_add(1);
        }
        result
    }

    /// Absolute m-cycle at which DIV next increments, or [`DISABLED`] while it is stopped.
    pub fn next_event(&self) -> u64 {
        if !self.enabled {
            return DISABLED;
        }
        self.next_tick.max(0) as u64
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

    /// One whole DIV period, in absolute m-cycles.
    const TICK: u64 = MachineCycles::PER_DIVIDER_TICK.m_cycles();

    #[test]
    fn enable_disable() {
        let mut divider = Divider::default();
        assert!(divider.is_enabled());

        divider.disable(0);
        assert!(!divider.is_enabled());
        assert_eq!(divider.catch_up(TICK), DividerClocks { initial_value: 0, count: 0 });
        assert_eq!(divider.value(), 0);
        assert_eq!(divider.next_event(), DISABLED);

        divider.enable(TICK);
        assert!(divider.is_enabled());
        assert_eq!(divider.catch_up(2 * TICK), DividerClocks { initial_value: 0, count: 1 });
        assert_eq!(divider.value(), 1);
    }

    /// `STOP` freezes the part-period DIV is in the middle of rather than restarting it.
    #[test]
    fn stopping_mid_period_keeps_what_has_elapsed() {
        let mut divider = Divider::default();
        divider.catch_up(TICK + 10); // one tick, then 10 cycles into the next period
        assert_eq!(divider.value(), 1);

        divider.disable(TICK + 10); // ...which also zeroes the visible counter
        divider.enable(9999);
        assert_eq!(divider.catch_up(9999 + TICK - 11), DividerClocks::ZERO, "one cycle short");
        assert_eq!(divider.catch_up(9999 + TICK - 10).count, 1);
    }

    #[test]
    fn wraps() {
        let mut divider = Divider::default();
        for i in 0..0xff {
            let clocks = divider.catch_up(u64::from(i + 1) * TICK);
            assert_eq!(clocks, DividerClocks { initial_value: i, count: 1 });
            assert_eq!(divider.value(), i + 1);
        }
        let clocks = divider.catch_up(0x100 * TICK);
        assert_eq!(clocks, DividerClocks { initial_value: 0xFF, count: 1 });
        assert_eq!(divider.value(), 0);
    }

    /// C1: the scheduler asks DIV when it next moves; being one cycle out here would let C2's
    /// HALT skip land past a frame-sequencer step.
    #[test]
    fn next_event_names_the_cycle_div_increments() {
        let mut divider = Divider::default();
        assert_eq!(divider.next_event(), TICK);

        divider.catch_up(TICK - 1);
        assert_eq!(divider.value(), 0);
        assert_eq!(divider.next_event(), TICK);

        divider.catch_up(TICK);
        assert_eq!(divider.value(), 1);
        assert_eq!(divider.next_event(), 2 * TICK);
    }

    #[test]
    fn a_snapshot_round_trips_against_a_restored_clock() {
        let mut divider = Divider::default();
        divider.catch_up(5 * TICK + 3);

        let mut restored = Divider::default();
        restored.restore(divider.snapshot(5 * TICK + 3), 5 * TICK + 3);
        assert_eq!(restored, divider);
        assert_eq!(restored.next_event(), divider.next_event());
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
