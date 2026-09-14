//! The four voices. Each one is its DMG channel's own state machine — duty position, envelope,
//! sweep shadow, wave position, LFSR — carrying its next transition as a countdown in the
//! renderer's sub-units, so [`super::Synth`] can integrate a constant level between transitions
//! instead of sampling one.
//!
//! A voice answers [`Voice::level`] with the 4-bit number its DAC is converting, or `None` when
//! the DAC is off and the channel is out of the mixer entirely. Nothing here knows about sample
//! rates, panning or volume.

use super::SUBS_PER_MCYCLE;

/// `NRx1` bits 6-7: bit `n` of the row is the level at duty position `n`, so the rows are 12.5%,
/// 25%, 50% and 75% high.
const DUTY: [u8; 4] = [0b1000_0000, 0b1000_0001, 0b1110_0001, 0b0111_1110];

/// The period registers count *down* from 2048, in the channel's own ticks.
const PERIOD_SPAN: u64 = 2048;

/// The wave channel steps twice as fast as the pulse channels: one position per half M-cycle.
const SUBS_PER_WAVE_TICK: u64 = SUBS_PER_MCYCLE / 2;

/// The volume a channel's DAC is converting, or `None` when the DAC is off.
pub type Level = Option<u8>;

/// `NRx2`, and the 64 Hz counter that steps it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Envelope {
    initial: u8,
    increase: bool,
    pace: u8,
    volume: u8,
    counter: u8,
}

impl Envelope {
    pub fn write(&mut self, volume: u8, increase: bool, pace: u8) {
        self.initial = volume;
        self.increase = increase;
        self.pace = pace;
    }

    /// The DAC is powered by the top five bits of `NRx2` alone: an initial volume of zero counting
    /// down is a channel that is not merely silent but disconnected.
    pub fn dac_on(&self) -> bool {
        self.initial != 0 || self.increase
    }

    pub fn trigger(&mut self) {
        self.volume = self.initial;
        self.counter = self.reload();
    }

    /// A pace of zero means the envelope never steps, but the counter still reloads with eight.
    fn reload(&self) -> u8 {
        if self.pace == 0 { 8 } else { self.pace }
    }

    pub fn clock(&mut self) {
        if self.pace == 0 {
            return;
        }
        self.counter = self.counter.saturating_sub(1);
        if self.counter != 0 {
            return;
        }
        self.counter = self.reload();
        if self.increase {
            self.volume = (self.volume + 1).min(15);
        } else {
            self.volume = self.volume.saturating_sub(1);
        }
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }
}

/// `NR10` and the shadow period it steps, clocked at 128 Hz.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sweep {
    pace: u8,
    decrease: bool,
    step: u8,
    enabled: bool,
    shadow: u16,
    timer: u8,
    /// Whether a period has been calculated downwards since the last trigger. Clearing the
    /// direction after one has disables the channel there and then.
    decreased_since_trigger: bool,
}

impl Sweep {
    fn reload(&mut self) {
        self.timer = if self.pace == 0 { 8 } else { self.pace };
    }

    /// Returns whether the channel is to be switched off.
    pub fn write(&mut self, pace: u8, decrease: bool, step: u8) -> bool {
        self.pace = pace;
        self.decrease = decrease;
        self.step = step;
        self.decreased_since_trigger && !decrease
    }

    /// Returns whether the overflow check failed, which switches the channel off at once. The
    /// calculated period is *discarded*: the check is all a trigger does with it, so the note
    /// sounds at the period that was written until the first 128 Hz iteration moves it.
    pub fn trigger(&mut self, period: u16) -> bool {
        self.decreased_since_trigger = false;
        self.shadow = period;
        self.reload();
        self.enabled = self.pace != 0 || self.step != 0;
        self.step > 0 && self.calculate().1
    }

