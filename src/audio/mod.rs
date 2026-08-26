use bincode::{Decode, Encode};
use frame_sequencer::FrameSequencer;
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
    /// Band-limited synthesis and resampling to the sink's rate. Also supplies the DC blocker that
    /// used to be a separate `CapacitanceFilter` — see [`blip::DEFAULT_BASS_HZ`].
    output: BlipStereo,
    /// Length in M-cycles of the instruction the CPU is executing. Set once per instruction; see
    /// [`Audio::set_instruction_length`]. Transient, so it is excluded from `PartialEq` and from the
    /// `apu` save-state section, exactly as `output` is.
    access_machine_cycles: u8,
    /// The mixer's current output level (C4). Derived from the channels, the panning and the
    /// master volume, so — like `output` — it is neither serialised nor part of equality; a
    /// restored `Audio` recomputes it on its first update because `mix_dirty` starts set.
    mixed: AudioSample,
    /// The packed channel levels [`Audio::mixed`] was computed from. Derived, like `mixed`.
    levels: u32,
    /// Something the levels cannot show has moved — panning, master volume, the power switch — so
    /// [`Audio::mixed`] is stale. Set by every register write, which is cheap and cannot be wrong.
    mix_dirty: bool,
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
        }
    }
}

impl Audio {
    /// Retune the resampler to the sink's rate.
    ///
    /// Not part of the serialised state (see the `Encode`/`Decode` impls below), so a caller that
    /// loads a save state has to re-apply this afterwards.
    pub fn set_output_sample_rate(&mut self, sample_rate: u32) {
        self.output.set_sample_rate(sample_rate);
    }

    /// The rate the resampler is currently producing at.
    ///
    /// Exists so that "every `load_state` has to re-apply this" can be *asserted* rather than
    /// inferred from how much audio came out — see `host::tests`. It reads derived state, so it
    /// takes no part in equality or serialisation.
    pub fn output_sample_rate(&self) -> u32 {
        self.output.sample_rate()
    }

    /// Tell the resampler how fast the emulator is running relative to real time (1.0 = realtime).
    ///
    /// Without this, fast-forwarding produces audio faster than the sink drains it: the queue grows
    /// without bound, latency climbs, and the buffer eventually starts dropping the backlog. With
    /// it, the sped-up audio plays back sped-up — higher pitched, like fast-forwarding a tape.
    ///
    /// Like the output sample rate, this is not part of the serialised state.
    pub fn set_emulation_speed(&mut self, speed: f64) {
        self.output.set_speed(speed);
    }

    /// What [`Self::set_emulation_speed`] was last told. Same reason as
    /// [`Self::output_sample_rate`]: a missed re-apply should fail a test, not a listener's ear.
    pub fn emulation_speed(&self) -> f64 {
        self.output.speed()
    }

    /// Fill `out` with interleaved L/R frames, returning the number of *frames* written; zero means
    /// nothing was ready.
    ///
    /// Knows nothing about the sink: an audio queue, a WAV file and a network stream all look the
    /// same from here. [`BlipStereo::read_interleaved_i16`] is the 16-bit equivalent, if a sink ever
    /// wants one.
    pub fn read_samples_f32(&mut self, out: &mut [f32]) -> usize {
        self.output.read_interleaved_f32(out)
    }

    fn reset(&mut self) {
        self.frame_sequencer.reset();
        self.panning = Panning::default();
        self.master_volume = MasterVolume::default();
        self.channel1 = SquareWaveChannel::channel1();
        self.channel2 = SquareWaveChannel::channel2();
        self.channel3.reset(); // not all of the wave channel is reset
        self.channel4 = NoiseChannel::default();
        // Deliberately *not* clearing the output buffer, which is what the old ring buffer did here.
        // A power-off already drives the mix to zero through `push_sample`, so the synth ramps down
        // on its own; throwing away audio the sink has not read yet would just add a click.
    }

    /// Advance the APU by `delta` and hand the resampler whatever the mixer is putting out.
    ///
    /// **C4: the mix is recomputed only when something feeding it moves.** This runs once per CPU
    /// instruction, and the four `output_f32()`s, four pans, multiply and divide below were
    /// measured at 10.5% of the whole emulator — spent, overwhelmingly, arriving at a number
    /// identical to last time. Each channel now reports whether its digital level changed, and
    /// [`Audio::mix_dirty`] covers everything the channels cannot see: panning, master volume and
    /// the power switch, all of which only move on a register write.
    ///
    /// The output is bit-identical either way: `mixed` is exactly the value the old code would
    /// have recomputed, and the resampler still gets a call every instruction so the 16.16 time
    /// cursor advances as before.
    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if !self.enabled {
            self.mixed = AudioSample::ZERO;
            self.push_sample(delta, AudioSample::ZERO);
            return;
        }

        let events = self.frame_sequencer.update(div_clocks);
        self.channel1.update(delta, events);
        self.channel2.update(delta, events);
        self.channel3.update(delta, events);
        self.channel4.update(delta, events);

