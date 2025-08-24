use std::ops::{Add, AddAssign, Mul, Sub, SubAssign};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd)]
pub struct MachineCycles(usize);

impl MachineCycles {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1);
    pub const CPU_FREQ: usize = 4194304; // 4.194304 MHz t-cycles/s
    pub const PER_SERIAL_BYTE_TRANSFER: MachineCycles = MachineCycles::from_hz(8192 / 8); // 8192 Hz serial transfer rate
    pub const PER_DIVIDER_TICK: MachineCycles = MachineCycles::from_hz(16384);

    pub const fn from_m(cycles: usize) -> Self {
        Self(cycles)
    }
    
    pub fn m_cycles(self) -> usize {
        self.0
    }

    pub fn t_cycles(self) -> usize {
        self.0 * 4 // 1 tick = 4 machine cycles
    }

    pub fn from_duration(duration: Duration) -> Self {
        let nanos = duration.as_nanos() as usize;
        let t_cycles = (nanos * Self::CPU_FREQ) / 1_000_000_000;
        let m_cycles = t_cycles / 4; // 1 machine cycle = 4 clock cycles
        Self(m_cycles)
    }

    pub const fn from_hz(hz: usize) -> Self {
        MachineCycles::from_t(Self::CPU_FREQ / hz)
    }
    
    pub fn to_hz(self) -> usize {
        Self::CPU_FREQ / self.t_cycles()
    }

    pub const fn from_t(ticks: usize) -> Self {
        Self(ticks / 4) // 4 tick = 1 machine cycle
    }

    pub fn to_duration(self) -> Duration {
        Duration::from_nanos((self.0 as u64 * 4_000_000_000) / Self::CPU_FREQ as u64)
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