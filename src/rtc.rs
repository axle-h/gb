//! **D5.** The MBC3 real-time clock.
//!
//! Two `0x0F`/`0x10` cartridge types carry a clock chip with its own crystal and battery, which
//! keeps counting while the console is off. Its five registers replace cartridge RAM at
//! `0xA000..=0xBFFF` when `0x08..=0x0C` is written to the bank register:
//!
//! | Bank | Register |
//! |---|---|
//! | `0x08` | seconds, 0-59 |
//! | `0x09` | minutes, 0-59 |
//! | `0x0A` | hours, 0-23 |
//! | `0x0B` | day counter, low 8 bits |
//! | `0x0C` | bit 0 = day counter bit 8, **bit 6 = halt**, **bit 7 = day carry** |
//!
//! # Why this is an offset and not a counter
//!
//! The clock runs on wall time, not on emulated cycles — it must advance while the emulator is
//! closed. So the state is a **`base`: the Unix second at which the counter read zero**
//! (gambatte's `rtc.cpp`), and the counter is `now - base`. Writing a register moves `base`;
//! nothing ticks.
//!
//! # ⚠️ The time source is injectable, and it has to be
//!
//! Reading `SystemTime::now()` directly from the emulator would make every fixture-driven test in
//! this repo non-deterministic — a replay would produce different register values on every run.
//! [`TimeSource::Fixed`] is what tests and replays use. It is an enum rather than a boxed trait
//! for the same reason [`crate::mbc::Mapper`] is: [`crate::mmu::MMU`] derives `Clone`, `PartialEq`,
//! `Encode` and `Decode`, and a trait object supplies none of them.

use bincode::{Decode, Encode};

/// Where the clock reads wall time from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum TimeSource {
    /// The host clock. What a real cartridge does, and the default.
    System,
    /// A fixed instant in Unix seconds, moved only by [`Rtc::set_time_source`]. **Use this in
    /// anything that must replay identically.**
    Fixed(u64),
}

impl TimeSource {
    fn now(&self) -> u64 {
        match self {
            Self::System => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            Self::Fixed(seconds) => *seconds,
        }
    }
}

/// The five registers as the guest reads them, frozen at the last latch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub struct RtcRegisters {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    /// The full 9-bit day counter, 0-511.
    pub days: u16,
    /// Bit 7 of `0x0C`: the day counter has passed 511 at least once. **Sticky** — only a write
    /// clears it, which is how a game detects a year-long absence.
    pub day_carry: bool,
    /// Bit 6 of `0x0C`: counting is stopped.
    pub halted: bool,
}

/// Seconds in a day, and the day counter's modulus.
const DAY: u64 = 24 * 60 * 60;
const DAY_COUNTER_MODULUS: u64 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Rtc {
    /// Unix second at which the running counter read zero.
    ///
    /// ⚠️ **Signed on purpose.** A guest may set the clock to a time *later* than the host's, and
    /// then the instant the counter read zero is before the epoch. An unsigned `base` with a
    /// saturating subtraction silently clamps that to zero and the write is lost — which is
    /// exactly what happened the first time this was wired through the MMU.
    base: i64,
    /// The counter's value at the moment it was halted. Meaningless while running.
    halted_at: u64,
    halted: bool,
    /// Sticky, and deliberately not derived from the counter: the counter wraps every 512 days
    /// and cannot tell you that it did.
    day_carry: bool,
    /// What `0xA000` shows. Hardware freezes the register file on latch so a guest can read five
    /// registers without one rolling over underneath it.
    latched: RtcRegisters,
    /// The `0x6000..=0x7FFF` latch sequence needs a `0` then a `1`; this is the last value seen.
    last_latch: u8,
    source: TimeSource,
}

impl Default for Rtc {
    fn default() -> Self {
        Self {
            base: TimeSource::System.now() as i64,
            halted_at: 0,
            halted: false,
            day_carry: false,
            latched: RtcRegisters::default(),
            last_latch: 0xFF,
            source: TimeSource::System,
        }
    }
}

impl Rtc {
    /// A clock pinned to a fixed instant and reading zero. For tests and replays.
    pub fn pinned(now: u64) -> Self {
        Self {
            base: now as i64,
            halted_at: 0,
            halted: false,
            day_carry: false,
            latched: RtcRegisters::default(),
            last_latch: 0xFF,
            source: TimeSource::Fixed(now),
        }
    }

