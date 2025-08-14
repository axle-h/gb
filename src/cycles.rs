use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd)]
pub struct MachineCycles(pub usize);

impl MachineCycles {
    pub const ZERO: Self = Self(0);
    pub const CPU_FREQ: usize = 4194304; // 4.194304 MHz t-cycles/s
    pub const PER_SERIAL_BYTE_TRANSFER: MachineCycles = MachineCycles::of_hz(8192 / 8); // 8192 Hz serial transfer rate
    pub const PER_DIVIDER_TICK: MachineCycles = MachineCycles::of_hz(16384);

    pub const fn of_machine(cycles: usize) -> Self {
        Self(cycles)
    }

    pub fn to_ticks(self) -> usize {
        self.0 * 4 // 1 tick = 4 machine cycles
    }

    pub fn of_real_time(duration: Duration) -> Self {
        let nanos = duration.as_nanos() as usize;
        let t_cycles = (nanos * Self::CPU_FREQ) / 1_000_000_000;
        let m_cycles = t_cycles / 4; // 1 machine cycle = 4 clock cycles
        Self(m_cycles)
    }

    pub const fn of_hz(hz: usize) -> Self {
        MachineCycles::of_ticks(Self::CPU_FREQ / hz)
    }

    pub const fn of_ticks(ticks: usize) -> Self {
        Self(ticks / 4) // 4 tick = 1 machine cycle
    }
}

impl From<usize> for MachineCycles {
    fn from(cycles: usize) -> Self {
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

impl Sub for MachineCycles {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

impl SubAssign for MachineCycles {
    fn sub_assign(&mut self, other: Self) {
        self.0 = self.0.saturating_sub(other.0);
    }
}

impl Mul<usize> for MachineCycles {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self {
        Self(self.0 * rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_cycles_conversion() {
        let cycles = MachineCycles::of_machine(100);
        assert_eq!(cycles.to_ticks(), 400);
        assert_eq!(MachineCycles::of_ticks(400), cycles);

        let duration = Duration::from_millis(1);
        let converted_cycles = MachineCycles::of_real_time(duration);
        assert_eq!(converted_cycles, MachineCycles::of_machine(1048));
    }

    #[test]
    fn of_real_time() {
        let one_second = MachineCycles::of_real_time(Duration::from_secs(1));
        assert_eq!(one_second, MachineCycles::of_machine(MachineCycles::CPU_FREQ / 4));
    }

    #[test]
    fn of_hz() {
        let cycles = MachineCycles::of_hz(16384);
        assert_eq!(cycles, MachineCycles(64));
        let cycles = MachineCycles::of_hz(4096);
        assert_eq!(cycles, MachineCycles(256));
    }
}