//! `DelayFrames` and `CheckForUserInterruption`, the two waits every movie screen is paced by.

use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::mode::Ctx;

/// What `CheckForUserInterruption` stops on besides a new A or START: Up, Select and B held
/// together and nothing else, which the title screen reads as the way to clear the save.
pub const CLEAR_SAVE_BUTTONS: Joypad = Joypad::UP.union(Joypad::SELECT).union(Joypad::B);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wait {
    frames: u16,
    /// `CheckForUserInterruption`: the pad is read after every frame.
    checks: bool,
    /// The pad has been read since the wait began.
    polled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// Still waiting; nothing more runs this frame.
    Waiting,
    /// Over, and the code after it runs in this frame.
    Done,
    /// A press cut it short, in this frame.
    Interrupted,
}

impl Wait {
    /// `DelayFrames`: the code after it runs in the frame the count ends.
    pub fn frames(frames: u16) -> Self {
        Self { frames, checks: false, polled: false }
    }

    /// `CheckForUserInterruption`: a `DelayFrame` and a read of the pad, `frames` times.
    pub fn check(frames: u16) -> Self {
        Self { frames, checks: true, polled: false }
    }

    pub fn is_running(&self) -> bool {
        self.frames != 0
    }

    /// Reading the pad, where a decision could be answered.
    pub fn is_polling(&self) -> bool {
        self.checks && self.polled && self.frames != 0
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Tick {
        if self.frames == 0 {
            return Tick::Done;
        }
        self.frames -= 1;
        if self.checks {
            self.polled = true;
            let pressed = ctx.pad.low_sensitivity(ctx.frame_counter);
            if ctx.pad.held == CLEAR_SAVE_BUTTONS || pressed.intersects(Joypad::START | Joypad::A) {
                self.frames = 0;
                return Tick::Interrupted;
            }
        }
        if self.frames == 0 { Tick::Done } else { Tick::Waiting }
    }
}