        // When all four channel DACs are off, the master volume units are disconnected from the
        // sound output and the output level becomes 0.
        //
        // ⚠️ Kept **ahead of `digital_levels`**, not folded into it. It asks the same question far
        // more cheaply, and it is the state a test ROM sits in for its whole run — blargg's power
        // the APU on and never play a note. Computing the packed levels there instead cost 10% of
        // `cpu_instrs`, buying a mix-skip that this branch already gives for free.
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
            // ⭐ Only *now* is there a transition to report. `BlipStereo::update` quantises with a
            // libm `roundf` per channel before `BlipSynth` discovers the amplitude has not moved —
            // `perf` put 5.5% of the whole emulator in `roundf` alone, essentially all of it
            // arriving at last instruction's answer. The resampler's clock still advances every
            // instruction, so the output is bit-identical.
            self.output.update(self.mixed);
        }
        // ⚠️ The two early returns above hand the resampler a literal `ZERO` instead, and must keep
        // doing so: leaving either state needs a register write, and `Audio::write` sets
        // `mix_dirty`, so this branch is guaranteed to re-report `mixed` on the way back.
        self.output.end_frame(delta.m_cycles() as u32);
    }

    /// How many **video** M-cycles the APU can be left alone for, or `None` if nothing is
    /// clocking. This is the bound C2's HALT skip must respect on the audio side.
    ///
    /// Only the four phase timers are in here, and that is the whole story: everything else that
    /// moves a channel's level — length counters, volume envelopes, the sweep — hangs off the
    /// frame sequencer, which hangs off DIV, and DIV is [`Ev::Divider`](crate::schedule::Ev). The
    /// rest only moves on a register write, which a halted CPU cannot make.
    ///
    /// **One M-cycle short of the clock, deliberately.** [`Audio::push_sample`] reports a level as
    /// changing at the *start* of the window it is given, so a skip that swallowed the clock would
    /// backdate the transition by the whole span — audible jitter, since HALT is 65% of Pokémon's
    /// cycles. Stopping a cycle early leaves the transition to a one-cycle step, which is exactly
    /// what the per-instruction driver used to produce.
    pub fn next_event(&self) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        let soonest = [
            self.channel1.next_event(),
            self.channel2.next_event(),
            self.channel3.next_event(),
            self.channel4.next_event(),
        ]
        .into_iter()
        .flatten()
        .min()?;
        Some(soonest.saturating_sub(1).max(1))
    }

    /// All four channels' DAC inputs packed into one word, so "has anything moved?" is a single
    /// comparison. `0xFF` marks a disconnected DAC — no channel can produce it, levels being 4-bit.
    ///
    /// Asking the channels once here beat having each `update` report its own change: that needed
    /// the level computed twice per channel and split every `update` in two, and measured *slower*
    /// than the mixing it saved.
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
    ///
    /// The level is reported as changing at the *start* of the window, which is what the old
    /// zero-order-hold loop here effectively did when it pushed `delta` copies of one value.
    ///
    /// There is no frame-time bookkeeping because there does not need to be any: the buffer's time
    /// cursor is 16.16 fixed point and carries its fractional part across calls, so ending a frame
    /// every instruction still lands every transition on the correct sub-sample phase. It also
    /// keeps latency at the kernel tail (8 output samples) rather than a chunk size.
    fn push_sample(&mut self, delta: MachineCycles, sample: AudioSample) {
        self.output.update(sample);
        self.output.end_frame(delta.m_cycles() as u32);
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
            0xFF21 => self.channel4.nr42_volume_and_envelope(), // NR42: Channel 4 volume and envelope register
            0xFF22 => self.channel4.nr43_frequency_and_randomness(), // NR43: Channel 4 frequency and randomness
            0xFF23 => self.channel4.nr44_control(), // NR44: Channel 4 control
            0xFF24 => self.nr50_master_volume(), // NR50: Sound volume register
            0xFF25 => self.nr51_panning(), // NR51: Sound panning register
            0xFF26 => self.nr52_master_control(), // NR52: Sound control register
            0xFF30..=0xFF3F => self.channel3().wave_ram((address - 0xFF30) as usize, self.access_offset()), // Wave RAM (0xFF30-0xFF3F)
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
        // Any APU register write can move the mixer's output, and several do so without the
        // channels seeing it at all (NR50/NR51/NR52). Marking it here rather than per register is
        // both cheaper and impossible to get wrong — see `Audio::update`.
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
                    // ignore other audio registers for now
                }
            }
        }
    }

    /// Tell the APU how long, in M-cycles, the instruction now executing is.
    ///
    /// Peripherals are still advanced once per instruction — this changes nothing about *when*
    /// they run. It only lets the APU work out *where* the CPU's bus access sits inside the
    /// instruction it is about to be advanced over, which is the one thing DMG's wave-RAM
    /// aperture depends on: that window is a single tick wide, so "somewhere in this instruction"
    /// is not good enough. Only [`WaveChannel`] reads it.
    pub fn set_instruction_length(&mut self, machine_cycles: u8) {
        self.access_machine_cycles = machine_cycles;
    }

    /// Where in the current instruction the CPU's bus access falls, in wave-timer ticks (2
    /// T-cycles each). Hardware puts a load's or store's memory access in the instruction's final
    /// M-cycle, so it is one M-cycle short of the whole instruction.
    fn access_offset(&self) -> u16 {
        (self.access_machine_cycles.saturating_sub(1) as u16) * 2
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
        writer.write(labels::APU, APU_SECTION_VERSION, &ApuSection {
            enabled: self.enabled,
            panning: self.panning,
            master_volume: self.master_volume.clone(),
            frame_sequencer: self.frame_sequencer.clone(),
            channel1: self.channel1.clone(),
            channel2: self.channel2.clone(),
            channel3: self.channel3.clone(),
            channel4: self.channel4.clone(),
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
            // `mixed` is derived and not in the section, so the restored machine must recompute it
            // before trusting it — see `Audio::update`.
            self.mix_dirty = true;
        }
        Ok(())
    }
}

impl PartialEq for Audio {
    fn eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled &&
            self.panning == other.panning &&
            self.master_volume == other.master_volume &&
            self.frame_sequencer == other.frame_sequencer &&
            self.channel1 == other.channel1 &&
            self.channel2 == other.channel2 &&
            self.channel3 == other.channel3 &&
            self.channel4 == other.channel4
    }
}

impl Eq for Audio {}

