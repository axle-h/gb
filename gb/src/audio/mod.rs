use bincode::{Decode, Encode};
use frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use blip::BlipStereo;
use master_volume::MasterVolume;
use square_channel::SquareWaveChannel;
use crate::audio::noise_channel::NoiseChannel;
use crate::audio::panning::Panning;
use crate::audio::sample::AudioSample;
use crate::audio::wave_channel::WaveChannel;
use crate::cycles::MachineCycles;
use crate::divider::DividerClocks;
use crate::savestate::{labels, SectionReader, SectionWriter};

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
pub mod blip;
mod timer;
#[cfg(test)]
mod reference;

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
    /// Band-limited synthesis and resampling to the sink's rate.
    output: BlipStereo,
    /// Length in M-cycles of the instruction the CPU is executing.
    access_machine_cycles: u8,
    /// The mixer's current output level.
    mixed: AudioSample,
    /// The packed channel levels [`Audio::mixed`] was computed from.
    levels: u32,
    /// Something the levels cannot show has moved — panning, master volume, the power switch — so
    /// [`Audio::mixed`] is stale.
    mix_dirty: bool,
    /// Whether the mixer and the resampler run at all.
    output_enabled: bool,
    /// Video M-cycles the four channels have not been advanced by yet, and equally the cycles the
    /// resampler's clock is behind the machine.
    pending: u64,
    /// Video M-cycles from the last flush to the soonest moment any channel's output could move
    /// on its own — the bound `pending` may not reach. `0` means "unknown, flush on the next
    /// update", which is the initial value and what every path that disturbs a channel leaves
    /// behind.
    channel_deadline: u64,
    /// Whether a batch is taken at all.
    #[cfg(all(test, not(feature = "slow-tests")))]
    batching: bool,
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
            output: BlipStereo::default(),
            access_machine_cycles: 0,
            mixed: AudioSample::ZERO,
            levels: 0,
            // Nothing has been mixed yet, so the first update must not trust `mixed`.
            mix_dirty: true,
            output_enabled: true,
            pending: 0,
            // Nothing has been measured yet, so the first update must flush rather than batch.
            channel_deadline: 0,
            #[cfg(all(test, not(feature = "slow-tests")))]
            batching: true,
        }
    }
}

impl Audio {
    /// Retune the resampler to the sink's rate.
    pub fn set_output_sample_rate(&mut self, sample_rate: u32) {
        self.output.set_sample_rate(sample_rate);
    }

    /// The rate the resampler is currently producing at.
    pub fn output_sample_rate(&self) -> u32 {
        self.output.sample_rate()
    }

    /// Tell the resampler how fast the emulator is running relative to real time (1.0 =
    /// realtime).
    pub fn set_emulation_speed(&mut self, speed: f64) {
        self.output.set_speed(speed);
    }

    /// What [`Self::set_emulation_speed`] was last told. Same reason as
    /// [`Self::output_sample_rate`]: a missed re-apply should fail a test, not a listener's ear.
    pub fn emulation_speed(&self) -> f64 {
        self.output.speed()
    }

    /// Turn the mixer and the resampler on or off, without touching the four channels.
    pub fn set_output_enabled(&mut self, enabled: bool) {
        if enabled == self.output_enabled {
            return;
        }
        // The gate cannot move with a batch outstanding.
        self.sync();
        self.output_enabled = enabled;
        if enabled {
            self.output.clear();
            self.mix_dirty = true;
        }
    }

    /// Whether the mixer and the resampler are running. Same reason as
    /// [`Self::output_sample_rate`]: a gate stuck shut should fail a test rather than a listener.
    pub fn output_enabled(&self) -> bool {
        self.output_enabled
    }

    /// How far behind the rest of the machine the four channels are being allowed to run, in
    /// video M-cycles. See the batching note in [`Audio::update`].
    #[cfg(test)]
    pub fn pending_channel_cycles(&self) -> u64 {
        self.pending
    }

    /// Turn the batch off, so a test has a per-instruction machine to compare a batching one
    /// against. The only callers are tests, and the field it sets does not exist in a release
    /// build or under `bench`; see [`Audio::deadline_after_flush`].
    #[cfg(all(test, not(feature = "slow-tests")))]
    pub fn set_channel_batching(&mut self, batching: bool) {
        self.sync();
        self.batching = batching;
    }

    /// Log every amplitude transition handed to the synth, run-length merged by
    /// [`Self::take_output_transitions`]. What the batching test compares two machines with:
    /// it is upstream of the samples and says *when* a level moved, which is the property
    /// batching has to preserve.
    #[cfg(test)]
    pub fn capture_output_transitions(&mut self) {
        self.output.start_capture();
    }

    #[cfg(test)]
    pub fn take_output_transitions(&mut self) -> Vec<(u16, i16, i16)> {
        self.output.take_capture()
    }

