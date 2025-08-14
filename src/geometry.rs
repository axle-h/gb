use std::ops::{Add, Div};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