    /// The next period and whether it overflowed eleven bits.
    fn calculate(&mut self) -> (u16, bool) {
        let delta = self.shadow >> self.step;
        let next = if self.decrease {
            self.decreased_since_trigger = true;
            self.shadow.wrapping_sub(delta)
        } else {
            self.shadow.wrapping_add(delta)
        };
        let overflow = next > 0x7FF;
        if overflow {
            self.enabled = false;
        }
        (next, overflow)
    }

    /// One 128 Hz iteration: the new period to play, or `None` when this iteration changes
    /// nothing. `overflow` switches the channel off.
    pub fn clock(&mut self) -> Option<(u16, bool)> {
        self.timer = self.timer.saturating_sub(1);
        if self.timer != 0 {
            return None;
        }
        self.reload();
        if !self.enabled || self.pace == 0 {
            return None;
        }
        let (next, overflow) = self.calculate();
        if overflow || self.step == 0 {
            return Some((next, overflow));
        }
        self.shadow = next;
        // The second overflow check, which the hardware performs on the value it has just stored
        // and then throws away.
        let (_, overflow) = self.calculate();
        Some((next, overflow))
    }
}

/// `NRx1`'s length counter, clocked at 256 Hz. Pokémon Red never enables one on a music channel
/// and enables one twice in the whole cartridge, but a channel that is switched off by its length
/// is switched off for good, so it cannot be left out.
#[derive(Debug, Clone, Copy)]
pub struct Length {
    enabled: bool,
    value: u16,
    span: u16,
}

impl Length {
    fn new(span: u16) -> Self {
        Self { enabled: false, value: span, span }
    }

    pub fn reload(&mut self, initial: u16) {
        self.value = self.span.saturating_sub(initial);
    }

    /// `on_length_step` is whether the sequencer's current step is one of the four that clock
    /// lengths: enabling a counter there costs it a tick at once.
    pub fn set_enabled(&mut self, enabled: bool, on_length_step: bool, active: &mut bool) {
        let was = self.enabled;
        self.enabled = enabled;
        if !was && enabled && on_length_step {
            self.clock(active);
        }
    }

    pub fn trigger(&mut self, on_length_step: bool) {
        if self.value == 0 {
            self.reload(0);
            if self.enabled && on_length_step {
                self.value -= 1;
            }
        }
    }

    pub fn clock(&mut self, active: &mut bool) {
        if !self.enabled {
            return;
        }
        self.value = self.value.saturating_sub(1);
        if self.value == 0 {
            *active = false;
        }
    }
}

/// One of the two pulse channels: `NR10`-`NR14` and `NR21`-`NR24`. Channel 2 has no sweep.
#[derive(Debug, Clone, Copy)]
pub struct Pulse {
    duty: u8,
    period: u16,
    position: u8,
    countdown: u64,
    envelope: Envelope,
    sweep: Option<Sweep>,
    length: Length,
    active: bool,
}

impl Pulse {
    pub fn new(sweep: bool) -> Self {
        Self {
            duty: 0,
            period: 0,
            position: 0,
            countdown: PERIOD_SPAN * SUBS_PER_MCYCLE,
            envelope: Envelope::default(),
            sweep: sweep.then(Sweep::default),
            length: Length::new(64),
            active: false,
        }
    }

    fn step_subs(&self) -> u64 {
        (PERIOD_SPAN - self.period as u64) * SUBS_PER_MCYCLE
    }

    pub fn set_duty(&mut self, duty: u8) {
        self.duty = duty & 3;
    }

    pub fn set_length(&mut self, length: u8) {
        self.length.reload(length as u16 & 0x3F);
    }

    pub fn set_envelope(&mut self, volume: u8, increase: bool, pace: u8) {
        self.envelope.write(volume, increase, pace);
        if !self.envelope.dac_on() {
            self.active = false;
        }
    }

    pub fn set_sweep(&mut self, pace: u8, decrease: bool, step: u8) {
        if let Some(sweep) = self.sweep.as_mut()
            && sweep.write(pace, decrease, step)
        {
            self.active = false;
        }
    }

    pub fn set_period_low(&mut self, low: u8) {
        self.period = self.period & 0x700 | low as u16;
    }

