use bincode::{Decode, Encode};
use crate::cycles::MachineCycles;
use crate::activation::Activation;
use crate::schedule::DISABLED;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Timer {
    enabled: bool,
    mode: TimerMode,
    value: u8,
    modulo: u8,
    /// When TIMA next increments. **While the timer runs this is an absolute m-cycle stamp on
    /// [`crate::mmu::MMU::now`]; while it is stopped it is the cycles the interrupted period still
    /// owes** — the same quantity measured from a standstill rather than from the clock.
    /// [`Timer::set_control`] rebases between the two, which is what stops a disabled timer from
    /// banking the cycles it slept through. (Hardware ties TIMA's phase to DIV and would not
    /// freeze it at all; freezing is what `gb` has always done, and C1 is a refactor.)
    ///
    /// **Signed**, because a TAC write that shortens the period can leave the deadline in the
    /// past: pre-C1 that showed up as a remainder larger than the new period, and it has to keep
    /// producing the same burst of catch-up ticks.
    next_tick: i64,
    interrupt_pending: bool,
}

/// The serialised shape of a [`Timer`]. Field-for-field what the `timer` save-state section has
/// always held — including `cycles`, the *elapsed* part of the current period, which C1 replaced
/// internally with the deadline it implies. Keeping the old shape is why the absolute clock landed
/// without regenerating any of the 91 committed fixtures.
#[derive(Debug, Clone, Decode, Encode)]
pub struct TimerSnapshot {
    enabled: bool,
    mode: TimerMode,
    value: u8,
    modulo: u8,
    cycles: MachineCycles,
    interrupt_pending: bool,
}

/// A whole period still owed, measured from a standstill — the deadline form of the `cycles: 0`
/// this used to derive from `#[derive(Default)]`. Zero would mean the *opposite*: a period that
/// has fully elapsed.
impl Default for Timer {
    fn default() -> Self {
        let mode = TimerMode::default();
        Self {
            enabled: false,
            mode,
            value: 0,
            modulo: 0,
            next_tick: mode.cycles_per_tick().m_cycles() as i64,
            interrupt_pending: false,
        }
    }
}

impl Timer {
    /// Where the deadline is measured from: the clock while running, a standstill while stopped.
    fn origin(enabled: bool, now: u64) -> i64 {
        if enabled { now as i64 } else { 0 }
    }

    pub fn snapshot(&self, now: u64) -> TimerSnapshot {
        let owed = self.next_tick - Self::origin(self.enabled, now);
        TimerSnapshot {
            enabled: self.enabled,
            mode: self.mode,
            value: self.value,
            modulo: self.modulo,
            cycles: MachineCycles::from_m((self.period() - owed).max(0) as u64),
            interrupt_pending: self.interrupt_pending,
        }
    }

    pub fn restore(&mut self, snapshot: TimerSnapshot, now: u64) {
        self.enabled = snapshot.enabled;
        self.mode = snapshot.mode;
        self.value = snapshot.value;
        self.modulo = snapshot.modulo;
        self.interrupt_pending = snapshot.interrupt_pending;
        self.next_tick =
            Self::origin(self.enabled, now) + self.period() - snapshot.cycles.m_cycles() as i64;
    }

    fn period(&self) -> i64 {
        self.mode.cycles_per_tick().m_cycles() as i64
    }

    pub fn enable(&mut self, now: u64) {
        self.set_control(self.control() | 0b0100, now);
    }

    pub fn disable(&mut self, now: u64) {
        self.set_control(self.control() & !0b0100, now);
    }

    pub fn control(&self) -> u8 {
        self.mode as u8 | if self.enabled { 0b0100 } else { 0 }
    }

    /// ⚠️ **Catch the timer up to `now` first.** The deadline is rebased against the state as of
    /// `now`, so a stale one would be carried into the new period.
    pub fn set_control(&mut self, value: u8, now: u64) {
        let enabled = value & 0b0100 != 0;
        let mode = TimerMode::from_repr(value & 0b11).unwrap_or_default();

        // Preserve how far into the period TIMA has already got, then re-express what is left
        // against the new period and the new origin. Pre-C1 this fell out of keeping the `cycles`
        // accumulator across the write, including the case where the new period is *shorter* than
        // what has already elapsed and the next catch-up owes several ticks at once.
        let elapsed = self.period() - (self.next_tick - Self::origin(self.enabled, now));
        self.enabled = enabled;
        self.mode = mode;
        self.next_tick = Self::origin(enabled, now) + self.period() - elapsed;
    }

    pub fn value(&self) -> u8 {
        self.value
    }

    pub fn set_value(&mut self, value: u8) {
        self.value = value;
    }

    pub fn modulo(&self) -> u8 {
        self.modulo
    }

    pub fn set_modulo(&mut self, value: u8) {
        self.modulo = value;
    }