    /// Fill `out` with interleaved L/R frames, returning the number of *frames* written; zero
    /// means nothing was ready.
    pub fn read_samples_f32(&mut self, out: &mut [f32]) -> usize {
        self.output.read_interleaved_f32(out)
    }

    fn reset(&mut self) {
        self.pending = 0;
        self.channel_deadline = 0;
        self.frame_sequencer.reset();
        self.panning = Panning::default();
        self.master_volume = MasterVolume::default();
        self.channel1 = SquareWaveChannel::channel1();
        self.channel2 = SquareWaveChannel::channel2();
        self.channel3.reset(); // not all of the wave channel is reset
        self.channel4 = NoiseChannel::default();
    }

    /// Advance the APU by `delta` and hand the resampler whatever the mixer is putting out.
    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if !self.enabled {
            // Nothing is ever outstanding here, and it is enforced where the state is *created*
            // rather than checked here: the only way to reach a powered-off APU is a NR52 write,
            // which flushes on the way through [`Audio::write`] and is then followed by
            // [`Audio::reset`] clearing the debt outright; a fresh or restored `Audio` starts
            // clear.
            debug_assert_eq!(self.pending, 0, "a powered-off APU is carrying a batch");
            self.mixed = AudioSample::ZERO;
            if self.output_enabled {
                self.push_sample(delta, AudioSample::ZERO);
            }
            return;
        }

        let events = self.frame_sequencer.update(div_clocks);

        // The four channels are advanced to a deadline rather than once per instruction.
        let lead = self.pending;
        self.pending += delta.m_cycles();
        if events.is_empty() && self.pending < self.channel_deadline {
            return;
        }
        if !events.is_empty() {
            // The batch is paid off before the event, not with it.
            if lead > 0 {
                self.advance_channels(MachineCycles::from_m(lead), FrameSequencerEvent::empty());
            }
            self.pending = delta.m_cycles();
        }
        let batch = MachineCycles::from_m(std::mem::take(&mut self.pending));
        self.advance_channels(batch, events);
        // `None` — nothing is clocking — batches until the frame sequencer next has something to
        // say, which is the correct answer and a few thousand cycles rather than for ever.
        self.channel_deadline = self.deadline_after_flush();

        // Everything past here produces samples for somebody, and when there is nobody it is
        // skipped.
        if !self.output_enabled {
            return;
        }

        // The batched cycles, paid to the resampler's clock and to nothing else.
        if lead > 0 {
            self.output.end_frame(lead as u32);
        }

        // When all four channel DACs are off, the master volume units are disconnected from the
        // sound output and the output level becomes 0.
        if !self.channel1.dac_enabled() && !self.channel2.dac_enabled()
            && !self.channel3.dac_enabled() && !self.channel4.dac_enabled() {
            self.mixed = AudioSample::ZERO;
            self.push_sample(delta, AudioSample::ZERO);
            return;
        }

