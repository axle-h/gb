use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AudioSample {
    pub left: f32,
    pub right: f32,
}

impl AudioSample {
    pub const ZERO: Self = Self { left: 0.0, right: 0.0 };
    pub fn new(left: f32, right: f32) -> Self {
        Self { left, right }
    }
}

impl Add for AudioSample {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            left: self.left + other.left,
            right: self.right + other.right,
        }
    }
}

impl AddAssign for AudioSample {
    fn add_assign(&mut self, other: Self) {
        self.left += other.left;
        self.right += other.right;
    }
}

impl Sub for AudioSample {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            left: self.left - other.left,
            right: self.right - other.right,
        }
    }
}

impl SubAssign for AudioSample {
    fn sub_assign(&mut self, other: Self) {
        self.left -= other.left;
        self.right -= other.right;
    }
}

impl Div<f32> for AudioSample {
    type Output = Self;

    fn div(self, rhs: f32) -> Self {
        Self {
            left: self.left / rhs,
            right: self.right / rhs,
        }
    }
}

impl Mul<f32> for AudioSample {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self {
        Self {
            left: self.left * rhs,
            right: self.right * rhs,
        }
    }
}

impl Mul<AudioSample> for AudioSample {
    type Output = AudioSample;

    fn mul(self, rhs: AudioSample) -> AudioSample {
        AudioSample {
            left: self.left * rhs.left,
            right: self.right * rhs.right,
        }
    }
}

impl Sum for AudioSample {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        let mut total = AudioSample::default();
        for sample in iter {
            total += sample;
        }
        total
    }
}