    /// Advance TIMA to absolute m-cycle `now`.
    #[inline]
    pub fn catch_up(&mut self, now: u64) {
        if !self.enabled {
            return;
        }
        let now = now as i64;
        if self.next_tick > now {
            return;
        }

        let period = self.period();
        while self.next_tick <= now {
            self.next_tick += period;
            if self.value == 0xFF {
                self.value = self.modulo;
                self.interrupt_pending = true;
            } else {
                self.value += 1;
            }
        }
    }

    /// Absolute m-cycle at which TIMA next increments, or [`DISABLED`] while the timer is off.
    pub fn next_event(&self) -> u64 {
        if !self.enabled {
            return DISABLED;
        }
        self.next_tick.max(0) as u64
    }
}

impl Activation for Timer {
    fn is_activation_pending(&self) -> bool {
        self.interrupt_pending
    }

    fn clear_activation(&mut self) {
        self.interrupt_pending = false;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, strum_macros::FromRepr, Decode, Encode)]
#[repr(u8)]
enum TimerMode {
    #[default]
    M256 = 0,
    M4 = 1,
    M16 = 2,
    M64 = 3,
}

impl TimerMode {
    pub fn cycles_per_tick(self) -> MachineCycles {
        match self {
            TimerMode::M256 => MachineCycles::from_m(256),
            TimerMode::M4 => MachineCycles::from_m(4),
            TimerMode::M16 => MachineCycles::from_m(16),
            TimerMode::M64 => MachineCycles::from_m(64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_timer() -> Timer {
        let mut timer = Timer::default();
        timer.set_control(0b0101, 0); // enabled, M4
        timer
    }

    /// C1: `next_event` has to name the exact cycle the counter moves, or the HALT fast-path C2
    /// builds on it would skip straight past a TIMA increment.
    #[test]
    fn next_event_names_the_cycle_tima_increments() {
        let mut timer = enabled_timer();
        assert_eq!(timer.next_event(), 4);

        timer.catch_up(3);
        assert_eq!(timer.value(), 0, "one cycle short");
        assert_eq!(timer.next_event(), 4);

        timer.catch_up(4);
        assert_eq!(timer.value(), 1);
        assert_eq!(timer.next_event(), 8);
    }

    #[test]
    fn a_disabled_timer_never_fires() {
        let timer = Timer::default();
        assert_eq!(timer.next_event(), DISABLED);
    }

    /// A stopped timer must not bank the cycles it slept through — pre-C1 the `cycles` accumulator
    /// simply froze, and the deadline's two origins reproduce that.
    #[test]
    fn a_disabled_timer_does_not_bank_the_cycles_it_slept_through() {
        let mut timer = Timer::default(); // disabled
        timer.catch_up(1000);
        timer.enable(1000);
        timer.catch_up(1002);
        assert_eq!(timer.value(), 0, "only 2 of the 256 cycles a tick needs have elapsed");

        // ...and stopping it mid-period keeps what has elapsed rather than restarting it.
        timer.disable(1002);
        timer.catch_up(9999);
        timer.enable(9999);
        timer.catch_up(9999 + 253);
        assert_eq!(timer.value(), 0, "2 + 253 is still one short of 256");
        timer.catch_up(9999 + 254);
        assert_eq!(timer.value(), 1);
    }

    /// Shortening the period mid-run leaves the deadline behind `now`, and the next catch-up owes
    /// several ticks at once. Pre-C1 this was a remainder larger than the new period; it is the
    /// reason the deadline is signed.
    #[test]
    fn shortening_the_period_pays_out_the_ticks_it_skipped() {
        let mut timer = Timer::default();
        timer.set_control(0b0100, 0); // enabled, M256
        timer.catch_up(200); // 200 of 256 elapsed, no tick yet
        assert_eq!(timer.value(), 0);

        timer.set_control(0b0101, 200); // switch to M4, keeping the 200 elapsed cycles
        timer.catch_up(203);
        // 203 cycles' worth of M4 periods: floor(203 / 4) = 50.
        assert_eq!(timer.value(), 50);
    }

    /// A snapshot is the pre-C1 field list exactly, and the deadline comes back from the restored
    /// clock — this is what lets 91 committed fixtures survive C1 untouched.
    #[test]
    fn a_snapshot_round_trips_against_a_restored_clock() {
        let mut timer = enabled_timer();
        timer.catch_up(4002);
        assert_eq!(timer.value(), (1000 % 256) as u8, "1000 M4 ticks, wrapping through TMA=0");

        let mut restored = Timer::default();
        restored.restore(timer.snapshot(4002), 4002);
        assert_eq!(restored, timer);
        assert_eq!(restored.next_event(), timer.next_event());
    }

    /// ...and the same for a stopped timer, whose deadline is measured from the other origin.
    #[test]
    fn a_stopped_timer_round_trips_too() {
        let mut timer = enabled_timer();
        timer.catch_up(4002);
        timer.disable(4002);

        let mut restored = Timer::default();
        restored.restore(timer.snapshot(4002), 4002);
        assert_eq!(restored, timer);
    }
}
