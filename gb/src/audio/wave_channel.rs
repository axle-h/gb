use bincode::{Decode, Encode};
use crate::audio::dac::dac_sample;
use crate::audio::frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use crate::audio::length::{LengthTimer};
use crate::audio::timer::WavetableTimer;
use crate::cycles::MachineCycles;

#[derive(Debug, Clone, Eq, PartialEq, Decode, Encode)]
pub struct WaveChannel {
    /// NR30: DAC enable
    /// bit 7 DAC power (0=Off, 1=On)
    dac_enabled: bool,

    /// NR31 Length timer
    /// bits 0-8 Initial length timer
    initial_length_timer: u8,
    length_timer: LengthTimer,

    /// NR32 output level
    /// 2 bits (0-3) nr32
    /// 00	Mute (No sound)
    /// 01	100% volume (use samples read from Wave RAM as-is)
    /// 10	50% volume (shift samples read from Wave RAM right once)
    /// 11	25% volume (shift samples read from Wave RAM right twice)
    volume_register: u8,

    // NR33 & NR34: frequency & control
    /// 11 bits (0-2047)
    /// low 8 bits in NR33, high 3 bits in NR34
    period_register: u16,

    /// 16 bytes of wave pattern RAM (32 4-bit samples)
    wave_ram: [u8; 16],

    /// Internal state
    active: bool, // the channel has been triggered and is active
    frequency_timer: WavetableTimer, // internal counter that overflows at current_period

    sample_buffer: u8, // current output sample (0-15)
}

