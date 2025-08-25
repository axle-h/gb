use crate::audio::frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use crate::audio::length::LengthTimer;
use crate::cycles::MachineCycles;
use crate::divider::DividerClocks;

#[derive(Debug, Clone)]
pub struct WaveChannel {
    /// the channel has been triggered and is active
    active: bool,
    /// bit 7 nr30
    dac_enabled: bool,
    /// nr31
    initial_length_timer: u8,
    /// 2 bits (0-3) nr32
    /// 00	Mute (No sound)
    /// 01	100% volume (use samples read from Wave RAM as-is)
    /// 10	50% volume (shift samples read from Wave RAM right once)
    /// 11	25% volume (shift samples read from Wave RAM right twice)
    volume_register: u8,
    /// 11 bits (0-2047)
    /// low 8 bits in NR33, high 3 bits in NR34
    period_register: u16,
    /// 16 bytes of wave pattern RAM (32 4-bit samples)
    wave_ram: [u8; 16],

    frame_sequencer: FrameSequencer,
    length_timer: LengthTimer,

    trigger_pending: bool,
    length_enabled: bool,

    current_period: usize, // copy of the period register for this duty cycle
    current_volume: u8, // copy of the initial volume for this duty cycle
    wave_index: usize, // current index into the wave pattern (0-31)
}

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            active: false,
            dac_enabled: false,
            initial_length_timer: 0,
            volume_register: 0,
            period_register: 0,
            wave_ram: [0; 16],
            frame_sequencer: FrameSequencer::default(),
            length_timer: LengthTimer::wave_channel(0),
            trigger_pending: false,
            length_enabled: false,
            current_period: 0,
            current_volume: 0,
            // When CH3 is started, the first sample read is the one at index 1,
            // i.e. the lower nibble of the first byte, NOT the upper nibble.
            wave_index: 1,
        }
    }
}

impl WaveChannel {
    pub fn nr30(&self) -> u8 {
        if self.dac_enabled {
            0x80
        } else {
            0x00
        }
    }

    pub fn set_nr30(&mut self, value: u8) {
        self.dac_enabled = value & 0x80 != 0;
    }

    pub fn nr31(&self) -> u8 {
        self.initial_length_timer
    }

    pub fn set_nr31(&mut self, value: u8) {
        self.initial_length_timer = value;
    }

    pub fn nr32(&self) -> u8 {
        (self.volume_register & 0b11) << 5
    }

    pub fn set_nr32(&mut self, value: u8) {
        self.volume_register = (value >> 5) & 0b11;
    }

    pub fn nr33(&self) -> u8 {
        (self.period_register & 0x00FF) as u8
    }

    pub fn set_nr33(&mut self, value: u8) {
        self.period_register = (self.period_register & 0xFF00) | value as u16;
    }

    pub fn nr34(&self) -> u8 {
        let mut value = ((self.period_register >> 8) & 0b111) as u8;
        if self.length_enabled {
            value |= 0b01000000;
        }
        value
    }

    pub fn set_nr34(&mut self, value: u8) {
        self.period_register = (self.period_register & 0x00FF) | (((value & 0b111) as u16) << 8);
        self.length_enabled = value & 0b01000000 != 0;
        if value & 0b10000000 != 0 {
            self.trigger_pending = true;
        }
    }

    pub fn wave_ram(&self, index: usize) -> u8 {
        self.wave_ram[index]
    }

    pub fn set_wave_ram(&mut self, index: usize, value: u8) {
        self.wave_ram[index] = value;
    }

    pub fn reset(&mut self) {
        self.frame_sequencer.reset();
    }

    pub fn trigger(&mut self) {
        self.active = true;
        self.length_timer.reset(self.initial_length_timer);
        self.current_period = self.period_register as usize;
        self.current_volume = self.volume_register;
        self.wave_index = 0;
    }

    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if self.trigger_pending {
            self.trigger();
            self.trigger_pending = false;
        }

        if !self.active {
            return;
        }

        let events = self.frame_sequencer.update(div_clocks);

        self.update_wave_duty(delta);

        if events.contains(FrameSequencerEvent::LengthCounter) {
            self.update_length_counter();
        }

        // TODO wave output logic
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

