use crate::audio::dac::dac_sample;
use crate::audio::frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use crate::audio::length::{LengthTimer};
use crate::audio::sweep::Sweep;
use crate::audio::volume::{EnvelopeFunction, VolumeAndEnvelopeRegister};
use crate::cycles::MachineCycles;

#[derive(Debug, Clone)]
pub struct SquareWaveChannel {
    /// NR10 (channel 1 only)
    sweep: Option<Sweep>,

    /// NRx1  Length timer & duty cycle
    /// bits 6-7 Duty cycle
    /// Controls the output waveform as follows:
    /// - 00: 12.5% duty cycle
    /// - 01: 25% duty cycle
    /// - 10: 50% duty cycle
    /// - 11: 75% duty cycle
    wave_duty_cycle: u8,
    /// bits 0-5 Initial length timer
    /// The higher this field is, the shorter the time before the channel is cut.
    initial_length_timer: u8,
    length_timer: LengthTimer,

    /// NRx2 Volume & envelope
    envelope_function: EnvelopeFunction,

    /// NRx3 Period low
    /// NRx4 Period high & control
    period: u16, // 11 bits

    /// Internal state
    initialised: bool, // the channel has been triggered at least once
    active: bool, // The channel has been triggered and is active
    current_period: usize, // copy of the period register for this duty cycle
    frequency_timer: usize, // internal counter that overflows at current_period
    wave_duty_index: usize, // current index into the wave duty pattern (0-7) for the current duty cycle
    output: u8,
}

impl SquareWaveChannel {
    pub fn new(sweep_enabled: bool) -> Self {
        Self {
            sweep: if sweep_enabled { Some(Sweep::default()) } else { None },
            wave_duty_cycle: 0,
            initial_length_timer: 0,
            length_timer: LengthTimer::square_or_noise_channel(),
            envelope_function: EnvelopeFunction::default(),
            period: 0,

            active: false,
            initialised: true,
            // Just after powering on, the first duty step of the square waves after they are triggered for the first time is played as if it were 0
            current_period: 0,
            frequency_timer: 0,
            wave_duty_index: 0,
            output: 0,
        }
    }

    pub fn channel1() -> Self {
        Self::new(true)
    }

    pub fn channel2() -> Self {
        Self::new(false)
    }

    pub fn nr10(&self) -> u8 {
        self.sweep.as_ref().map(|s| s.nr10()).unwrap_or(0xFF)
    }

    pub fn set_nr10(&mut self, value: u8) {
        if let Some(sweep) = self.sweep.as_mut() {
            sweep.set_nr10(value, &mut self.active);
        }
    }

    pub fn nrx1_length_timer_duty_cycle(&self) -> u8 {
        // write only bits 0-5 always read as 1
        0x3F | ((self.wave_duty_cycle & 0x03) << 6) // Bits 6-7: Wave duty cycle
    }

    pub fn set_nrx1_length_timer_duty_cycle(&mut self, value: u8, apu_active: bool) {
        if apu_active {
            // can only set the wave duty cycle when APU is active
            self.wave_duty_cycle = (value >> 6) & 0x03; // Bits 6-7
        }
        self.initial_length_timer = value & 0x3F; // Bits 0-5

        // the length timer can be reset at any time
        self.length_timer.reset(self.initial_length_timer);
    }

    pub fn volume_envelope_register(&self) -> &VolumeAndEnvelopeRegister {
        &self.envelope_function.register()
    }

    pub fn volume_envelope_register_mut(&mut self) -> &mut VolumeAndEnvelopeRegister {
        self.envelope_function.register_mut()
    }

    pub fn nrx3_period_low(&self) -> u8 {
        0xFF // the lower 8 bits of the period, write-only bits return 1
    }

    pub fn set_nrx3_period_low(&mut self, value: u8) {
        self.period = (self.period & 0xFF00) | value as u16; // Set the lower 8 bits
    }

    pub fn nrx4_period_high_and_control(&self) -> u8 {
        // Bits 0-5 & 7 are always 1 when read
        0xBF | if self.length_timer.enabled() { 0x40 } else { 0 }
    }

