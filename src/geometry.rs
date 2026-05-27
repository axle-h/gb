use std::fmt::{Display, Formatter};
use std::ops::{Add, Div};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode, Hash)]
pub struct Point8 {
    pub x: u8,
    pub y: u8,
}

impl Add<Point8> for Point8 {
    type Output = Point8;

    fn add(self, other: Point8) -> Point8 {
        Point8 {
            x: self.x.wrapping_add(other.x),
            y: self.y.wrapping_add(other.y),
        }
    }
}

impl Div<u8> for Point8 {
    type Output = Point8;

    fn div(self, divisor: u8) -> Point8 {
        Point8 {
            x: self.x / divisor,
            y: self.y / divisor,
        }
    }
}

impl PartialOrd for Point8 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Point8 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.y.cmp(&other.y).then_with(|| self.x.cmp(&other.x))
    }
}


impl Display for Point8 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}