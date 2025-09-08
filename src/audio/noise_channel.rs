use crate::audio::dac::dac_sample;
use crate::audio::frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use crate::audio::length::{LengthTimer};
use crate::audio::volume::{EnvelopeFunction, VolumeAndEnvelopeRegister};
use crate::cycles::MachineCycles;

#[derive(Debug, Clone)]
pub struct NoiseChannel {
    /// NR41 length timer
    /// bits 0-5 Initial length timer
    initial_length_timer: u8,
    length_timer: LengthTimer,

    /// NR42 volume and envelope
    envelope_function: EnvelopeFunction,

    /// NR43 frequency and randomness
    clock_shift: u8, // bits 4-7 Clock shift
    lfsr_width: bool, // bit 3 LFSR width (0=15 bits, 1=7 bits)
    clock_divider: u8, // bits 0-2 Dividing ratio of frequencies (0-7)

    /// internal state
    active: bool,
    lfsr: u16, // 15-bit LFSR
    output: u8,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            initial_length_timer: 0,
            length_timer: LengthTimer::square_or_noise_channel(),
            envelope_function: EnvelopeFunction::default(),
            clock_shift: 0,
            lfsr_width: false,
            clock_divider: 0,
            active: false,
            lfsr: 0,
            output: 0
        }
    }
}

impl NoiseChannel {
    pub fn nr41_length_timer(&self) -> u8 {
        0xFF // write only
    }

    pub fn set_nr41_length_timer(&mut self, value: u8) {
        self.initial_length_timer = value & 0x3F; // Bits 0-5

        // the length timer can be reset at any time
        self.length_timer.reset(self.initial_length_timer);
    }

    pub fn nr42_volume_and_envelope(&self) -> &VolumeAndEnvelopeRegister {
        &self.envelope_function.register()
    }

    pub fn nr42_volume_and_envelope_mut(&mut self) -> &mut VolumeAndEnvelopeRegister {
        self.envelope_function.register_mut()
    }

    pub fn nr43_frequency_and_randomness(&self) -> u8 {
        (self.clock_shift << 4) | ((self.lfsr_width as u8) << 3) | (self.clock_divider & 0x07)
    }

    pub fn set_nr43_frequency_and_randomness(&mut self, value: u8) {
        self.clock_shift = value >> 4; // bits 4-7
        self.lfsr_width = (value & 0x08) != 0; // bit 3
        self.clock_divider = value & 0x07; // bits 0-2
    }

    pub fn nr44_control(&self) -> u8 {
        // only bit 6 is readable, all other bits read as 1
        if self.length_timer.enabled() { 0xFF } else { 0xBF }
    }

    pub fn set_nr44_control(&mut self, value: u8, frame_sequencer: &FrameSequencer) {
        let length_enabled = (value & 0x40) != 0; // bit 6
        self.length_timer.set_enabled(length_enabled, frame_sequencer, &mut self.active);
        if value & 0x80 != 0 {
            self.trigger(frame_sequencer); // trigger on bit 7
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn dac_enabled(&self) -> bool {
        self.envelope_function.dac_enabled()
    }

    pub fn output(&self) -> u8 {
        self.output
    }

    pub fn output_f32(&self) -> f32 {
        if self.envelope_function.dac_enabled() {
            dac_sample(self.output)
        } else {
            0.0
        }
    }

    pub fn trigger(&mut self, frame_sequencer: &FrameSequencer) {
        if !self.envelope_function.dac_enabled() {
            // the length timer is still triggered even when the dac is disabled.
            self.length_timer.trigger(frame_sequencer);
            return;
        }
        self.active = true;
        self.length_timer.trigger(frame_sequencer);
        self.envelope_function.reset();
        self.lfsr = 0;
    }

    pub fn update(&mut self, delta: MachineCycles, events: FrameSequencerEvent) {
        if self.active && !self.envelope_function.dac_enabled() {
            self.active = false;
        }

        if !self.active {
            self.output = 0;

            // disabled channels still clock the length counter
            if events.is_length_counter() {
                self.length_timer.clock(&mut self.active);
            }
            return
        }

        if events.is_length_counter() {
            self.length_timer.clock(&mut self.active);
        }

        // TODO noise output logic
        self.output = 0;
    }
}