        let levels = self.digital_levels();
        if levels != self.levels || self.mix_dirty {
            self.levels = levels;
            self.mix_dirty = false;
            self.mixed = self.mix();
            // Only *now* is there a transition to report.
            self.output.update(self.mixed);
        }
        // The two early returns above hand the resampler a literal `ZERO` instead, and must keep
        // doing so: leaving either state needs a register write, and `Audio::write` sets
        // `mix_dirty`, so this branch is guaranteed to re-report `mixed` on the way back.
        self.output.end_frame(delta.m_cycles() as u32);
    }

    /// How many video M-cycles the APU can be left alone for, or `None` if nothing is clocking.
    pub fn next_event(&self) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        // Net of the outstanding batch. The channels are up to `pending` M-cycles behind the rest
        // of the machine (see [`Audio::update`]), so their own answer is that much too late.
        let soonest = self.soonest_channel_event()?.saturating_sub(self.pending);
        Some(soonest.saturating_sub(1).max(1))
    }

    /// Video M-cycles from where the *channels* have got to until the soonest of them could move
    /// its own output, or `None` if none of them is clocking at all.
    #[inline]
    fn soonest_channel_event(&self) -> Option<u64> {
        [
            self.channel1.next_event(),
            self.channel2.next_event(),
            self.channel3.next_event(),
            self.channel4.next_event(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Pay off the outstanding batch, so the channels are where the rest of the machine is.
    fn sync(&mut self) {
        self.channel_deadline = 0;
        if self.pending == 0 {
            return;
        }
        let delta = MachineCycles::from_m(std::mem::take(&mut self.pending));
        self.advance_channels(delta, FrameSequencerEvent::empty());
        // And the resampler's clock with them.
        if self.output_enabled {
            self.output.end_frame(delta.m_cycles() as u32);
        }
    }

    /// How long the channels may be left alone from a flush that has just happened.
    #[inline]
    fn deadline_after_flush(&self) -> u64 {
        #[cfg(all(test, not(feature = "slow-tests")))]
        {
            if !self.batching {
                return 0;
            }
        }
        self.soonest_channel_event().unwrap_or(u64::MAX)
    }

    /// Hand all four channels one window.
    #[inline]
    fn advance_channels(&mut self, delta: MachineCycles, events: FrameSequencerEvent) {
        self.channel1.update(delta, events);
        self.channel2.update(delta, events);
        self.channel3.update(delta, events);
        self.channel4.update(delta, events);
    }

    /// The four channels as they would be with the batch paid off, without paying it off.
    fn settled(&self) -> (SquareWaveChannel, SquareWaveChannel, WaveChannel, NoiseChannel) {
        let (mut c1, mut c2, mut c3, mut c4) =
            (self.channel1.clone(), self.channel2.clone(), self.channel3.clone(), self.channel4.clone());
        if self.pending > 0 {
            let delta = MachineCycles::from_m(self.pending);
            let events = FrameSequencerEvent::empty();
            c1.update(delta, events);
            c2.update(delta, events);
            c3.update(delta, events);
            c4.update(delta, events);
        }
        (c1, c2, c3, c4)
    }

    #[inline]
    fn digital_levels(&self) -> u32 {
        fn packed(level: Option<u8>) -> u32 {
            level.unwrap_or(0xFF) as u32
        }
        packed(self.channel1.digital_level())
            | packed(self.channel2.digital_level()) << 8
            | packed(self.channel3.digital_level()) << 16
            | packed(self.channel4.digital_level()) << 24
    }

    /// Unreachable with every DAC off — [`Audio::update`] returns before it.
    fn mix(&self) -> AudioSample {
        let channel1 = self.panning.channel1.pan(self.channel1.output_f32());
        let channel2 = self.panning.channel2.pan(self.channel2.output_f32());
        let channel3 = self.panning.channel3.pan(self.channel3.output_f32());
        let channel4 = self.panning.channel4.pan(self.channel4.output_f32());

        let volume = self.master_volume.volume_sample();
        volume * (channel1 + channel2 + channel3 + channel4) / 4.0
    }

    /// Hand the mixed output level to the resampler and advance its clock by `delta`.
    fn push_sample(&mut self, delta: MachineCycles, sample: AudioSample) {
        self.output.update(sample);
        self.output.end_frame(delta.m_cycles() as u32);
    }

    pub fn nr52_master_control(&self) -> u8 {
        // Bits 4-6 are always 1
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
        // The rest of this register is not writable
        if self.enabled && !enable {
            // Apu registers are cleared on the transition from 1 to 0 of bit 7
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
        // Not writable if APU is disabled
        if self.enabled {
            self.panning.set_byte(value);
        }
    }

    pub fn nr50_master_volume(&self) -> u8 {
        self.master_volume.get_byte()
    }

    pub fn set_nr50_master_volume(&mut self, value: u8) {
        // Not writable if APU is disabled
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
            0xFF21 => self.channel4.nr42_volume_and_envelope(), // NR42: Channel 4 volume and envelope register
            0xFF22 => self.channel4.nr43_frequency_and_randomness(), // NR43: Channel 4 frequency and randomness
            0xFF23 => self.channel4.nr44_control(), // NR44: Channel 4 control
            0xFF24 => self.nr50_master_volume(), // NR50: Sound volume register
            0xFF25 => self.nr51_panning(), // NR51: Sound panning register
            0xFF26 => self.nr52_master_control(), // NR52: Sound control register
            0xFF30..=0xFF3F => self.channel3().wave_ram((address - 0xFF30) as usize, self.access_offset()), // Wave RAM (0xFF30-0xFF3F)
            _ => {
                // Ignore other audio registers for now
                0xFF
            }
        };

        // Println!("Read from audio register: {:04X} = {:02X}", address, value);
        value
    }

    pub fn write(&mut self, address: u16, value: u8) {
        // Println!("Write to audio register: {:04X} = {:02X}", address, value); Before anything
        // else.
        self.sync();
        // Any APU register write can move the mixer's output, and several do so without the
        // channels seeing it at all (NR50/NR51/NR52).
        self.mix_dirty = true;
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
                0xFF1E => self.channel3.set_nr34_period_high_and_control(value, &self.frame_sequencer, self.access_offset()), // NR34: Channel 3 frequency high and control
                0xFF20 => self.channel4.set_nr41_length_timer(value), // NR41: Channel 4 length register
                0xFF21 => self.channel4.set_nr42_volume_and_envelope_mut(value), // NR42: Channel 4 volume and envelope register
                0xFF22 => self.channel4.set_nr43_frequency_and_randomness(value), // NR43: Channel 4 frequency and randomness
                0xFF23 => self.channel4.set_nr44_control(value, &self.frame_sequencer), // NR44: Channel 4 control
                0xFF24 => self.set_nr50_master_volume(value), // NR50: Sound volume register
                0xFF25 => self.set_nr51_panning_mut(value), // NR51: Sound panning register
                0xFF26 => self.set_nr52_master_control(value), // NR52: Sound control register
                0xFF30..=0xFF3F => { let offset = self.access_offset(); self.channel3_mut().set_wave_ram((address - 0xFF30) as usize, value, offset) } // Wave RAM (0xFF30-0xFF3F)
                _ => {
                    // Ignore other audio registers for now
                }
            }
        }
    }

    /// Tell the APU how long, in M-cycles, the instruction now executing is.
    pub fn set_instruction_length(&mut self, machine_cycles: u8) {
        self.access_machine_cycles = machine_cycles;
    }

    /// Where in the current instruction the CPU's bus access falls, in wave-timer ticks (2
    /// T-cycles each).
    fn access_offset(&self) -> u16 {
        // Plus the outstanding batch, because the offset is measured from where the wave timer
        // has actually got to and [`Audio::update`] may have left it up to `pending` M-cycles
        // short of the instruction boundary.
        let batch = u16::try_from(self.pending).unwrap_or(u16::MAX).saturating_mul(2);
        ((self.access_machine_cycles.saturating_sub(1) as u16) * 2).saturating_add(batch)
    }

    pub fn channel3(&self) -> &WaveChannel {
        &self.channel3
    }
    
    pub fn channel3_mut(&mut self) -> &mut WaveChannel {
        &mut self.channel3
    }

}

