use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};
use std::time::Duration;
use bincode::{Decode, Encode};

/// A count of machine (M-) cycles. **`u64`, not `usize`** — C1 made the emulator's clock absolute,
/// and an absolute m-cycle count has to be the same width on every host: a 32-bit `usize` wraps
/// after 34 minutes of emulated time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Decode, Encode)]
pub struct MachineCycles(u64);

impl MachineCycles {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1);
    pub const CPU_FREQ: u64 = 4194304; // 4.194304 MHz t-cycles/s
    pub const PER_SERIAL_BYTE_TRANSFER: MachineCycles = MachineCycles::from_hz(8192 / 8); // 8192 Hz serial transfer rate
    pub const PER_DIVIDER_TICK: MachineCycles = MachineCycles::from_hz(16384);

    pub const fn from_m(cycles: u64) -> Self {
        Self(cycles)
    }

    pub const fn m_cycles(self) -> u64 {
        self.0
    }

    pub const fn t_cycles(self) -> u64 {
        self.0 * 4 // 1 tick = 4 machine cycles
    }

    pub const fn from_duration(duration: Duration) -> Self {
        let nanos = duration.as_nanos(); // u128 — avoids overflow for durations up to ~years
        let t_cycles = (nanos * Self::CPU_FREQ as u128) / 1_000_000_000;
        let m_cycles = (t_cycles / 4) as u64;
        Self(m_cycles)
    }

    pub const fn from_hz(hz: u64) -> Self {
        MachineCycles::from_t(Self::CPU_FREQ / hz)
    }

    pub const fn to_hz(self) -> u64 {
        Self::CPU_FREQ / self.t_cycles()
    }

    pub const fn from_t(ticks: u64) -> Self {
        Self(ticks / 4) // 4 tick = 1 machine cycle
    }

    /// ⚠️ **The multiply is done in `u128`, and that is not defensive — `u64` overflows here after
    /// about 73 minutes of emulated time.** `self.0 * 4_000_000_000` passes `u64::MAX` once `self.0`
    /// reaches ~4.6e9 m-cycles, and in release builds that wraps silently rather than panicking: the
    /// figure just becomes nonsense partway through a long run. It surfaced in `soak`, whose progress
    /// line stopped appearing after 3600 s and looked like a bug in the test's own bookkeeping.
    ///
    /// Anything that reports emulated time over a long run was affected — `meta.json`'s `emulated_ms`
    /// and the status heartbeat both go through here, so a deployed run's clock silently wrapped
    /// every 73 minutes. `from_duration` already used `u128` for the same reason in the other
    /// direction.
    pub const fn to_duration(self) -> Duration {
        let nanos = (self.0 as u128 * 4_000_000_000) / Self::CPU_FREQ as u128;
        Duration::from_nanos(nanos as u64)
    }

    /// Subtraction that clamps at zero. The plain [`Sub`] impl deliberately does **not** do this —
    /// see its comment — so the two callers that genuinely want a floor ask for it by name.
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}


impl From<u64> for MachineCycles {
    fn from(cycles: u64) -> Self {
        Self(cycles)
    }
}

impl Add for MachineCycles {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl AddAssign for MachineCycles {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

/// ⚠️ **Not saturating.** It used to be, which silently turned every cycle-ordering bug into a
/// timing skew instead of a panic (finding F11). A `debug_assert` catches the ordering bug in the
/// test suite while release builds keep the wrapping-free single `sub`; callers that legitimately
/// want a floor use [`MachineCycles::saturating_sub`].
impl Sub for MachineCycles {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        debug_assert!(self.0 >= other.0, "MachineCycles underflow: {} - {}", self.0, other.0);
        Self(self.0.wrapping_sub(other.0))
    }
}

impl SubAssign for MachineCycles {
    fn sub_assign(&mut self, other: Self) {
        *self = *self - other;
    }
}

impl Mul<u64> for MachineCycles {
    type Output = Self;

    fn mul(self, rhs: u64) -> Self {
        Self(self.0 * rhs)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion() {
        let cycles = MachineCycles::from_m(100);
        assert_eq!(cycles.t_cycles(), 400);
        assert_eq!(MachineCycles::from_t(400), cycles);

        let duration = Duration::from_millis(1);
        let converted_cycles = MachineCycles::from_duration(duration);
        assert_eq!(converted_cycles, MachineCycles::from_m(1048));
    }

    #[test]
    fn from_duration() {
        let one_second = MachineCycles::from_duration(Duration::from_secs(1));
        assert_eq!(one_second, MachineCycles::from_m(MachineCycles::CPU_FREQ / 4));
    }

    /// ⚠️ `to_duration` multiplied by 4e9 in `u64`, which wraps past ~73 minutes — silently, in
    /// release. A five-hour run's clock read as nonsense and nothing complained.
    #[test]
    fn to_duration_survives_a_long_run() {
        for hours in [1u64, 2, 5, 24] {
            let expected = Duration::from_secs(hours * 3600);
            let round_tripped = MachineCycles::from_duration(expected).to_duration();
            let drift = round_tripped.abs_diff(expected);
            assert!(drift < Duration::from_millis(1),
                    "{hours}h round-tripped to {round_tripped:?}, off by {drift:?}");
        }
    }

    #[test]
    fn from_hz() {
        let cycles = MachineCycles::from_hz(16384);
        assert_eq!(cycles, MachineCycles(64));
        let cycles = MachineCycles::from_hz(4096);
        assert_eq!(cycles, MachineCycles(256));
    }

    #[test]
    fn to_duration() {
        let cycles = MachineCycles::from_m(100);
        let back_to_cycles = MachineCycles::from_duration(cycles.to_duration());
        assert_eq!(back_to_cycles, MachineCycles::from_m(99));
    }
}