    pub fn set_period_high(&mut self, high: u8, trigger: bool, length_enable: bool, on_length_step: bool) {
        self.period = self.period & 0xFF | (high as u16 & 7) << 8;
        self.length.set_enabled(length_enable, on_length_step, &mut self.active);
        if trigger {
            self.trigger(on_length_step);
        }
    }

    fn trigger(&mut self, on_length_step: bool) {
        self.length.trigger(on_length_step);
        if !self.envelope.dac_on() {
            return;
        }
        self.active = true;
        if let Some(sweep) = self.sweep.as_mut()
            && sweep.trigger(self.period)
        {
            self.active = false;
        }
        // The duty position is reset here rather than left where it was: see the note on
        // `Synth`'s spec about what the comparison against the emulator's APU needs.
        self.position = 0;
        self.countdown = self.step_subs();
        self.envelope.trigger();
    }

    pub fn clock_length(&mut self) {
        self.length.clock(&mut self.active);
    }

    pub fn clock_envelope(&mut self) {
        self.envelope.clock();
    }

    pub fn clock_sweep(&mut self) {
        let Some(sweep) = self.sweep.as_mut() else { return };
        match sweep.clock() {
            Some((_, true)) => self.active = false,
            Some((period, false)) => self.period = period & 0x7FF,
            None => {}
        }
    }

    pub fn level(&self) -> Level {
        if !self.envelope.dac_on() {
            return None;
        }
        let high = DUTY[self.duty as usize] & 1 << self.position != 0;
        Some(if self.active && high { self.envelope.volume() } else { 0 })
    }

    pub fn subs_to_next(&self) -> Option<u64> {
        self.active.then_some(self.countdown)
    }

    pub fn advance(&mut self, mut subs: u64) {
        if !self.active {
            return;
        }
        while subs >= self.countdown {
            subs -= self.countdown;
            self.position = self.position + 1 & 7;
            // A period written mid-note only takes effect when the position next advances.
            self.countdown = self.step_subs();
        }
        self.countdown -= subs;
    }
}

/// The wave channel, `NR30`-`NR34` and the sixteen bytes of wave RAM: 32 4-bit samples, one per
/// position, played high nibble first.
#[derive(Debug, Clone, Copy)]
pub struct Wave {
    dac: bool,
    /// `NR32`: 0 silences the digital output, 1 plays it whole, 2 halves it, 3 quarters it.
    level: u8,
    period: u16,
    ram: [u8; 16],
    position: u8,
    countdown: u64,
    /// The nibble last fetched. A trigger does not refill it, so the position it lands on is heard
    /// one period late and position 0 is not heard at all until the pattern wraps.
    sample: u8,
    length: Length,
    active: bool,
}

impl Wave {
    /// What a DMG leaves in wave RAM at power-on, which is what the cartridge would find there
    /// before its first `AudioN_ApplyWavePattern`.
    const POWER_ON_RAM: [u8; 16] = [
        0x84, 0x40, 0x43, 0xAA, 0x2D, 0x78, 0x92, 0x3C, 0x60, 0x59, 0x59, 0xB0, 0x34, 0xB8, 0x2E, 0xDA,
    ];

    pub fn new() -> Self {
        Self {
            dac: false,
            level: 0,
            period: 0,
            ram: Self::POWER_ON_RAM,
            position: 0,
            countdown: PERIOD_SPAN * SUBS_PER_WAVE_TICK,
            sample: 0,
            length: Length::new(256),
            active: false,
        }
    }

    fn step_subs(&self) -> u64 {
        (PERIOD_SPAN - self.period as u64) * SUBS_PER_WAVE_TICK
    }

    pub fn set_dac(&mut self, on: bool) {
        self.dac = on;
        if !on {
            self.active = false;
            self.sample = 0;
        }
    }

    pub fn set_level(&mut self, level: u8) {
        self.level = level & 3;
    }

    pub fn set_length(&mut self, length: u8) {
        self.length.reload(length as u16);
    }

