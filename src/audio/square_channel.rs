use crate::activation::Activation;
use crate::audio::control::PeriodAndControlRegisters;
use crate::audio::dac::dac_sample;
use crate::audio::frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use crate::audio::length::{LengthTimer, LengthTimerAndDutyCycleRegister};
use crate::audio::sweep::{Sweep, SweepRegister};
use crate::audio::volume::{EnvelopeFunction, VolumeAndEnvelopeRegister};
use crate::cycles::MachineCycles;
use crate::divider::DividerClocks;

#[derive(Debug, Clone)]
pub struct SquareWaveChannel {
    active: bool, // the channel has been triggered and is active
    sweep_enabled: bool, // channel 1 only
    sweep: Sweep,
    envelope: EnvelopeFunction,
    length_duty_register: LengthTimerAndDutyCycleRegister,
    period_control_register: PeriodAndControlRegisters,
    frame_sequencer: FrameSequencer,
    length_timer: LengthTimer,

    current_period: usize, // copy of the period register for this duty cycle
    frequency_timer: usize, // internal counter that overflows at current_period
    wave_duty_index: [usize; 4], // current index into the wave duty pattern (0-7) for the current duty cycle

    output: u8,
}

impl SquareWaveChannel {
    pub fn new(sweep_enabled: bool) -> Self {
        let period_control_register = PeriodAndControlRegisters::default();
        let length_duty_register = LengthTimerAndDutyCycleRegister::default();
        Self {
            active: false,
            sweep_enabled,
            sweep: Sweep::default(),
            envelope: EnvelopeFunction::default(),
            current_period: period_control_register.period() as usize,
            frame_sequencer: FrameSequencer::default(),
            frequency_timer: 0,
            wave_duty_index: [0; 4], // this is to store an index per wave duty, is this correct?
            length_timer: LengthTimer::square_channel(&length_duty_register),
            output: 0,
            length_duty_register,
            period_control_register,
        }
    }

    pub fn channel1() -> Self {
        Self::new(true)
    }

    pub fn channel2() -> Self {
        Self::new(false)
    }

    pub fn sweep_register(&self) -> &SweepRegister {
        &self.sweep.register()
    }

    pub fn sweep_register_mut(&mut self) -> &mut SweepRegister {
        self.sweep.register_mut()
    }

    pub fn length_duty_register(&self) -> &LengthTimerAndDutyCycleRegister {
        &self.length_duty_register
    }

    pub fn length_duty_register_mut(&mut self) -> &mut LengthTimerAndDutyCycleRegister {
        &mut self.length_duty_register
    }

    pub fn volume_envelope_register(&self) -> &VolumeAndEnvelopeRegister {
        &self.envelope.register()
    }

    pub fn volume_envelope_register_mut(&mut self) -> &mut VolumeAndEnvelopeRegister {
        self.envelope.register_mut()
    }

    pub fn period_control_register(&self) -> &PeriodAndControlRegisters {
        &self.period_control_register
    }

    pub fn period_control_register_mut(&mut self) -> &mut PeriodAndControlRegisters {
        &mut self.period_control_register
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn output(&self) -> u8 {
        self.output
    }

    pub fn dac_on(&self) -> bool {
        !self.envelope.dac_off()
    }

    pub fn output_f32(&self) -> f32 {
        if self.dac_on() {
            dac_sample(self.output)
        } else {
            0.0
        }
    }

    pub fn trigger(&mut self) {
        self.active = true;
        self.length_timer.reset(self.length_duty_register.initial_length_timer());

        // When triggering Ch1 and Ch2, the low two bits of the frequency timer are NOT modified.
        self.frequency_timer = self.frequency_timer & 0b00000011;
        self.current_period = if self.sweep_enabled {
            let initial_sweep = self.sweep.reset(self.current_period);
            if initial_sweep.overflows {
                self.active = false;
            }
            initial_sweep.value
        } else {
            self.period_control_register.period() as usize
        };
        self.envelope.reset();
    }

    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if self.period_control_register.consume_pending_activation() {
            self.trigger();
        }

        if !self.active {
            return
        }

        // Sweep -> Period Counter -> Duty -> Length Timer -> Envelope
        let events = self.frame_sequencer.update(div_clocks);

        if events.contains(FrameSequencerEvent::Sweep) {
            self.update_sweep();
        }

        self.update_wave_duty(delta);

        if events.contains(FrameSequencerEvent::LengthCounter) {
            self.update_length_counter();
        }

        if events.contains(FrameSequencerEvent::VolumeEnvelope) {
            self.update_volume_envelope();
        }

        let wave_duty = self.wave_duty_index[self.length_duty_register.wave_duty_cycle() as usize];
        let waveform_bit = 7 - wave_duty;
        let waveform_sample = (self.length_duty_register.waveform() >> waveform_bit) & 0x1;
        self.output = self.envelope.current_volume() * waveform_sample;
    }

    fn update_sweep(&mut self) {
        if !self.sweep_enabled {
            // channel 2 has no sweep
            return
        }

        if let Some(next_sweep) = self.sweep.step() {
            if next_sweep.overflows {
                self.active = false;
            } else {
                self.current_period = next_sweep.value;
                self.period_control_register.set_period(next_sweep.value as u16);
            }
        }
    }

    fn update_length_counter(&mut self) {
        if self.period_control_register.length_enable() && self.length_timer.step() {
            // length overflowed, disable the channel
            self.active = false;
        }
    }

    fn update_volume_envelope(&mut self) {
        self.envelope.step();
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

            let wave_duty = &mut self.wave_duty_index[self.length_duty_register.wave_duty_cycle() as usize];
            *wave_duty += 1;
            *wave_duty %= 8;

            // Period changes (written to NR13 or NR14) only take effect after the current “sample” ends
            self.current_period = self.period_control_register.period() as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_tone() {
        let mut channel = SquareWaveChannel::channel2();
        channel.length_duty_register.set(0b10000000); // 50% duty cycle
        channel.period_control_register.set_nrx3(0x00); // low byte of period
        channel.period_control_register.set_nrx4(0b00000101); // high byte of period (period = 0x500)
        channel.envelope.register_mut().set(0xF0); // initial volume 15
        channel.trigger();

        // With a period of 0x500, we should get a frequency of 1 sample per 768 machine cycles
        // at 50% duty cycle, we should get 4 samples of 0x00 followed by 4 samples of 0xFF
        let expected_waveform: [u8; 768 * 8] = std::array::from_fn(|i| {
            let phase = i / 768;
            if phase < 4 { 0x00 } else { 0xF }
        });
        let mut waveform = [0u8; 768 * 8];
        for i in 0..waveform.len() {
            channel.update(MachineCycles::from_m(1), DividerClocks::ZERO);
            waveform[i] = channel.output;
        }

        assert_eq!(waveform, expected_waveform);
    }
}