use crate::audio::dac::dac_sample;
use crate::audio::frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use crate::audio::length::{LengthTimer};
use crate::audio::sweep::Sweep;
use crate::audio::timer::PulseTimer;
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
    frequency_timer: PulseTimer, // internal counter that overflows at current_period
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
            frequency_timer: PulseTimer::default(),
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
        // the length timer is still triggered even when the dac is disabled.
        self.length_timer.trigger(frame_sequencer);

        if !self.envelope_function.dac_enabled() {
            return;
        }

        self.initialised = true;
        self.active = true;

        let initial_frequency = if let Some(sweep) = self.sweep.as_mut() {
            let initial_sweep = sweep.trigger(self.period);
            if initial_sweep.overflows {
                self.active = false;
                0
            } else {
                initial_sweep.value
            }
        } else {
            self.period
        };
        self.frequency_timer.set_frequency(initial_frequency);
        self.frequency_timer.trigger();

        self.envelope_function.trigger();
    }

    pub fn update(&mut self, delta: MachineCycles, events: FrameSequencerEvent) {
        if self.active && !self.envelope_function.dac_enabled() {
            self.active = false;
        }

        // disabled channels still clock the length counter
        if events.is_length_counter() {
            self.length_timer.clock(&mut self.active);
        }

        if !self.active {
            self.output = 0;
            return
        }

        if events.is_sweep() {
            self.update_sweep();
        }

        if events.is_volume_envelope() {
            self.envelope_function.clock();
        }

        // Obscure behavior: Just after powering on, the first duty step of the square waves after they are triggered for the first time is played as if it were 0.
        // Obscure behavior: the square duty sequence clocking is disabled until the first trigger.
        if !self.initialised {
            return;
        }

        // update wave duty
        if self.frequency_timer.update(delta) {
            self.output = if self.waveform_bit() {
                self.envelope_function.current_volume()
            } else {
                0
            };

            // Period changes (written to NR13 or NR14) only take effect after the current “sample” ends
            self.frequency_timer.set_frequency(self.period);
        }
    }

    fn waveform_bit(&self) -> bool {
        let bit = 7 - self.frequency_timer.phase();
        match self.wave_duty_cycle {
            0 => bit == 0, // 12.5% duty cycle
            1 => bit < 2, // 25% duty cycle
            2 => bit < 4, // 50% duty cycle
            3 => bit > 1, // 75% duty cycle
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
                    self.frequency_timer.set_frequency(next_sweep.value);
                    self.period = next_sweep.value & 0x07FF;
                }
            }
        }
    }
}