    pub fn set_period_low(&mut self, low: u8) {
        self.period = self.period & 0x700 | low as u16;
    }

    pub fn set_period_high(&mut self, high: u8, trigger: bool, length_enable: bool, on_length_step: bool) {
        self.period = self.period & 0xFF | (high as u16 & 7) << 8;
        self.length.set_enabled(length_enable, on_length_step, &mut self.active);
        if trigger {
            self.trigger(on_length_step);
        }
    }

    fn trigger(&mut self, on_length_step: bool) {
        self.length.trigger(on_length_step);
        self.active = self.dac;
        self.position = 0;
        // A DMG's first fetch after a trigger comes three ticks late.
        self.countdown = self.step_subs() + 3 * SUBS_PER_WAVE_TICK;
    }

    pub fn write_ram(&mut self, index: u8, byte: u8) {
        self.ram[index as usize & 0xF] = byte;
    }

    /// `NR52` cleared: everything but wave RAM, which survives the power going off.
    pub fn power_off(&mut self) {
        *self = Self { ram: self.ram, ..Self::new() };
    }

    pub fn clock_length(&mut self) {
        let was = self.active;
        self.length.clock(&mut self.active);
        if was && !self.active {
            self.sample = 0;
        }
    }

    pub fn level_out(&self) -> Level {
        if !self.dac {
            return None;
        }
        // Level 0 is silence, not a disconnected DAC: the digital output is zero and the DAC
        // converts zero.
        Some(if self.level == 0 { 0 } else { self.sample >> (self.level - 1) })
    }

    pub fn subs_to_next(&self) -> Option<u64> {
        self.active.then_some(self.countdown)
    }

    pub fn advance(&mut self, mut subs: u64) {
        if !self.active {
            return;
        }
        while subs >= self.countdown {
            subs -= self.countdown;
            self.position = self.position + 1 & 31;
            let byte = self.ram[(self.position >> 1) as usize];
            self.sample = if self.position & 1 == 0 { byte >> 4 } else { byte & 0xF };
            self.countdown = self.step_subs();
        }
        self.countdown -= subs;
    }
}

/// The noise channel, `NR41`-`NR44`: a shift register clocked by `NR43`'s divisor and shift, in
/// either width. Pokémon Red's drums use both.
#[derive(Debug, Clone, Copy)]
pub struct Noise {
    shift: u8,
    short: bool,
    divisor: u8,
    lfsr: u16,
    countdown: u64,
    envelope: Envelope,
    length: Length,
    active: bool,
}

impl Noise {
    pub fn new() -> Self {
        Self {
            shift: 0,
            short: false,
            divisor: 0,
            lfsr: 0,
            countdown: SUBS_PER_MCYCLE,
            envelope: Envelope::default(),
            length: Length::new(64),
            active: false,
        }
    }

    /// A divisor code of 0 is half of 1, and the shift doubles it each time. The result is in
    /// M-cycles, which is a quarter of the T-cycles the divisor table is written in.
    fn step_subs(&self) -> u64 {
        let divisor = if self.divisor == 0 { 8 } else { 16 * self.divisor as u64 };
        (divisor << self.shift) * SUBS_PER_MCYCLE / 4
    }

    pub fn set_length(&mut self, length: u8) {
        self.length.reload(length as u16 & 0x3F);
    }

    pub fn set_envelope(&mut self, volume: u8, increase: bool, pace: u8) {
        self.envelope.write(volume, increase, pace);
        if !self.envelope.dac_on() {
            self.active = false;
        }
    }

    pub fn set_noise(&mut self, shift: u8, short: bool, divisor: u8) {
        self.shift = shift;
        self.short = short;
        self.divisor = divisor;
    }

    pub fn set_control(&mut self, trigger: bool, length_enable: bool, on_length_step: bool) {
        self.length.set_enabled(length_enable, on_length_step, &mut self.active);
        if trigger {
            self.trigger(on_length_step);
        }
    }