    /// Swap the time source, **rebasing so the counter reads the same across the swap**. This is
    /// what pins a clock that is already running.
    ///
    /// ⚠️ It is *not* how you make time pass — that is [`Rtc::advance`]. Rebasing means setting a
    /// later `Fixed` instant here moves nothing, which is exactly the trap that made the first
    /// draft of this module's tests all pass vacuously.
    pub fn set_time_source(&mut self, source: TimeSource) {
        let counter = self.counter();
        self.source = source;
        self.set_counter(counter);
    }

    /// Let `seconds` of wall time pass on a **pinned** clock. A no-op on a system clock, which
    /// moves on its own.
    pub fn advance(&mut self, seconds: u64) {
        if let TimeSource::Fixed(now) = self.source {
            self.source = TimeSource::Fixed(now + seconds);
        }
    }

    pub fn time_source(&self) -> TimeSource {
        self.source
    }

    /// Seconds since the counter last read zero.
    fn counter(&self) -> u64 {
        if self.halted {
            self.halted_at
        } else {
            (self.source.now() as i64 - self.base).max(0) as u64
        }
    }

    /// Move the counter to `seconds` without changing whether it is running.
    fn set_counter(&mut self, seconds: u64) {
        if self.halted {
            self.halted_at = seconds;
        } else {
            self.base = self.source.now() as i64 - seconds as i64;
        }
    }

    /// The counter decomposed, with the day counter wrapped and the carry folded in.
    fn registers_now(&self) -> RtcRegisters {
        let counter = self.counter();
        let days = counter / DAY;
        RtcRegisters {
            seconds: (counter % 60) as u8,
            minutes: (counter / 60 % 60) as u8,
            hours: (counter / 3600 % 24) as u8,
            days: (days % DAY_COUNTER_MODULUS) as u16,
            // Sticky: once set it stays until written, so `||` rather than `=`.
            day_carry: self.day_carry || days >= DAY_COUNTER_MODULUS,
            halted: self.halted,
        }
    }

    /// A write to `0x6000..=0x7FFF`. The register file freezes on a `0` → `1` edge.
    pub fn write_latch(&mut self, value: u8) {
        if self.last_latch == 0 && value == 1 {
            self.latched = self.registers_now();
        }
        self.last_latch = value;
    }

    /// Read one of the five registers. `register` is the bank number, `0x08..=0x0C`.
    pub fn read(&self, register: u8) -> u8 {
        let r = self.latched;
        match register {
            0x08 => r.seconds,
            0x09 => r.minutes,
            0x0A => r.hours,
            0x0B => r.days as u8,
            0x0C => {
                (r.days >> 8) as u8 & 0x01
                    | if r.halted { 0x40 } else { 0 }
                    | if r.day_carry { 0x80 } else { 0 }
            }
            // Not a clock register. An open bus reads high.
            _ => 0xFF,
        }
    }

    /// Write one of the five registers.
    ///
    /// ⚠️ A write hits the **live** clock as well as the latched copy. Hardware has one register
    /// file; the latch only gates what a *read* sees, so setting the time through it must move the
    /// counter or the next latch would undo the write.
    pub fn write(&mut self, register: u8, value: u8) {
        let mut r = self.registers_now();
        match register {
            0x08 => r.seconds = value % 60,
            0x09 => r.minutes = value % 60,
            0x0A => r.hours = value % 24,
            0x0B => r.days = (r.days & 0x100) | value as u16,
            0x0C => {
                r.days = (r.days & 0x0FF) | ((value as u16 & 0x01) << 8);
                r.day_carry = value & 0x80 != 0;
                let halting = value & 0x40 != 0;
                if halting != self.halted {
                    // Freeze at, or resume from, the counter's current value.
                    let counter = self.counter();
                    self.halted = halting;
                    self.set_counter(counter);
                }
                r.halted = halting;
            }
            _ => return,
        }
        self.day_carry = r.day_carry;
        self.set_counter(
            r.days as u64 * DAY + r.hours as u64 * 3600 + r.minutes as u64 * 60 + r.seconds as u64,
        );
        self.latched = r;
    }

