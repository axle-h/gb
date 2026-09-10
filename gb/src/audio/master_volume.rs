use bincode::{Decode, Encode};
use crate::audio::sample::AudioSample;

/// FF24 — NR50: Master volume & VIN panning
/// VIN left/right: Set to 0 if external sound hardware is not being used.
/// Left/right volume: These specify the master volume, i.e. how much each output should be scaled.
///                    A value of 0 is treated as a volume of 1 (very quiet),
///                    and a value of 7 is treated as a volume of 8 (no volume reduction).
///                    Importantly, the amplifier never mutes a non-silent input.
#[derive(Debug, Clone, Default, Eq, PartialEq, Decode, Encode)]
pub struct MasterVolume {
    vin_left: bool, // bit 7
    vin_right: bool, // bit 3
    left_volume: u8, // bits 4-6
    right_volume: u8, // bits 0-2
}

impl MasterVolume {
    pub fn get_byte(&self) -> u8 {
        let mut byte = 0;
        if self.vin_left { byte |= 0x80; }
        if self.vin_right { byte |= 0x08; }
        byte |= (self.left_volume & 0x7) << 4;
        byte |= self.right_volume & 0x7;
        byte
    }

    pub fn set_byte(&mut self, value: u8) {
        self.vin_left = (value & 0x80) != 0; // bit 7
        self.vin_right = (value & 0x08) != 0; // bit 3
        self.left_volume = (value >> 4) & 0x07; // bits 4-6
        self.right_volume = value & 0x07; // bits 0-2
    }

    pub fn left_volume(&self) -> u8 {
        self.left_volume
    }

    pub fn right_volume(&self) -> u8 {
        self.right_volume
    }

    pub fn volume_sample(&self) -> AudioSample {
        AudioSample::new(Self::to_f32(self.left_volume), Self::to_f32(self.right_volume)) / 7.0
    }

    pub fn vin_left(&self) -> bool {
        self.vin_left
    }

    pub fn vin_right(&self) -> bool {
        self.vin_right
    }

    fn to_f32(volume: u8) -> f32 {
        match volume {
            0 => 1.0 / 8.0,
            1 => 2.0 / 8.0,
            2 => 3.0 / 8.0,
            3 => 4.0 / 8.0,
            4 => 5.0 / 8.0,
            5 => 6.0 / 8.0,
            6 => 7.0 / 8.0,
            7 => 1.0,
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let volume = MasterVolume::default();
        assert_eq!(volume.get_byte(), 0);
        assert_eq!(volume.left_volume(), 0);
        assert_eq!(volume.right_volume(), 0);
        assert!(!volume.vin_left());
        assert!(!volume.vin_right());
    }

    #[test]
    fn set_and_get_all_bits() {
        let mut volume = MasterVolume::default();
        volume.set_byte(0xFF); // All bits set

        assert_eq!(volume.get_byte(), 0xFF);
        assert!(volume.vin_left());
        assert!(volume.vin_right());
        assert_eq!(volume.left_volume(), 7);  // bits 4-6 = 0b111 = 7
        assert_eq!(volume.right_volume(), 7); // bits 0-2 = 0b111 = 7
    }

    #[test]
    fn vin_bits() {
        let mut volume = MasterVolume::default();

        // Test VIN left (bit 7)
        volume.set_byte(0x80);
        assert!(volume.vin_left());
        assert!(!volume.vin_right());
        assert_eq!(volume.left_volume(), 0);
        assert_eq!(volume.right_volume(), 0);
        assert_eq!(volume.get_byte(), 0x80);

        // Test VIN right (bit 3)
        volume.set_byte(0x08);
        assert!(!volume.vin_left());
        assert!(volume.vin_right());
        assert_eq!(volume.left_volume(), 0);
        assert_eq!(volume.right_volume(), 0);
        assert_eq!(volume.get_byte(), 0x08);
    }

    #[test]
    fn volume_levels() {
        let mut volume = MasterVolume::default();

        // Test left volume (bits 4-6)
        for level in 0..=7 {
            volume.set_byte(level << 4);
            assert_eq!(volume.left_volume(), level);
            assert_eq!(volume.right_volume(), 0);
            assert_eq!(volume.get_byte(), level << 4);
        }

        // Test right volume (bits 0-2)
        for level in 0..=7 {
            volume.set_byte(level);
            assert_eq!(volume.right_volume(), level);
            assert_eq!(volume.left_volume(), 0);
            assert_eq!(volume.get_byte(), level);
        }
    }

    #[test]
    fn roundtrip_consistency() {
        let mut volume = MasterVolume::default();

        // Test that set/get roundtrips work for all valid combinations
        for value in 0..=255 {
            volume.set_byte(value);
            assert_eq!(volume.get_byte(), value);
        }
    }
}
