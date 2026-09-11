//! The sound hardware as the audio engine sees it. The engine speaks in `Write`s, each the exact
//! register byte it would be on the cartridge, and a backend renders them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    Pulse1,
    Pulse2,
    Wave,
    Noise,
}

impl Channel {
    /// NRx0, the first of the channel's five registers.
    const fn base(self) -> u16 {
        match self {
            Channel::Pulse1 => 0xFF10,
            Channel::Pulse2 => 0xFF15,
            Channel::Wave => 0xFF1A,
            Channel::Noise => 0xFF1F,
        }
    }
}

/// Bits the hardware ignores are not carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Write {
    /// NR10.
    Sweep { pace: u8, decrease: bool, step: u8 },
    /// NR11, NR21, NR31 and NR41; a wave or noise length has no duty.
    Length { channel: Channel, duty: u8, length: u8 },
    /// NR12, NR22 and NR42.
    Envelope { channel: Channel, volume: u8, increase: bool, pace: u8 },
    /// NR13, NR23 and NR33.
    PeriodLow { channel: Channel, low: u8 },
    /// NR14, NR24, NR34 and NR44, which has no period bits.
    PeriodHigh { channel: Channel, high: u8, trigger: bool, length_enable: bool },
    /// NR30.
    WaveDac(bool),
    /// NR32: 0 silent, 1 full, 2 half, 3 quarter.
    WaveLevel(u8),
    /// NR43.
    Noise { shift: u8, short: bool, divisor: u8 },
    /// NR50.
    MasterVolume { left: u8, right: u8, vin_left: bool, vin_right: bool },
    /// NR51, a bit per channel and side.
    Panning(u8),
    /// NR52.
    Power(bool),
    /// Two 4-bit samples of the 32 in wave RAM.
    WaveRam { index: u8, samples: u8 },
}

impl Write {
    /// The register and the byte written to it.
    pub fn register(self) -> (u16, u8) {
        let bit = |set: bool, n: u8| (set as u8) << n;
        match self {
            Write::Sweep { pace, decrease, step } => (0xFF10, pace << 4 | bit(decrease, 3) | step),
            Write::Length { channel, duty, length } => (channel.base() + 1, duty << 6 | length),
            Write::Envelope { channel, volume, increase, pace } => (channel.base() + 2, volume << 4 | bit(increase, 3) | pace),
            Write::PeriodLow { channel, low } => (channel.base() + 3, low),
            Write::PeriodHigh { channel, high, trigger, length_enable } =>
                (channel.base() + 4, bit(trigger, 7) | bit(length_enable, 6) | high),
            Write::WaveDac(on) => (0xFF1A, bit(on, 7)),
            Write::WaveLevel(level) => (0xFF1C, level << 5),
            Write::Noise { shift, short, divisor } => (0xFF22, shift << 4 | bit(short, 3) | divisor),
            Write::MasterVolume { left, right, vin_left, vin_right } =>
                (0xFF24, bit(vin_left, 7) | left << 4 | bit(vin_right, 3) | right),
            Write::Panning(panning) => (0xFF25, panning),
            Write::Power(on) => (0xFF26, bit(on, 7)),
            Write::WaveRam { index, samples } => (0xFF30 + index as u16, samples),
        }
    }

    /// The write a byte to `address` is, or `None` for an address outside the sound registers.
    pub fn decode(address: u16, value: u8) -> Option<Write> {
        let set = |n: u8| value & (1 << n) != 0;
        let channel = match address {
            0xFF10..=0xFF14 => Channel::Pulse1,
            0xFF15..=0xFF19 => Channel::Pulse2,
            0xFF1A..=0xFF1E => Channel::Wave,
            0xFF1F..=0xFF23 => Channel::Noise,
            _ => Channel::Pulse1,
        };
        Some(match address {
            0xFF10 => Write::Sweep { pace: value >> 4 & 7, decrease: set(3), step: value & 7 },
            0xFF11 | 0xFF16 => Write::Length { channel, duty: value >> 6, length: value & 0x3F },
            0xFF1B => Write::Length { channel, duty: 0, length: value },
            0xFF20 => Write::Length { channel, duty: 0, length: value & 0x3F },
            0xFF12 | 0xFF17 | 0xFF21 => Write::Envelope { channel, volume: value >> 4, increase: set(3), pace: value & 7 },
            0xFF13 | 0xFF18 | 0xFF1D => Write::PeriodLow { channel, low: value },
            0xFF14 | 0xFF19 | 0xFF1E => Write::PeriodHigh { channel, high: value & 7, trigger: set(7), length_enable: set(6) },
            0xFF23 => Write::PeriodHigh { channel, high: 0, trigger: set(7), length_enable: set(6) },
            0xFF1A => Write::WaveDac(set(7)),
            0xFF1C => Write::WaveLevel(value >> 5 & 3),
            0xFF22 => Write::Noise { shift: value >> 4, short: set(3), divisor: value & 7 },
            0xFF24 => Write::MasterVolume { left: value >> 4 & 7, right: value & 7, vin_left: set(7), vin_right: set(3) },
            0xFF25 => Write::Panning(value),
            0xFF26 => Write::Power(set(7)),
            0xFF30..=0xFF3F => Write::WaveRam { index: (address - 0xFF30) as u8, samples: value },
            _ => return None,
        })
    }
}

/// Where the engine's writes go.
pub trait Voices {
    fn write(&mut self, write: Write);
    /// Plays one frame's sound, after that frame's writes.
    fn end_frame(&mut self);
    /// Interleaved stereo samples, returning how many frames of them were ready.
    fn read_samples(&mut self, out: &mut [f32]) -> usize;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bits each register keeps; the rest the hardware ignores.
    fn meaningful(address: u16) -> u8 {
        match address {
            0xFF10 => 0x7F,
            0xFF14 | 0xFF19 | 0xFF1E => 0xC7,
            0xFF1A => 0x80,
            0xFF1C => 0x60,
            0xFF20 => 0x3F,
            0xFF23 => 0xC0,
            0xFF26 => 0x80,
            _ => 0xFF,
        }
    }

    #[test]
    fn every_register_byte_round_trips_through_its_write() {
        for address in (0xFF10..=0xFF26).chain(0xFF30..=0xFF3F) {
            for value in 0..=u8::MAX {
                let Some(write) = Write::decode(address, value) else {
                    assert!(matches!(address, 0xFF15 | 0xFF1F), "${address:04X} is a register");
                    continue;
                };
                let mask = meaningful(address);
                assert_eq!(write.register(), (address, value & mask), "${address:04X} = ${value:02X}");
            }
        }
    }
}
