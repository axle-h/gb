use crate::audio::dac::dac_sample;
use crate::audio::frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use crate::audio::length::{LengthTimer};
use crate::audio::timer::WavetableTimer;
use crate::cycles::MachineCycles;

#[derive(Debug, Clone)]
pub struct WaveChannel {
    /// NR30: DAC enable
    /// bit 7 DAC power (0=Off, 1=On)
    dac_enabled: bool,

    /// NR31 Length timer
    /// bits 0-8 Initial length timer
    initial_length_timer: u8,
    length_timer: LengthTimer,

    /// NR32 output level
    /// 2 bits (0-3) nr32
    /// 00	Mute (No sound)
    /// 01	100% volume (use samples read from Wave RAM as-is)
    /// 10	50% volume (shift samples read from Wave RAM right once)
    /// 11	25% volume (shift samples read from Wave RAM right twice)
    volume_register: u8,

    // NR33 & NR34: frequency & control
    /// 11 bits (0-2047)
    /// low 8 bits in NR33, high 3 bits in NR34
    period_register: u16,

    /// 16 bytes of wave pattern RAM (32 4-bit samples)
    wave_ram: [u8; 16],

    /// Internal state
    active: bool, // the channel has been triggered and is active
    frequency_timer: WavetableTimer, // internal counter that overflows at current_period

    sample_buffer: u8, // current output sample (0-15)
}

// From https://gbdev.gg8.se/wiki/articles/Gameboy_sound_hardware#Power_Control
const DMG_INITIAL_RAM: [u8; 16] = [
    0x84, 0x40, 0x43, 0xAA, 0x2D, 0x78, 0x92, 0x3C, 0x60, 0x59, 0x59, 0xB0, 0x34, 0xB8, 0x2E, 0xDA,
];

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            dac_enabled: false,
            initial_length_timer: 0,
            length_timer: LengthTimer::wave_channel(),
            volume_register: 0,
            period_register: 0,
            wave_ram: DMG_INITIAL_RAM,
            active: false,
            frequency_timer: WavetableTimer::default(),
            sample_buffer: 0
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        // wave ram is not touched on reset
        *self = Self { wave_ram: self.wave_ram, ..Self::default() };
    }

    pub fn nr30(&self) -> u8 {
        // Bit 7: DAC power (0=Off, 1=On)
        // Bits 0-6: Read as 1
        if self.dac_enabled {
            0xFF
        } else {
            0x7F
        }
    }

    pub fn set_nr30(&mut self, value: u8) {
        self.dac_enabled = value & 0x80 != 0;
        if !self.dac_enabled {
            self.active = false;
        }
    }

    pub fn nr31_length_timer(&self) -> u8 {
        0xFF // write only
    }

    pub fn set_nr31_length_timer(&mut self, value: u8) {
        self.initial_length_timer = value;

        // the length timer can be reset at any time
        self.length_timer.reset(self.initial_length_timer);
    }

    pub fn nr32_output_level(&self) -> u8 {
        // Bits 0-4: Read as 1
        // Bits 5-6: Volume code
        // Bit 7: Read as 1
        0x9F | ((self.volume_register & 0b11) << 5)
    }

    pub fn set_nr32_output_level(&mut self, value: u8) {
        self.volume_register = (value >> 5) & 0b11;
    }

    pub fn nr33_period_low(&self) -> u8 {
        0xFF // nr33 is write-only
    }

    pub fn set_nr33_period_low(&mut self, value: u8) {
        self.period_register = (self.period_register & 0xFF00) | value as u16;
    }

    pub fn nr34_period_high_and_control(&self) -> u8 {
        // Bits 0-5 & 7 are always 1 when read
        0xBF | if self.length_timer.enabled() { 0b01000000 } else { 0 }
    }

    pub fn set_nr34_period_high_and_control(&mut self, value: u8, frame_sequencer: &FrameSequencer) {
        self.period_register = (self.period_register & 0x00FF) | (((value & 0b111) as u16) << 8);
        let length_enabled = value & 0b01000000 != 0;
        self.length_timer.set_enabled(length_enabled, frame_sequencer, &mut self.active);
        if value & 0b10000000 != 0 {
            self.trigger(frame_sequencer);
        }
    }

    pub fn wave_ram(&self, index: usize) -> u8 {
        // Reading from wavetable RAM while the channel is playing returns the contents of RAM at
        // the current wave position rather than the requested address.
        if self.active {
            self.current_sample_byte()
        } else {
            self.wave_ram[index]
        }
    }

    pub fn set_wave_ram(&mut self, index: usize, value: u8) {
        self.wave_ram[index] = value;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn dac_enabled(&self) -> bool {
        self.dac_enabled
    }

    pub fn output_f32(&self) -> f32 {
        if !self.dac_enabled || self.volume_register == 0 {
            return 0.0;
        }

        let sample_byte = self.sample_buffer >> (self.volume_register - 1);
        let sample = if self.frequency_timer.phase() & 0x1 == 0 {
            sample_byte >> 4
        } else {
            sample_byte & 0xF
        };

        dac_sample(sample)
    }

    pub fn trigger(&mut self, frame_sequencer: &FrameSequencer) {
        self.active = self.dac_enabled;
        self.length_timer.trigger(frame_sequencer);
        self.frequency_timer.set_frequency(self.period_register);
    }

    pub fn update(&mut self, delta: MachineCycles, events: FrameSequencerEvent) {
        if self.active && !self.dac_enabled() {
            self.active = false;
        }

        if !self.active {
            self.sample_buffer = 0;

            // disabled channels still clock the length counter
            if events.is_length_counter() {
                self.clock_length_timer();
            }
            return;
        }

        if events.is_length_counter() {
            self.clock_length_timer();
        }

        if self.active && self.frequency_timer.update(delta) {
            // overflow, emit a sample
            self.sample_buffer = self.current_sample_byte();
        }
    }

    fn current_sample_byte(&self) -> u8 {
        self.wave_ram[(self.frequency_timer.phase() >> 1) as usize]
    }

    fn clock_length_timer(&mut self) {
        let prev_active = self.active;
        self.length_timer.clock(&mut self.active);
        if prev_active && !self.active {
            // Explicitly clear the sample buffer when the length counter disables the channel.
            // Necessary because the wavetable channel continues to output the current sample buffer
            // when disabled, as long as the DAC is still enabled
            self.sample_buffer = 0;
        }
    }
}

