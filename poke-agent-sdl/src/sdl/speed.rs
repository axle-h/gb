//! How fast the window plays both games: a whole number of frames a host frame, or as many as the
//! host frame has time for.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Speed {
    #[default]
    One,
    Two,
    Four,
    Unthrottled,
}

impl Speed {
    /// In the order the button cycles them, and keys `1` to `4` choose them.
    pub const ALL: [Speed; 4] = [Speed::One, Speed::Two, Speed::Four, Speed::Unthrottled];

    pub fn next(self) -> Self {
        Self::ALL[(Self::ALL.iter().position(|&s| s == self).unwrap() + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Speed::One => "SPEED 1x",
            Speed::Two => "SPEED 2x",
            Speed::Four => "SPEED 4x",
            Speed::Unthrottled => "SPEED MAX",
        }
    }

    /// Frames each host frame plays, or `None` for as many as fit in it.
    pub fn frames(self) -> Option<u32> {
        match self {
            Speed::One => Some(1),
            Speed::Two => Some(2),
            Speed::Four => Some(4),
            Speed::Unthrottled => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_cycles_through_every_speed_back_to_the_first() {
        let cycle: Vec<Speed> = std::iter::successors(Some(Speed::One), |s| Some(s.next())).take(5).collect();
        assert_eq!(cycle, [Speed::One, Speed::Two, Speed::Four, Speed::Unthrottled, Speed::One]);
        assert_eq!(Speed::ALL.map(Speed::frames), [Some(1), Some(2), Some(4), None]);
    }
}