    pub fn set_nrx4_period_high_and_control(&mut self, value: u8, frame_sequencer: &FrameSequencer) {
        self.period = (self.period & 0x00FF) | ((value as u16 & 0x07) << 8); // Set the upper 3 bits
        let trigger = (value & 0x80) != 0; // bit 7
        let length_enabled = (value & 0x40) != 0; // bit 6

        self.length_timer.set_enabled(length_enabled, frame_sequencer, &mut self.active);

        if trigger {
            self.trigger(frame_sequencer);
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
        if self.envelope_function.dac_enabled() && self.active {
            dac_sample(self.output)
        } else {
            0.0
        }
    }

    fn trigger(&mut self, frame_sequencer: &FrameSequencer) {
        if !self.envelope_function.dac_enabled() {
            // the length timer is still triggered even when the dac is disabled.
            self.length_timer.trigger(frame_sequencer);
            return;
        }
        self.initialised = true;
        self.active = true;
        self.length_timer.trigger(frame_sequencer);

        // When triggering Ch1 and Ch2, the low two bits of the frequency timer are NOT modified.
        self.frequency_timer = self.frequency_timer & 0b00000011;
        self.current_period = if let Some(sweep) = self.sweep.as_mut() {
            let initial_sweep = sweep.trigger(self.period as usize);
            if initial_sweep.overflows {
                self.active = false;
            }
            initial_sweep.value
        } else {
            self.period as usize
        };
        self.envelope_function.trigger();
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

        // Sweep -> Period Counter -> Duty -> Length Timer -> Envelope
        if events.is_sweep() {
            self.update_sweep();
        }

        if events.is_length_counter() {
            self.length_timer.clock(&mut self.active);
        }

        if events.is_volume_envelope() {
            self.update_volume_envelope();
        }

        // Obscure behavior: Just after powering on, the first duty step of the square waves after they are triggered for the first time is played as if it were 0.
        // Obscure behavior: the square duty sequence clocking is disabled until the first trigger.
        if !self.initialised {
            return;
        }

        self.update_wave_duty(delta);
        let waveform_bit = 7 - self.wave_duty_index;
        let waveform_sample = (self.waveform() >> waveform_bit) & 0x1;
        self.output = self.envelope_function.current_volume() * waveform_sample;
    }

    fn waveform(&self) -> u8 {
        match self.wave_duty_cycle {
            0 => 0b00000001, // 12.5% duty cycle
            1 => 0b00000011, // 25% duty cycle
            2 => 0b00001111, // 50% duty cycle
            3 => 0b11111100, // 75% duty cycle
            _ => unreachable!(), // Should never happen
        }
    }

    fn update_sweep(&mut self) {
        // channel 2 has no sweep
        if let Some(sweep) = self.sweep.as_mut() {
            if let Some(next_sweep) = sweep.clock() {
                if next_sweep.overflows {
                    self.active = false;
                } else {
                    self.current_period = next_sweep.value;
                    self.period = (next_sweep.value & 0x07FF) as u16;
                }
            }
        }
    }

    fn update_volume_envelope(&mut self) {
        self.envelope_function.step();
    }

    fn update_wave_duty(&mut self, delta: MachineCycles) {
        self.frequency_timer += delta.m_cycles();
        // https://gbdev.io/pandocs/Audio_Registers.html#ff13--nr13-channel-1-period-low-write-only
        // The channel treats the value in the period as a negative number in 11-bit two’s complement.
        // The higher the period value in the register, the lower the period, and the higher the frequency.
        // For example:
        //     Period value $500 means -$300, or 1 sample per 768 input cycles
        //     Period value $740 means -$C0, or 1 sample per 192 input cycles
        let frequency = 0x800 - self.current_period;
        while self.frequency_timer > frequency {
            // overflow, emit a sample
            self.frequency_timer -= frequency;

            self.wave_duty_index += 1;
            self.wave_duty_index %= 8;

            // Period changes (written to NR13 or NR14) only take effect after the current “sample” ends
            self.current_period = self.period as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_tone() {
        let frame_sequencer = FrameSequencer::default();
        let mut channel = SquareWaveChannel::channel2();
        channel.set_nrx1_length_timer_duty_cycle(0b10000000, true); // 50% duty cycle
        channel.set_nrx3_period_low(0x00); // low byte of period
        channel.envelope_function.register_mut().set(0xF0); // initial volume 15
        channel.set_nrx4_period_high_and_control(0b10000101, &frame_sequencer); // high byte of period (period = 0x500)

        // With a period of 0x500, we should get a frequency of 1 sample per 768 machine cycles
        // at 50% duty cycle, we should get 4 samples of 0x00 followed by 4 samples of 0xFF
        let expected_waveform: [u8; 768 * 8] = std::array::from_fn(|i| {
            let phase = i / 768;
            if phase < 4 { 0x00 } else { 0xF }
        });
        let mut waveform = [0u8; 768 * 8];
        for i in 0..waveform.len() {
            channel.update(MachineCycles::from_m(1), FrameSequencerEvent::empty());
            waveform[i] = channel.output;
        }

        assert_eq!(waveform, expected_waveform);
    }
}