use serde::{Deserialize, Serialize};

/// `HandleDownArrowBlinkTiming` counts loop iterations, not frames (1536 to the first toggle, then
/// 1530 hidden and 1535 shown), and how many a frame holds varies with its VBlank, so each caller
/// passes its loop's average, in hundredths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrowBlink {
    remaining: u32,
    shown: bool,
}

impl Default for ArrowBlink {
    fn default() -> Self {
        Self { remaining: 1536 * 100, shown: true }
    }
}

impl ArrowBlink {
    /// One frame's iterations; `Some(shown)` when the arrow toggles.
    pub fn tick(&mut self, per_frame: u32) -> Option<bool> {
        if self.remaining > per_frame {
            self.remaining -= per_frame;
            return None;
        }
        self.shown = !self.shown;
        self.remaining += if self.shown { 1535 * 100 } else { 1530 * 100 };
        self.remaining -= per_frame;
        Some(self.shown)
    }
}