// From https://gbdev.gg8.se/wiki/articles/Gameboy_sound_hardware#Power_Control
const DMG_INITIAL_RAM: [u8; 16] = [
    0x84, 0x40, 0x43, 0xAA, 0x2D, 0x78, 0x92, 0x3C, 0x60, 0x59, 0x59, 0xB0, 0x34, 0xB8, 0x2E, 0xDA,
];

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            dac_enabled: false,
            initial_length_timer: 0,
            length_timer: LengthTimer::wave_channel(),
            volume_register: 0,
            period_register: 0,
            wave_ram: DMG_INITIAL_RAM,
            active: false,
            frequency_timer: WavetableTimer::default(),
            sample_buffer: 0
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        // wave ram is not touched on reset
        *self = Self { wave_ram: self.wave_ram, ..Self::default() };
    }

    pub fn nr30(&self) -> u8 {
        // Bit 7: DAC power (0=Off, 1=On)
        // Bits 0-6: Read as 1
        if self.dac_enabled {
            0xFF
        } else {
            0x7F
        }
    }

    pub fn set_nr30(&mut self, value: u8) {
        self.dac_enabled = value & 0x80 != 0;
        if !self.dac_enabled {
            self.active = false;
        }
    }

    pub fn nr31_length_timer(&self) -> u8 {
        0xFF // write only
    }

    pub fn set_nr31_length_timer(&mut self, value: u8) {
        self.initial_length_timer = value;

        // the length timer can be reset at any time
        self.length_timer.reset(self.initial_length_timer);
    }

    pub fn nr32_output_level(&self) -> u8 {
        // Bits 0-4: Read as 1
        // Bits 5-6: Volume code
        // Bit 7: Read as 1
        0x9F | ((self.volume_register & 0b11) << 5)
    }

    pub fn set_nr32_output_level(&mut self, value: u8) {
        self.volume_register = (value >> 5) & 0b11;
    }

    pub fn nr33_period_low(&self) -> u8 {
        0xFF // nr33 is write-only
    }

    pub fn set_nr33_period_low(&mut self, value: u8) {
        self.period_register = (self.period_register & 0xFF00) | value as u16;
        self.reload_period();
    }

    /// Hand the new period to the frequency timer **without disturbing the interval in flight**.
    /// Hardware reloads the timer from NR33/NR34 as they read at each overflow, so a write part-way
    /// through a note takes effect at the next reload, not immediately — gambatte gets this by
    /// recomputing `toPeriod(nr3_, nr4_)` on every catch-up (`channel3.cpp:104`). gb used to latch
    /// the period at trigger time and never look again, which `09-wave read while on` depends on:
    /// it triggers at a long period and then drops NR33 to `0xFE` before reading.
    fn reload_period(&mut self) {
        self.frequency_timer.set_frequency(self.period_register);
    }

    pub fn nr34_period_high_and_control(&self) -> u8 {
        // Bits 0-5 & 7 are always 1 when read
        0xBF | if self.length_timer.enabled() { 0b01000000 } else { 0 }
    }

    pub fn set_nr34_period_high_and_control(
        &mut self,
        value: u8,
        frame_sequencer: &FrameSequencer,
        access_offset: u16,
    ) {
        self.period_register = (self.period_register & 0x00FF) | (((value & 0b111) as u16) << 8);
        self.reload_period();
        let length_enabled = value & 0b01000000 != 0;
        self.length_timer.set_enabled(length_enabled, frame_sequencer, &mut self.active);
        if value & 0b10000000 != 0 {
            self.trigger(frame_sequencer, access_offset);
        }
    }

    /// Read wave RAM. `access_offset` places the CPU's bus access within the instruction that is
    /// executing — see [`crate::audio::Audio::set_access_offset`].
    ///
    /// While the channel is playing, the CPU does not get the byte it asked for. On DMG the bus is
    /// only connected for the single tick in which the channel fetches its next sample: hit that
    /// tick and you read the byte the channel just fetched, miss it and you read `0xFF`
    /// (gambatte `channel3.h:47-56`). CGB widens that window; that is a B-phase concern.
    pub fn wave_ram(&self, index: usize, access_offset: u16) -> u8 {
        if !self.active {
            return self.wave_ram[index];
        }
        match self.fetch_at(access_offset) {
            Some(fetch) => self.wave_ram[self.fetch_index(fetch)],
            None => 0xFF,
        }
    }

    /// Write wave RAM, under the same aperture as [`WaveChannel::wave_ram`]: while the channel is
    /// playing, a write outside the fetch tick is **dropped**, and one inside it lands on the byte
    /// being fetched rather than the addressed one.
    pub fn set_wave_ram(&mut self, index: usize, value: u8, access_offset: u16) {
        if !self.active {
            self.wave_ram[index] = value;
            return;
        }
        if let Some(fetch) = self.fetch_at(access_offset) {
            let index = self.fetch_index(fetch);
            self.wave_ram[index] = value;
        }
    }

    /// Does a sample fetch land **exactly** on the tick `offset` ticks into the instruction now
    /// executing? If so, which upcoming fetch is it (1 = the next one)?
    ///
    /// The channel fetches every `period` ticks starting `counter` ticks from the instruction
    /// boundary, and at short periods several of those fall inside a single instruction — the
    /// blargg tests drive it at `period == 2`, one fetch per M-cycle — so this cannot just compare
    /// against the next fetch.
    fn fetch_at(&self, offset: u16) -> Option<u16> {
        let counter = self.frequency_timer.counter();
        let period = self.frequency_timer.period();
        let elapsed = offset.checked_sub(counter)?;
        (elapsed % period == 0).then(|| elapsed / period + 1)
    }

    /// How many sample fetches have happened by the tick `offset` ticks into the instruction now
    /// executing, counting one landing exactly on it.
    fn fetches_by(&self, offset: u16) -> u16 {
        match offset.checked_sub(self.frequency_timer.counter()) {
            Some(elapsed) => elapsed / self.frequency_timer.period() + 1,
            None => 0,
        }
    }

    /// Tick of the first fetch strictly after `offset`, measured from the instruction boundary.
    fn next_fetch_after(&self, offset: u16) -> u16 {
        self.frequency_timer.counter() + self.fetches_by(offset) * self.frequency_timer.period()
    }

    /// Wave-RAM byte the `n`th upcoming fetch reads. A fetch advances the phase and then reads, so
    /// `n = 1` is one step ahead of [`WaveChannel::current_sample_byte`].
    fn fetch_index(&self, n: u16) -> usize {
        (((self.frequency_timer.phase() as u16 + n) & 31) >> 1) as usize
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn dac_enabled(&self) -> bool {
        self.dac_enabled
    }

    pub fn output_f32(&self) -> f32 {
        match self.digital_level() {
            None => 0.0,
            Some(level) => dac_sample(level),
        }
    }

    /// See [`crate::audio::square_channel::SquareWaveChannel::digital_level`]. Note the **phase
    /// parity** in here: a wave-RAM byte holds two nibbles, so advancing the phase changes the
    /// output even when `sample_buffer` has not moved.
    #[inline]
    pub fn digital_level(&self) -> Option<u8> {
        if !self.dac_enabled || self.volume_register == 0 {
            return None;
        }
        let sample_byte = self.sample_buffer >> (self.volume_register - 1);
        Some(if self.frequency_timer.phase() & 0x1 == 0 {
            sample_byte >> 4
        } else {
            sample_byte & 0xF
        })
    }

    pub fn trigger(&mut self, frame_sequencer: &FrameSequencer, access_offset: u16) {
        // DMG wave-RAM corruption: retriggering one tick before a sample fetch copies the byte
        // that fetch was about to read down over the start of wave RAM (gambatte
        // `channel3.cpp:60-68`). CGB does not do this.
        if self.active && self.dac_enabled
            && self.next_fetch_after(access_offset) == access_offset + 1 {
            // The byte that imminent fetch was about to read, at the phase the channel has
            // reached by the time of the write.
            let position = self.fetch_index(self.fetches_by(access_offset) + 1);
            if position < 4 {
                self.wave_ram[0] = self.wave_ram[position];
            } else {
                let aligned = position & !3;
                self.wave_ram.copy_within(aligned..aligned + 4, 0);
            }
        }

        self.active = self.dac_enabled;
        self.length_timer.trigger(frame_sequencer);
        self.frequency_timer.set_frequency(self.period_register);
        // The first fetch is `period + 3` ticks out, and the write's own placement within its
        // instruction shifts it: the timer counts from the instruction boundary, the write happens
        // `access_offset` ticks into it.
        self.frequency_timer.trigger_after(3 + access_offset);
    }

    pub fn update(&mut self, delta: MachineCycles, events: FrameSequencerEvent) {
        if self.active && !self.dac_enabled() {
            self.active = false;
        }

        if !self.active {
            self.sample_buffer = 0;

            // disabled channels still clock the length counter
            if events.is_length_counter() {
                self.clock_length_timer();
            }
            return;
        }

        if events.is_length_counter() {
            self.clock_length_timer();
        }

        if self.active && self.frequency_timer.update(delta) {
            // overflow, emit a sample
            self.sample_buffer = self.current_sample_byte();
        }
    }

    /// M-cycles until this channel's output can next move on its own. See
    /// [`crate::audio::Audio::next_event`].
    pub fn next_event(&self) -> Option<u64> {
        if !self.active {
            return None;
        }
        Some(self.frequency_timer.machine_cycles_to_next_phase())
    }

    fn current_sample_byte(&self) -> u8 {
        self.wave_ram[(self.frequency_timer.phase() >> 1) as usize]
    }

    fn clock_length_timer(&mut self) {
        let prev_active = self.active;
        self.length_timer.clock(&mut self.active);
        if prev_active && !self.active {
            // Explicitly clear the sample buffer when the length counter disables the channel.
            // Necessary because the wavetable channel continues to output the current sample buffer
            // when disabled, as long as the DAC is still enabled
            self.sample_buffer = 0;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A channel playing a `period`-tick note, triggered by a write placed `access_offset` ticks
    /// into its instruction, with wave RAM holding `00 11 22 ... FF`.
    fn playing(period: u16, access_offset: u16) -> WaveChannel {
        let mut channel = WaveChannel::default();
        for index in 0..16 {
            channel.set_wave_ram(index, (index as u8) * 0x11, 0);
        }
        channel.set_nr30(0x80); // DAC on
        channel.set_nr32_output_level(0x20); // full volume, so the channel really runs

        let frequency = 2048 - period;
        channel.set_nr33_period_low(frequency as u8);
        channel.set_nr34_period_high_and_control(
            0x80 | ((frequency >> 8) as u8 & 0b111),
            &FrameSequencer::default(),
            access_offset,
        );
        assert!(channel.is_active());
        channel
    }

    fn advance(channel: &mut WaveChannel, m_cycles: u64) {
        channel.update(MachineCycles::from_m(m_cycles), FrameSequencerEvent::empty());
    }

    /// The first fetch after a trigger is `period + 3` ticks out, counted from the *write*, which
    /// itself sits `access_offset` ticks into its instruction.
    #[test]
    fn trigger_delays_the_first_fetch_by_three_ticks() {
        let channel = playing(100, 4);
        assert_eq!(channel.frequency_timer.counter(), 100 + 3 + 4);
    }

    /// A16 / dmg_sound 09. While the channel plays, a read only sees wave RAM on the exact tick
    /// the channel fetches its next sample; every other tick reads `0xFF`.
    #[test]
    fn a_read_only_lands_on_the_fetch_tick() {
        let channel = playing(2, 0); // counter = 5
        assert_eq!(channel.wave_ram(0, 4), 0xFF, "one tick early");
        assert_eq!(channel.wave_ram(0, 6), 0xFF, "between fetches");
        // Fetch 1 advances the phase 0 -> 1, so it reads byte 0; fetch 2 reads byte 1.
        assert_eq!(channel.wave_ram(0, 5), 0x00, "on the first fetch");
        assert_eq!(channel.wave_ram(9, 7), 0x11, "on the second fetch — and not byte 9");
    }

    /// With the channel off, wave RAM is ordinary memory again.
    #[test]
    fn a_read_with_the_channel_off_returns_the_addressed_byte() {
        let mut channel = playing(2, 0);
        channel.set_nr30(0x00); // DAC off disables the channel
        assert_eq!(channel.wave_ram(9, 4), 0x99);
    }

    /// A16 / dmg_sound 12. Same aperture for writes: outside it the write is lost, inside it the
    /// write lands on the byte being fetched rather than the addressed one.
    #[test]
    fn a_write_only_lands_on_the_fetch_tick() {
        let mut channel = playing(2, 0); // counter = 5

        channel.set_wave_ram(9, 0xAB, 4);
        assert_eq!(channel.wave_ram(9, 5), 0x00, "dropped, so byte 0 is still 0x00");

        channel.set_wave_ram(9, 0xAB, 5);
        assert_eq!(channel.wave_ram(9, 5), 0xAB, "landed on byte 0, not byte 9");
    }

    /// A16 / dmg_sound 09's real prerequisite. Hardware reloads the frequency timer from
    /// NR33/NR34 at each overflow, so a period written mid-note takes effect at the **next**
    /// reload — not immediately, and not only at the next trigger. gb used to latch the period at
    /// trigger time, which left the channel fetching at the old rate forever.
    #[test]
    fn a_period_written_mid_note_applies_at_the_next_reload() {
        let mut channel = playing(100, 0);
        assert_eq!(channel.frequency_timer.counter(), 103);

        channel.set_nr33_period_low(0xFE); // period 2048 - 0x7FE = 2
        assert_eq!(channel.frequency_timer.counter(), 103, "the interval in flight is untouched");

        // 103 ticks reach the first fetch, which reloads; the 104th spends one tick of the new
        // period. With the period latched at trigger the reload would have been 100, not 2.
        advance(&mut channel, 52);
        assert_eq!(channel.frequency_timer.counter(), 1);
        assert_eq!(channel.wave_ram(0, 1), 0x11, "fetch 2 reads byte 1");
    }

    /// A16 / dmg_sound 10. Retriggering exactly one tick before a fetch copies the byte that fetch
    /// was about to read down over the start of wave RAM. DMG only.
    #[test]
    fn a_retrigger_one_tick_before_a_fetch_corrupts_wave_ram() {
        // Line the channel up so the next fetch is at tick 5 and would read byte 6.
        let mut channel = playing(2, 0);
        advance(&mut channel, 6); // 12 ticks: fetches at 5, 7, 9, 11 -> phase 4, counter 1
        assert_eq!(channel.frequency_timer.phase(), 4);
        assert_eq!(channel.frequency_timer.counter(), 1);

        // A trigger written at offset 0, with the fetch one tick later, corrupts.
        channel.set_nr34_period_high_and_control(0x80, &FrameSequencer::default(), 0);

        // The imminent fetch would have read byte (4 + 1) / 2 = 2, which is below 4, so byte 0
        // takes byte 2's value.
        channel.set_nr30(0x00);
        assert_eq!(channel.wave_ram(0, 0), 0x22);
        assert_eq!(channel.wave_ram(2, 0), 0x22, "the source byte is unchanged");
    }

    /// The same retrigger a tick out of alignment leaves wave RAM alone.
    #[test]
    fn a_retrigger_anywhere_else_leaves_wave_ram_alone() {
        let mut channel = playing(2, 0);
        advance(&mut channel, 6);
        assert_eq!(channel.frequency_timer.counter(), 1);

        // Offset 1 puts the write *on* the fetch rather than one tick before it.
        channel.set_nr34_period_high_and_control(0x80, &FrameSequencer::default(), 1);

        channel.set_nr30(0x00);
        assert_eq!(channel.wave_ram(0, 0), 0x00);
    }
}
