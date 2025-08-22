use channel::Channel;
use master_control::MasterControlRegister;
use panning::AudioPanningRegister;
use master_volume::MasterVolumeRegister;
use square_channel::SquareWaveChannel;
use crate::audio::channel::Channel::{Channel1, Channel2};
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

#[derive(Debug, Clone)]
pub struct Audio {
    control: MasterControlRegister,
    panning: AudioPanningRegister,
    master_volume: MasterVolumeRegister,
    channel1: SquareWaveChannel,
    channel2: SquareWaveChannel,

    output_left: f32,
    output_right: f32
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            control: MasterControlRegister::default(),
            panning: AudioPanningRegister::default(),
            master_volume: MasterVolumeRegister::default(),
            channel1: SquareWaveChannel::channel1(),
            channel2: SquareWaveChannel::channel2(),
            output_left: 0.0,
            output_right: 0.0
        }
    }
}

impl Audio {
    pub fn output(&self) -> (f32, f32) {
        (self.output_left, self.output_right)
    }

    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if !self.control.is_enabled() {
            return;
        }

        self.channel1.update(delta, div_clocks);
        self.control.set_channel_enabled(Channel1, self.channel1.is_active());

        self.channel2.update(delta, div_clocks);
        self.control.set_channel_enabled(Channel1, self.channel2.is_active());

        let (ch1_left, ch1_right) = self.panning.panning(Channel1).pan(self.channel1.output_f32());
        let (ch2_left, ch2_right) = self.panning.panning(Channel2).pan(self.channel2.output_f32());
        // TODO channel 3, 4

        self.output_left = self.master_volume.left_volume_f32() * (ch1_left + ch2_left /* + ch3_left + ch4_left*/) / 2.0;
        self.output_right = self.master_volume.right_volume_f32() * (ch1_right + ch2_right  /*+ ch3_left + ch4_left*/) / 2.0;
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
}

