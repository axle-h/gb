use bincode::{Decode, Encode};
use crate::audio::frame_sequencer::FrameSequencer;

#[derive(Debug, Clone, Eq, PartialEq, Decode, Encode)]
pub struct LengthTimer {
    enabled: bool,
    offset: u16,
    value: u16
}

impl LengthTimer {
    pub fn square_or_noise_channel() -> Self {
        Self::new(64)
    }

    pub fn wave_channel() -> Self {
        Self::new(256)
    }

    fn new(offset: u16) -> Self {
        Self { enabled: false, value: offset, offset }
    }

    pub fn reset(&mut self, initial_length_timer: u8) {
        self.value = self.offset - initial_length_timer as u16;
    }

    pub fn trigger(&mut self, frame_sequencer: &FrameSequencer) {
        // Triggering resets the counter to max value if it has expired
        if self.value == 0 {
            self.reset(0x00);
            // Quirk: Immediately clock if enabled during trigger and this is a length counter cycle
            if self.enabled && frame_sequencer.current_events().is_length_counter() {
                self.value = self.value.saturating_sub(1);
            }
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool, frame_sequencer: &FrameSequencer, channel_active: &mut bool) {
        let prev_enabled = self.enabled;
        self.enabled = enabled;

        // Quirk: Immediately clock if newly enabled and this is a length counter cycle
        if !prev_enabled && self.enabled && frame_sequencer.current_events().is_length_counter() {
            self.clock(channel_active);
        }
    }

    pub fn clock(&mut self, channel_active: &mut bool) {
        if !self.enabled {
            return;
        }

        if self.value > 0 {
            self.value -= 1;
        }
        if self.value == 0 {
            // length overflowed, disable the channel
            *channel_active = false;
        }
    }
}