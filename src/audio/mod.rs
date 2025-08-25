use std::collections::VecDeque;
use master_control::MasterControlRegister;
use panning::AudioPanningRegister;
use master_volume::MasterVolumeRegister;
use square_channel::SquareWaveChannel;
use crate::audio::channel::Channel::{Channel1, Channel2};
use crate::audio::sample::AudioSample;
use crate::audio::wave_channel::WaveChannel;
use crate::cycles::MachineCycles;
use crate::divider::DividerClocks;

pub mod master_control;
pub mod panning;
pub mod channel;
pub mod master_volume;
mod sweep;
mod length;
mod volume;
mod control;
mod square_channel;
mod frame_sequencer;
mod sample;
mod dac;
mod wave_channel;

pub const GB_SAMPLE_RATE: usize = 1048576; // Game Boy native audio frequency

#[derive(Debug, Clone)]
pub struct Audio {
    control: MasterControlRegister,
    panning: AudioPanningRegister,
    master_volume: MasterVolumeRegister,
    channel1: SquareWaveChannel,
    channel2: SquareWaveChannel,
    channel3: WaveChannel,
    buffer: VecDeque<f32>,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            control: MasterControlRegister::default(),
            panning: AudioPanningRegister::default(),
            master_volume: MasterVolumeRegister::default(),
            channel1: SquareWaveChannel::channel1(),
            channel2: SquareWaveChannel::channel2(),
            channel3: WaveChannel::default(),
            buffer: VecDeque::with_capacity(2 * GB_SAMPLE_RATE / 10), // buffer for 100ms of audio, 2 channels
        }
    }
}

impl Audio {
    pub fn buffer_mut(&mut self) -> &mut VecDeque<f32> {
        &mut self.buffer
    }

    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if !self.control.is_enabled() {
            self.push_sample(delta, AudioSample::ZERO);
            return;
        }

        self.channel1.update(delta, div_clocks);
        self.control.set_channel_enabled(Channel1, self.channel1.is_active());

        self.channel2.update(delta, div_clocks);
        self.control.set_channel_enabled(Channel1, self.channel2.is_active());

        let channel1 = self.panning.panning(Channel1).pan(self.channel1.output_f32());
        let channel2 = self.panning.panning(Channel2).pan(self.channel2.output_f32());
        // TODO channel 3, 4

        let volume = self.master_volume.volume_sample();
        let sample = volume * (channel1 + channel2) / 2.0;
        // TODO implement DAC capacitor effect
        self.push_sample(delta, sample);
    }

    fn push_sample(&mut self, delta: MachineCycles, sample: AudioSample) {
        for _ in 0..delta.m_cycles() {
            self.buffer.push_back(sample.left);
            self.buffer.push_back(sample.right);
            if self.buffer.len() >= self.buffer.capacity() {
                // audio buffer overflow :-(
                self.buffer.drain(..2);
            }
        }
    }

    pub fn control(&self) -> &MasterControlRegister {
        &self.control
    }

    pub fn control_mut(&mut self) -> &mut MasterControlRegister {
        &mut self.control
    }

    pub fn panning(&self) -> &AudioPanningRegister {
        &self.panning
    }

    pub fn panning_mut(&mut self) -> &mut AudioPanningRegister {
        &mut self.panning
    }

    pub fn master_volume(&self) -> &MasterVolumeRegister {
        &self.master_volume
    }

    pub fn master_volume_mut(&mut self) -> &mut MasterVolumeRegister {
        &mut self.master_volume
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
}

