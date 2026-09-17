//! `engine/play_time.asm`: `TrackPlayTime`, which VBlank runs every frame.

use serde::{Deserialize, Serialize};

/// `wPlayTimeHours` to `wPlayTimeFrames`, and `BIT_GAME_TIMER_COUNTING`, which `SpecialEnterMap`
/// sets on the way into the world.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayTime {
    pub hours: u8,
    /// `wPlayTimeMaxed`: the clock stops for good at 255 hours.
    pub maxed: bool,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub counting: bool,
}

impl PlayTime {
    /// `TrackPlayTime`: sixty frames a second, sixty seconds a minute and sixty minutes an hour,
    /// until the hours reach 255.
    pub fn track(&mut self) {
        if !self.counting || self.maxed {
            return;
        }
        self.frames += 1;
        if self.frames < 60 {
            return;
        }
        self.frames = 0;
        self.seconds += 1;
        if self.seconds < 60 {
            return;
        }
        self.seconds = 0;
        self.minutes += 1;
        if self.minutes < 60 {
            return;
        }
        self.minutes = 0;
        self.hours += 1;
        self.maxed = self.hours == 0xFF;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_hour_is_216000_frames() {
        let mut time = PlayTime { counting: true, ..PlayTime::default() };
        for _ in 0..216_000 {
            time.track();
        }
        assert_eq!(time, PlayTime { hours: 1, counting: true, ..PlayTime::default() });
    }

    #[test]
    fn the_clock_stops_at_255_hours_and_only_counts_once_started() {
        let mut stopped = PlayTime::default();
        stopped.track();
        assert_eq!(stopped.frames, 0);

        let mut time = PlayTime { hours: 254, minutes: 59, seconds: 59, frames: 59, counting: true, ..PlayTime::default() };
        time.track();
        assert_eq!((time.hours, time.minutes, time.maxed), (255, 0, true));
        time.track();
        assert_eq!(time.frames, 0, "nothing moves once maxed");
    }
}
