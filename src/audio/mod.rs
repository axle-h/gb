use std::collections::VecDeque;
use frame_sequencer::FrameSequencer;
use filters::CapacitanceFilter;
use master_volume::MasterVolume;
use square_channel::SquareWaveChannel;
use crate::audio::noise_channel::NoiseChannel;
use crate::audio::panning::Panning;
use crate::audio::sample::AudioSample;
use crate::audio::wave_channel::WaveChannel;
use crate::cycles::MachineCycles;
use crate::divider::DividerClocks;

pub mod panning;
pub mod master_volume;
pub mod sweep;
pub mod length;
pub mod volume;
pub mod square_channel;
pub mod frame_sequencer;
pub mod sample;
pub mod dac;
pub mod wave_channel;
pub mod noise_channel;
mod filters;
mod timer;

pub const GB_SAMPLE_RATE: usize = 1048576; // Game Boy native audio frequency

#[derive(Debug, Clone)]
pub struct Audio {
    enabled: bool,
    panning: Panning,
    master_volume: MasterVolume,
    frame_sequencer: FrameSequencer,
    channel1: SquareWaveChannel,
    channel2: SquareWaveChannel,
    channel3: WaveChannel,
    channel4: NoiseChannel,
    high_pass_filter: CapacitanceFilter,
    buffer: VecDeque<f32>,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            enabled: false,
            panning: Panning::default(),
            master_volume: MasterVolume::default(),
            frame_sequencer: FrameSequencer::default(),
            channel1: SquareWaveChannel::channel1(),
            channel2: SquareWaveChannel::channel2(),
            channel3: WaveChannel::default(),
            channel4: NoiseChannel::default(),
            high_pass_filter: CapacitanceFilter::default(),
            buffer: VecDeque::with_capacity(2 * GB_SAMPLE_RATE / 10), // buffer for 100ms of audio, 2 channels
        }
    }
}

impl Audio {
    pub fn buffer_mut(&mut self) -> &mut VecDeque<f32> {
        &mut self.buffer
    }

    fn reset(&mut self) {
        self.frame_sequencer.reset();
        self.panning = Panning::default();
        self.master_volume = MasterVolume::default();
        self.channel1 = SquareWaveChannel::channel1();
        self.channel2 = SquareWaveChannel::channel2();
        self.channel3.reset(); // not all of the wave channel is reset
        self.channel4 = NoiseChannel::default();
        self.buffer.clear();
    }

    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if !self.enabled {
            self.push_sample(delta, AudioSample::ZERO);
            return;
        }

        let events = self.frame_sequencer.update(div_clocks);
        self.channel1.update(delta, events);
        self.channel2.update(delta, events);
        self.channel3.update(delta, events);
        self.channel4.update(delta, events);

        if !self.channel1.dac_enabled() && !self.channel2.dac_enabled() && !self.channel3.dac_enabled() && !self.channel4.dac_enabled() {
            // When all four channel DACs are off, the master volume units are disconnected from the sound output and the output level becomes 0
            self.push_sample(delta, AudioSample::ZERO);
            return;
        }

        let channel1 = self.panning.channel1.pan(self.channel1.output_f32());
        let channel2 = self.panning.channel2.pan(self.channel2.output_f32());
        let channel3 = self.panning.channel3.pan(self.channel3.output_f32());
        let channel4 = AudioSample::ZERO; //self.panning.channel4.pan(self.channel4.output_f32());