/// Contents of the `apu` save-state section. Excludes `output` — the resampler is a sink, not
/// machine state.
#[derive(Debug, Clone, Decode, Encode)]
pub struct ApuSection {
    pub enabled: bool,
    pub panning: Panning,
    pub master_volume: MasterVolume,
    pub frame_sequencer: FrameSequencer,
    pub channel1: SquareWaveChannel,
    pub channel2: SquareWaveChannel,
    pub channel3: WaveChannel,
    pub channel4: NoiseChannel,
}

pub const APU_SECTION_VERSION: u16 = 1;

impl Audio {
    pub(crate) fn write_sections(&self, writer: &mut SectionWriter) -> Result<(), String> {
        // Settled, not as they stand: the channels may be running a batch behind the CPU (see
        // [`Audio::update`]) and a save state has to be the machine at one moment.
        let (channel1, channel2, channel3, channel4) = self.settled();
        writer.write(labels::APU, APU_SECTION_VERSION, &ApuSection {
            enabled: self.enabled,
            panning: self.panning,
            master_volume: self.master_volume.clone(),
            frame_sequencer: self.frame_sequencer.clone(),
            channel1,
            channel2,
            channel3,
            channel4,
        })
    }

    pub(crate) fn read_sections(&mut self, reader: &SectionReader) -> Result<(), String> {
        if let Some((_version, section)) = reader.read::<ApuSection>(labels::APU)? {
            self.enabled = section.enabled;
            self.panning = section.panning;
            self.master_volume = section.master_volume;
            self.frame_sequencer = section.frame_sequencer;
            self.channel1 = section.channel1;
            self.channel2 = section.channel2;
            self.channel3 = section.channel3;
            self.channel4 = section.channel4;
            // `mixed` is derived and not in the section, so the restored machine must recompute
            // it before trusting it — see `Audio::update`.
            self.mix_dirty = true;
            // The section was written settled, so the restored channels owe nothing; the deadline
            // that let them fall behind belonged to the machine that was saved, not this one.
            self.pending = 0;
            self.channel_deadline = 0;
        }
        Ok(())
    }
}

impl PartialEq for Audio {
    /// Compares the channels settled, for the same reason the save-state section writes them
    /// settled: two machines at the same instant may be carrying different batches (see
    /// [`Audio::update`]), and one stepping every M-cycle against one taking the HALT skip is
    /// precisely what `the_halt_fast_path_matches_stepping_cycle_by_cycle` builds.
    fn eq(&self, other: &Self) -> bool {
        let header = self.enabled == other.enabled &&
            self.panning == other.panning &&
            self.master_volume == other.master_volume &&
            self.frame_sequencer == other.frame_sequencer;
        if !header {
            return false;
        }
        if self.pending == 0 && other.pending == 0 {
            return self.channel1 == other.channel1 &&
                self.channel2 == other.channel2 &&
                self.channel3 == other.channel3 &&
                self.channel4 == other.channel4;
        }
        self.settled() == other.settled()
    }
}

impl Eq for Audio {}