    /// Gambatte's `.rtc` sidecar: the base time as **4 bytes, big-endian**. Kept for interop with
    /// existing save directories — that is the only reason the format is this and not bincode.
    /// A base before the epoch cannot be expressed in the sidecar's unsigned 32 bits and clamps
    /// to zero. Only reachable with an artificially pinned clock; a system clock is never near it.
    pub fn to_gambatte_bytes(&self) -> [u8; 4] {
        (self.base.clamp(0, u32::MAX as i64) as u32).to_be_bytes()
    }

    /// Adopt a gambatte `.rtc` sidecar. Anything shorter than four bytes is ignored rather than
    /// treated as an error: a missing sidecar just means a clock that starts now.
    pub fn set_from_gambatte_bytes(&mut self, bytes: &[u8]) {
        if let [a, b, c, d, ..] = bytes {
            self.base = u32::from_be_bytes([*a, *b, *c, *d]) as i64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test pins the clock; see [`TimeSource`] for why that is not optional.
    fn latch(rtc: &mut Rtc) {
        rtc.write_latch(0);
        rtc.write_latch(1);
    }

    #[test]
    fn the_counter_decomposes_into_the_five_registers() {
        let mut rtc = Rtc::pinned(0);
        rtc.advance(3 * DAY + 4 * 3600 + 5 * 60 + 6);
        latch(&mut rtc);
        assert_eq!(rtc.read(0x08), 6, "seconds");
        assert_eq!(rtc.read(0x09), 5, "minutes");
        assert_eq!(rtc.read(0x0A), 4, "hours");
        assert_eq!(rtc.read(0x0B), 3, "days");
        assert_eq!(rtc.read(0x0C), 0, "no high day bit, not halted, no carry");
    }

    /// ⚠️ The registers freeze on latch. Without this a guest reading five registers in sequence
    /// could see 00:00:59 roll to 00:01:00 between two of them and record 00:00:00.
    #[test]
    fn reads_are_frozen_until_the_next_latch() {
        let mut rtc = Rtc::pinned(0);
        latch(&mut rtc);
        assert_eq!(rtc.read(0x08), 0);

        rtc.advance(30);
        assert_eq!(rtc.read(0x08), 0, "still the latched value");
        latch(&mut rtc);
        assert_eq!(rtc.read(0x08), 30, "now it moves");
    }

    /// The latch needs a genuine `0` → `1` edge, not just a write of `1`.
    #[test]
    fn latching_needs_a_zero_then_a_one() {
        let mut rtc = Rtc::pinned(0);
        rtc.advance(42);
        rtc.write_latch(1);
        rtc.write_latch(1);
        assert_eq!(rtc.read(0x08), 0, "no edge, no latch");
        latch(&mut rtc);
        assert_eq!(rtc.read(0x08), 42);
    }

    #[test]
    fn the_day_counter_is_nine_bits_and_the_carry_is_sticky() {
        let mut rtc = Rtc::pinned(0);
        rtc.advance(300 * DAY);
        latch(&mut rtc);
        assert_eq!(rtc.read(0x0B), (300 & 0xFF) as u8);
        assert_eq!(rtc.read(0x0C) & 0x01, 1, "day 300 needs the ninth bit");
        assert_eq!(rtc.read(0x0C) & 0x80, 0, "not yet wrapped");

        rtc.advance(300 * DAY); // day 600
        latch(&mut rtc);
        assert_eq!(rtc.read(0x0B), ((600 - 512) & 0xFF) as u8, "wrapped past 511");
        assert_eq!(rtc.read(0x0C) & 0x80, 0x80, "carry set");

        // Sticky: it stays set even once the counter is well past the wrap.
        rtc.advance(100 * DAY);
        latch(&mut rtc);
        assert_eq!(rtc.read(0x0C) & 0x80, 0x80);
    }

    /// Halting freezes the counter; resuming continues from where it stopped rather than jumping
    /// forward by the time spent halted.
    #[test]
    fn halting_freezes_the_counter() {
        let mut rtc = Rtc::pinned(0);
        rtc.advance(100); // 00:01:40
        rtc.write(0x0C, 0x40); // halt
        latch(&mut rtc);
        assert_eq!((rtc.read(0x09), rtc.read(0x08)), (1, 40));
        assert_eq!(rtc.read(0x0C) & 0x40, 0x40, "halt bit reads back");

        rtc.advance(10_000);
        latch(&mut rtc);
        assert_eq!((rtc.read(0x09), rtc.read(0x08)), (1, 40), "frozen while halted");

        rtc.write(0x0C, 0x00); // resume
        rtc.advance(60);
        latch(&mut rtc);
        assert_eq!((rtc.read(0x09), rtc.read(0x08)), (2, 40), "resumed from 100s, not from 10100s");
    }

    /// ⚠️ A write must move the *live* counter, not just the latched copy — otherwise the next
    /// latch silently undoes it.
    #[test]
    fn a_write_survives_the_next_latch() {
        let mut rtc = Rtc::pinned(1_000_000);
        rtc.write(0x0A, 7);
        rtc.write(0x09, 30);
        rtc.write(0x08, 15);
        latch(&mut rtc);
        assert_eq!((rtc.read(0x0A), rtc.read(0x09), rtc.read(0x08)), (7, 30, 15));

        // ...and it keeps running from there.
        rtc.advance(45);
        latch(&mut rtc);
        assert_eq!((rtc.read(0x0A), rtc.read(0x09), rtc.read(0x08)), (7, 31, 0));
    }

    /// Writing the carry bit clears it — that is how a game acknowledges the wrap.
    #[test]
    fn the_carry_bit_is_cleared_by_writing_it() {
        let mut rtc = Rtc::pinned(0);
        rtc.advance(600 * DAY);
        latch(&mut rtc);
        assert_eq!(rtc.read(0x0C) & 0x80, 0x80);

        rtc.write(0x0C, 0x00);
        latch(&mut rtc);
        assert_eq!(rtc.read(0x0C) & 0x80, 0, "acknowledged");
    }

    /// ⚠️ Pinning a running clock preserves what it reads; it does not rewind it. Making time
    /// pass is [`Rtc::advance`], and conflating the two makes every test above pass vacuously.
    #[test]
    fn pinning_a_running_clock_preserves_the_counter() {
        let mut rtc = Rtc::pinned(0);
        rtc.advance(12_345);
        latch(&mut rtc);
        let before = (rtc.read(0x0A), rtc.read(0x09), rtc.read(0x08));

        rtc.set_time_source(TimeSource::Fixed(999_999));
        latch(&mut rtc);
        assert_eq!((rtc.read(0x0A), rtc.read(0x09), rtc.read(0x08)), before);
    }

    /// ⚠️ Regression: a guest may set the clock **later than the host's own time**, which puts the
    /// instant the counter read zero before the epoch. With an unsigned base and a saturating
    /// subtraction the write was silently lost.
    #[test]
    fn a_time_later_than_the_host_clock_survives() {
        let mut rtc = Rtc::pinned(0);
        rtc.write(0x0B, 200); // day 200, well past a base of zero
        rtc.write(0x0A, 13);
        latch(&mut rtc);
        assert_eq!((rtc.read(0x0B), rtc.read(0x0A)), (200, 13));

        rtc.advance(3600);
        latch(&mut rtc);
        assert_eq!((rtc.read(0x0B), rtc.read(0x0A)), (200, 14), "and it keeps running");
    }

    #[test]
    fn the_gambatte_sidecar_round_trips() {
        let mut rtc = Rtc::pinned(1_700_000_000);
        let bytes = rtc.to_gambatte_bytes();
        let base = rtc.base;

        rtc.set_from_gambatte_bytes(&[0, 0, 0, 0]);
        assert_ne!(rtc.base, base);
        rtc.set_from_gambatte_bytes(&bytes);
        assert_eq!(rtc.base, base);

        // A missing or short sidecar is ignored, not an error.
        rtc.set_from_gambatte_bytes(&[1, 2]);
        assert_eq!(rtc.base, base);
    }

    /// A register that is not one of the five reads as open bus and drops writes.
    #[test]
    fn a_non_register_reads_high() {
        let mut rtc = Rtc::pinned(0);
        assert_eq!(rtc.read(0x0D), 0xFF);
        rtc.write(0x0D, 0x12); // must not panic or disturb the clock
        latch(&mut rtc);
        assert_eq!(rtc.read(0x08), 0);
    }
}
