use crate::audio::dac::dac_sample;
use crate::audio::frame_sequencer::{FrameSequencerEvent};
use crate::audio::length::{LengthTimer};
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
    length_enabled: bool,
    current_period: usize, // copy of the period register for this duty cycle
    current_volume: u8, // copy of the initial volume for this duty cycle
    wave_index: usize, // current index into the wave pattern (0-31)

    output: u8, // current output sample (0-15)
}

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            dac_enabled: false,
            initial_length_timer: 0,
            length_timer: LengthTimer::wave_channel(),
            volume_register: 0,
            period_register: 0,
            wave_ram: [0; 16],

            active: false,
            length_enabled: false,
            current_period: 0,
            current_volume: 0,
            // When CH3 is started, the first sample read is the one at index 1,
            // i.e. the lower nibble of the first byte, NOT the upper nibble.
            wave_index: 1,
            output: 0
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        self.active = false;
        self.dac_enabled = false;
        self.volume_register = 0;
        self.period_register = 0;
        // wave ram is not touched on reset

        self.initial_length_timer = 0;
        self.length_timer.reset(0);
        self.length_enabled = false;
        self.current_period = 0;
        self.current_volume = 0;
        self.wave_index = 1;
        self.output = 0;
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
        0xBF | if self.length_enabled { 0b01000000 } else { 0 }
    }

    pub fn set_nr34_period_high_and_control(&mut self, value: u8) {
        self.period_register = (self.period_register & 0x00FF) | (((value & 0b111) as u16) << 8);
        self.length_enabled = value & 0b01000000 != 0;
        if value & 0b10000000 != 0 {
            self.trigger();
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn dac_enabled(&self) -> bool {
        self.dac_enabled
    }

    pub fn output_f32(&self) -> f32 {
        if self.dac_enabled {
            dac_sample(self.output)
        } else {
            0.0
        }
    }

    pub fn wave_ram(&self, index: usize) -> u8 {
        self.wave_ram[index]
    }

    pub fn set_wave_ram(&mut self, index: usize, value: u8) {
        self.wave_ram[index] = value;
    }

    pub fn trigger(&mut self) {
        if !self.dac_enabled {
            return;
        }
        self.active = true;
        self.length_timer.restart_from_max_if_expired();
        self.current_period = self.period_register as usize;
        self.current_volume = self.volume_register;
        self.wave_index = 0;
    }

    pub fn update(&mut self, delta: MachineCycles, events: FrameSequencerEvent) {
        if !self.active {
            self.output = 0;

            // disabled channels still clock the length counter
            if events.contains(FrameSequencerEvent::LengthCounter) {
                self.update_length_counter();
            }
            return;
        }

        self.update_wave_duty(delta);

        if events.contains(FrameSequencerEvent::LengthCounter) {
            self.update_length_counter();
        }

        // TODO wave output logic
        self.output = 0;
    }

    fn update_length_counter(&mut self) {
        if self.length_enabled && self.length_timer.step() {
            // length overflowed, disable the channel
            self.active = false;
        }
    }

    fn update_wave_duty(&mut self, delta: MachineCycles) {
        // Advance the wave index based on the current period and elapsed cycles
        let cycles_per_step = (self.current_period + 1) * 2; // Each step takes (period + 1) * 2 machine cycles
        // TODO: Implement wave duty update logic
        // let steps = (delta.0 as usize) / cycles_per_step;
        // self.wave_index = (self.wave_index + steps) % 32; // There are 32 samples in the wave pattern
    }
}

