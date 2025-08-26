use crate::audio::sample::AudioSample;

#[derive(Debug, Clone, Default)]
pub struct HighPassFilter {
    capacitor_left: f32,
    capacitor_right: f32,
}

impl HighPassFilter {
    pub fn process(&mut self, input: AudioSample) -> AudioSample {
        let (left, capacitor_left) = self.process_channel(input.left, self.capacitor_left);
        let (right, capacitor_right) = self.process_channel(input.right, self.capacitor_right);
        self.capacitor_left = capacitor_left;
        self.capacitor_right = capacitor_right;
        AudioSample { left, right }
    }

    fn process_channel(&mut self, input: f32, capacitor: f32) -> (f32, f32) {
        let output = input - capacitor;
        let capacitor = input - output * 0.999832011; // Simple feedback to simulate capacitor behavior
        (output, capacitor)
    }
}