    fn trigger(&mut self, on_length_step: bool) {
        self.length.trigger(on_length_step);
        self.envelope.trigger();
        self.lfsr = 0x7FFF;
        self.countdown = self.step_subs();
        self.active = self.envelope.dac_on();
    }

    pub fn clock_length(&mut self) {
        self.length.clock(&mut self.active);
    }

    pub fn clock_envelope(&mut self) {
        self.envelope.clock();
    }

    pub fn level(&self) -> Level {
        if !self.envelope.dac_on() {
            return None;
        }
        let high = self.lfsr & 1 == 0;
        Some(if self.active && high { self.envelope.volume() } else { 0 })
    }

    /// Shifts 14 and 15 are not used by the hardware and stop the register dead.
    fn clocking(&self) -> bool {
        self.active && self.shift < 14
    }

    pub fn subs_to_next(&self) -> Option<u64> {
        self.clocking().then_some(self.countdown)
    }

    pub fn advance(&mut self, mut subs: u64) {
        if !self.clocking() {
            return;
        }
        while subs >= self.countdown {
            subs -= self.countdown;
            self.step_lfsr();
            self.countdown = self.step_subs();
        }
        self.countdown -= subs;
    }

    fn step_lfsr(&mut self) {
        let feedback = (self.lfsr ^ self.lfsr >> 1) & 1;
        self.lfsr = self.lfsr >> 1 | feedback << 14;
        if self.short {
            // The narrow width feeds the same bit back at 6 as well, which is the whole of it: the
            // register is still fifteen bits wide and still shifted the same way.
            self.lfsr = self.lfsr & !(1 << 6) | feedback << 6;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four duty rows are the widths their percentages name.
    #[test]
    fn a_duty_row_is_high_for_its_share_of_the_period() {
        let high = |duty: usize| (0..8).filter(|n| DUTY[duty] & 1 << n != 0).count();
        assert_eq!([high(0), high(1), high(2), high(3)], [1, 2, 4, 6]);
    }

    fn noise(short: bool) -> Noise {
        let mut noise = Noise::new();
        noise.set_envelope(15, false, 0);
        noise.set_noise(0, short, 0);
        noise.set_control(true, false, false);
        noise
    }

    /// The wide register runs through every state but all-zero before it repeats.
    #[test]
    fn the_fifteen_bit_register_repeats_after_32767_shifts() {
        let mut noise = noise(false);
        let start = noise.lfsr;
        let period = (1..=0x10000).find(|_| {
            noise.step_lfsr();
            noise.lfsr == start
        });
        assert_eq!(period, Some(32767));
    }

    /// The narrow one is a seven-bit register living in the same fifteen bits.
    #[test]
    fn the_seven_bit_register_repeats_after_127_shifts() {
        let mut noise = noise(true);
        let start = noise.lfsr & 0x7F;
        let period = (1..=0x10000).find(|_| {
            noise.step_lfsr();
            noise.lfsr & 0x7F == start
        });
        assert_eq!(period, Some(127));
    }

    /// One shift every two M-cycles at the top of the range, and the divisor and shift each
    /// double it.
    #[test]
    fn the_noise_period_is_the_divisor_shifted() {
        let mut noise = Noise::new();
        noise.set_noise(0, false, 0);
        assert_eq!(noise.step_subs(), 2 * SUBS_PER_MCYCLE);
        noise.set_noise(0, false, 1);
        assert_eq!(noise.step_subs(), 4 * SUBS_PER_MCYCLE);
        noise.set_noise(1, false, 1);
        assert_eq!(noise.step_subs(), 8 * SUBS_PER_MCYCLE);
    }

    /// A pace of zero never steps the volume; every other pace steps it that many 64 Hz ticks
    /// apart, and it stops at the ends rather than wrapping.
    #[test]
    fn the_envelope_steps_once_every_pace_ticks() {
        let mut envelope = Envelope::default();
        envelope.write(15, false, 0);
        envelope.trigger();
        for _ in 0..64 {
            envelope.clock();
        }
        assert_eq!(envelope.volume(), 15, "a pace of zero holds");

        envelope.write(2, true, 3);
        envelope.trigger();
        for _ in 0..2 {
            envelope.clock();
        }
        assert_eq!(envelope.volume(), 2, "not yet");
        envelope.clock();
        assert_eq!(envelope.volume(), 3);
        for _ in 0..60 {
            envelope.clock();
        }
        assert_eq!(envelope.volume(), 15, "and it stops at the top");
    }

    /// A trigger checks the shifted period for overflow and then throws it away; the first
    /// iteration is what moves the note.
    #[test]
    fn a_trigger_checks_the_swept_period_without_playing_it() {
        let mut sweep = Sweep::default();
        sweep.write(1, false, 3);
        assert!(!sweep.trigger(0x100), "0x100 + 0x20 fits");
        assert_eq!(sweep.shadow, 0x100, "the shadow is what was written, not what was calculated");
        assert_eq!(sweep.clock(), Some((0x120, false)), "and the first iteration is what moves it");

        let mut sweep = Sweep::default();
        sweep.write(1, false, 1);
        assert!(sweep.trigger(0x600), "0x600 + 0x300 does not fit");
    }

    /// The iteration whose *own* successor overflows is still played: the second check is made on
    /// the period that has just been stored, and it is that one which switches the channel off.
    #[test]
    fn the_second_overflow_check_ends_the_note_a_step_late() {
        let mut sweep = Sweep::default();
        sweep.write(1, false, 1);
        assert!(!sweep.trigger(0x400), "0x400 + 0x200 fits");
        assert_eq!(sweep.clock(), Some((0x600, true)), "0x600 plays, and 0x900 would not fit");
    }

    /// A length counter runs out `64 - n` ticks after `n` is written, and takes the channel with
    /// it. Pokémon Red enables one twice in the whole cartridge, so nothing else here would catch
    /// it being wrong.
    #[test]
    fn a_length_counter_switches_its_channel_off_when_it_runs_out() {
        let mut pulse = Pulse::new(false);
        pulse.set_envelope(15, false, 0);
        pulse.set_length(62);
        pulse.set_period_high(0, true, true, false);
        assert!(pulse.active, "the note never started");
        pulse.clock_length();
        assert!(pulse.active, "gone a tick early");
        pulse.clock_length();
        assert!(!pulse.active, "still playing after its length");
    }

    /// Without the enable it counts nothing, which is how every other sound in the cartridge is
    /// played.
    #[test]
    fn a_length_counter_that_was_never_enabled_switches_nothing_off() {
        let mut pulse = Pulse::new(false);
        pulse.set_envelope(15, false, 0);
        pulse.set_length(63);
        pulse.set_period_high(0, true, false, false);
        for _ in 0..300 {
            pulse.clock_length();
        }
        assert!(pulse.active, "a note with no length enabled was cut off");
    }

    /// Triggering a channel whose length has run out gives it the whole of one back.
    #[test]
    fn triggering_an_expired_length_reloads_it() {
        let mut pulse = Pulse::new(false);
        pulse.set_envelope(15, false, 0);
        pulse.set_length(63);
        pulse.set_period_high(0, true, true, false);
        pulse.clock_length();
        assert!(!pulse.active);
        pulse.set_period_high(0, true, true, false);
        assert!(pulse.active, "a retrigger did not give it a length back");
        for _ in 0..63 {
            pulse.clock_length();
        }
        assert!(pulse.active, "the reload was not the full 64");
        pulse.clock_length();
        assert!(!pulse.active);
    }

    /// Clearing the direction after a period has been calculated downwards switches the channel
    /// off where it stands.
    #[test]
    fn dropping_the_decrease_after_a_decrease_switches_the_channel_off() {
        let mut sweep = Sweep::default();
        sweep.write(1, true, 1);
        sweep.trigger(0x400);
        assert!(!sweep.write(1, true, 1), "still decreasing");
        assert!(sweep.write(1, false, 1), "and now it is not");
    }
}