        let volume = self.master_volume.volume_sample();
        let sample = volume * (channel1 + channel2 + channel3 + channel4) / 4.0;
        self.push_sample(delta, sample);
    }

    fn push_sample(&mut self, delta: MachineCycles, sample: AudioSample) {
        for _ in 0..delta.m_cycles() {
            let filtered_sample = self.high_pass_filter.process(sample);
            self.buffer.push_back(filtered_sample.left);
            self.buffer.push_back(filtered_sample.right);
            if self.buffer.len() >= self.buffer.capacity() {
                // audio buffer overflow :-(
                self.buffer.drain(..2);
            }
        }
    }

    pub fn nr52_master_control(&self) -> u8 {
        // bits 4-6 are always 1
        let mut byte = 0x70;
        if self.enabled {
            byte |= 0x80; // Bit 7: Master enable
        }
        if self.channel1.is_active() {
            byte |= 0x01; // Bit 0: Channel 1 enable
        }
        if self.channel2.is_active() {
            byte |= 0x02; // Bit 1: Channel 2 enable
        }
        if self.channel3.is_active() {
            byte |= 0x04; // Bit 2: Channel 3 enable
        }
        if self.channel4.is_active() {
            byte |= 0x08; // Bit 3: Channel 4 enable
        }
        byte
    }

    pub fn set_nr52_master_control(&mut self, value: u8) {
        let enable = (value & 0x80) != 0; // Bit 7: Master enable
        // the rest of this register is not writable
        if self.enabled && !enable {
            // apu registers are cleared on the transition from 1 to 0 of bit 7
            self.reset();
        } else if !self.enabled && enable {
            // Reset frame sequencer when APU is re-enabled
            self.frame_sequencer.reset_to_max();
        }
        self.enabled = enable;
    }

    pub fn nr51_panning(&self) -> u8 {
        self.panning.get_byte()
    }

    pub fn set_nr51_panning_mut(&mut self, value: u8) {
        // not writable if APU is disabled
        if self.enabled {
            self.panning.set_byte(value);
        }
    }

    pub fn nr50_master_volume(&self) -> u8 {
        self.master_volume.get_byte()
    }

    pub fn set_nr50_master_volume(&mut self, value: u8) {
        // not writable if APU is disabled
        if self.enabled {
            self.master_volume.set_byte(value)
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        let value = match address {
            0xFF10 => self.channel1.nr10(), // NR10: Channel 1 sweep register
            0xFF11 => self.channel1.nrx1_length_timer_duty_cycle(), // NR11: Channel 1 length and duty register
            0xFF12 => self.channel1.volume_envelope_register().get(), // NR12: Channel 1 volume and envelope register
            0xFF13 => self.channel1.nrx3_period_low(), // NR13: Channel 1 period low byte
            0xFF14 => self.channel1.nrx4_period_high_and_control(), // NR14: Channel 1 period high byte and control
            0xFF16 => self.channel2.nrx1_length_timer_duty_cycle(), // NR21: Channel 2 length and duty register
            0xFF17 => self.channel2.volume_envelope_register().get(), // NR22: Channel 2 volume and envelope register
            0xFF18 => self.channel2.nrx3_period_low(), // NR23: Channel 2 period low byte
            0xFF19 => self.channel2.nrx4_period_high_and_control(), // NR24: Channel 2 period high byte and control
            0xFF1A => self.channel3.nr30(), // NR30: Channel 3 DAC power
            0xFF1B => self.channel3.nr31_length_timer(), // NR31: Channel 3 length timer
            0xFF1C => self.channel3.nr32_output_level(), // NR32: Channel 3 output level
            0xFF1D => self.channel3.nr33_period_low(), // NR33: Channel 3 frequency low
            0xFF1E => self.channel3.nr34_period_high_and_control(), // NR34: Channel 3 frequency high and control
            0xFF20 => self.channel4.nr41_length_timer(), // NR41: Channel 4 length register
            0xFF21 => self.channel4.nr42_volume_and_envelope().get(), // NR42: Channel 4 volume and envelope register
            0xFF22 => self.channel4.nr43_frequency_and_randomness(), // NR43: Channel 4 frequency and randomness
            0xFF23 => self.channel4.nr44_control(), // NR44: Channel 4 control
            0xFF24 => self.nr50_master_volume(), // NR50: Sound volume register
            0xFF25 => self.nr51_panning(), // NR51: Sound panning register
            0xFF26 => self.nr52_master_control(), // NR52: Sound control register
            0xFF30..=0xFF3F => self.channel3().wave_ram((address - 0xFF30) as usize), // Wave RAM (0xFF30-0xFF3F)
            _ => {
                // ignore other audio registers for now
                0xFF
            }
        };

        // println!("Read from audio register: {:04X} = {:02X}", address, value);
        value
    }

    pub fn write(&mut self, address: u16, value: u8) {
        // println!("Write to audio register: {:04X} = {:02X}", address, value);
        let write_allowed = self.enabled || matches!(address, 0xFF11 | 0xFF16 | 0xFF1B | 0xFF20 | 0xFF26 | 0xFF30..=0xFF3F);
        if write_allowed {
            match address {
                0xFF10 => self.channel1.set_nr10(value), // NR10: Channel 1 sweep register
                0xFF11 => self.channel1.set_nrx1_length_timer_duty_cycle(value, self.enabled), // NR11: Channel 1 length and duty register
                0xFF12 => self.channel1.volume_envelope_register_mut().set(value), // NR12: Channel 1 volume and envelope register
                0xFF13 => self.channel1.set_nrx3_period_low(value), // NR13: Channel 1 period low byte
                0xFF14 => self.channel1.set_nrx4_period_high_and_control(value, &self.frame_sequencer), // NR14: Channel 1 period high byte and control
                0xFF16 => self.channel2.set_nrx1_length_timer_duty_cycle(value, self.enabled), // NR21: Channel 2 length and duty register
                0xFF17 => self.channel2.volume_envelope_register_mut().set(value), // NR22: Channel 2 volume and envelope register
                0xFF18 => self.channel2.set_nrx3_period_low(value), // NR23: Channel 2 period low byte
                0xFF19 => self.channel2.set_nrx4_period_high_and_control(value, &self.frame_sequencer), // NR24: Channel 2 period high byte and control
                0xFF1A => self.channel3.set_nr30(value), // NR30: Channel 3 DAC power
                0xFF1B => self.channel3.set_nr31_length_timer(value), // NR31: Channel 3 length timer
                0xFF1C => self.channel3.set_nr32_output_level(value), // NR32: Channel 3 output level
                0xFF1D => self.channel3.set_nr33_period_low(value), // NR33: Channel 3 frequency low
                0xFF1E => self.channel3.set_nr34_period_high_and_control(value, &self.frame_sequencer), // NR34: Channel 3 frequency high and control
                0xFF20 => self.channel4.set_nr41_length_timer(value), // NR41: Channel 4 length register
                0xFF21 => self.channel4.nr42_volume_and_envelope_mut().set(value), // NR42: Channel 4 volume and envelope register
                0xFF22 => self.channel4.set_nr43_frequency_and_randomness(value), // NR43: Channel 4 frequency and randomness
                0xFF23 => self.channel4.set_nr44_control(value, &self.frame_sequencer), // NR44: Channel 4 control
                0xFF24 => self.set_nr50_master_volume(value), // NR50: Sound volume register
                0xFF25 => self.set_nr51_panning_mut(value), // NR51: Sound panning register
                0xFF26 => self.set_nr52_master_control(value), // NR52: Sound control register
                0xFF30..=0xFF3F => self.channel3_mut().set_wave_ram((address - 0xFF30) as usize, value), // Wave RAM (0xFF30-0xFF3F)
                _ => {
                    // ignore other audio registers for now
                }
            }
        }
    }

    pub fn channel1(&self) -> &SquareWaveChannel {
        &self.channel1
    }

    pub fn channel1_mut(&mut self) -> &mut SquareWaveChannel {
        &mut self.channel1
    }

    pub fn channel2(&self) -> &SquareWaveChannel {
        &self.channel2
    }

    pub fn channel2_mut(&mut self) -> &mut SquareWaveChannel {
        &mut self.channel2
    }
    
    pub fn channel3(&self) -> &WaveChannel {
        &self.channel3
    }
    
    pub fn channel3_mut(&mut self) -> &mut WaveChannel {
        &mut self.channel3
    }

    pub fn channel4(&self) -> &NoiseChannel {
        &self.channel4
    }

    pub fn channel4_mut(&mut self) -> &mut NoiseChannel {
        &mut self.channel4
    }
}

