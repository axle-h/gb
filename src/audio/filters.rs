use bincode::{Decode, Encode};
use crate::audio::sample::AudioSample;

#[derive(Debug, Clone, PartialEq, Default, Decode, Encode)]
pub struct CapacitanceFilter {
    capacitor_left: f32,
    capacitor_right: f32,
}

impl CapacitanceFilter {
    pub fn process(&mut self, input: AudioSample) -> AudioSample {
        AudioSample {
            left: Self::process_channel(input.left, &mut self.capacitor_left),
            right: Self::process_channel(input.right, &mut self.capacitor_right),
        }
    }

    fn process_channel(input: f32, capacitor: &mut f32) -> f32 {
        let output = input - *capacitor;
        *capacitor = input - output * 0.999832011; // Simple feedback to simulate capacitor behavior
        output
    }
}