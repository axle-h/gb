use bincode::{Decode, Encode};

use crate::lcd_palette::LcdColor;

/// Bytes in one palette bank: 8 palettes x 4 colours x 2 bytes.
pub const PALETTE_BYTES: usize = 64;
/// Colours in one palette bank.
pub const PALETTE_COLORS: usize = PALETTE_BYTES / 2;

/// One bank of CGB palette RAM plus its index register — `BCPS`/`BCPD` (`FF68`/`FF69`) for the
/// background, `OCPS`/`OCPD` (`FF6A`/`FF6B`) for objects.
///
/// The raw bytes are authoritative; `colors` is a pre-expanded mirror kept in step on every write
/// so the pixel path never unpacks RGB555 per pixel (gambatte keeps the same mirror,
/// `video.h:205-206`). Only the raw bytes are serialised — the mirror is rebuilt on load.
///
/// **Mode-3 access blocking is not modelled.** Hardware rejects palette reads and writes during
/// mode 3; `gb` renders mode 3 as a fixed 172-tick block, so it has nowhere accurate to put the
/// boundary. Deferred with the rest of the mode-3 timing work (plan §0.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteBank {
    data: [u8; PALETTE_BYTES],
    /// Raw index register: bit 7 = auto-increment, bits 0-5 = byte address.
    index: u8,
    colors: [LcdColor; PALETTE_COLORS],
}

impl Default for PaletteBank {
    fn default() -> Self {
        // Power-on palette RAM is not specified; gambatte starts it all-ones, which reads as
        // white and is what a CGB game sees before it writes its own palettes.
        let mut bank = Self {
            data: [0xFF; PALETTE_BYTES],
            index: 0,
            colors: [LcdColor::WHITE; PALETTE_COLORS],
        };
        bank.rebuild();
        bank
    }
}

impl PaletteBank {
    /// `BCPS`/`OCPS` read-back: bit 6 is unused and reads 1.
    pub fn index(&self) -> u8 {
        self.index | 0x40
    }

    pub fn set_index(&mut self, value: u8) {
        self.index = value & 0xBF;
    }

    /// `BCPD`/`OCPD`.
    pub fn read(&self) -> u8 {
        self.data[(self.index & 0x3F) as usize]
    }

    pub fn write(&mut self, value: u8) {
        let address = (self.index & 0x3F) as usize;
        self.data[address] = value;
        self.refresh(address / 2);
        // Auto-increment wraps within the 64-byte window and leaves bit 7 alone.
        if self.index & 0x80 != 0 {
            self.index = (self.index & 0x80) | ((self.index + 1) & 0x3F);
        }
    }

    /// The expanded colour at `palette * 4 + shade`.
    pub fn color(&self, palette: u8, shade: u8) -> LcdColor {
        self.colors[((palette & 0x07) * 4 + (shade & 0x03)) as usize]
    }

    pub fn data(&self) -> &[u8; PALETTE_BYTES] {
        &self.data
    }

    /// Write one 4-colour palette from eight raw bytes, bypassing the index register. This is how
    /// the boot-ROM palette is installed (B5).
    pub fn set_palette(&mut self, palette: usize, bytes: [u8; 8]) {
        let base = palette * 8;
        self.data[base..base + 8].copy_from_slice(&bytes);
        for i in 0..4 {
            self.refresh(palette * 4 + i);
        }
    }

    fn refresh(&mut self, color: usize) {
        let lo = self.data[color * 2] as u16;
        let hi = self.data[color * 2 + 1] as u16;
        self.colors[color] = LcdColor::from_rgb555(lo | (hi << 8));
    }

    fn rebuild(&mut self) {
        for color in 0..PALETTE_COLORS {
            self.refresh(color);
        }
    }
}

/// The `cgb` save-state section's palette payload: raw bytes and index registers only. The
/// expanded mirrors are derived and are rebuilt on load.
#[derive(Debug, Clone, Decode, Encode)]
pub struct PaletteBankState {
    pub data: [u8; PALETTE_BYTES],
    pub index: u8,
}

impl From<&PaletteBank> for PaletteBankState {
    fn from(bank: &PaletteBank) -> Self {
        Self { data: bank.data, index: bank.index }
    }
}

impl From<PaletteBankState> for PaletteBank {
    fn from(state: PaletteBankState) -> Self {
        let mut bank = PaletteBank { data: state.data, index: state.index, colors: [LcdColor::WHITE; PALETTE_COLORS] };
        bank.rebuild();
        bank
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_round_trip_and_auto_increment_wraps() {
        let mut bank = PaletteBank::default();
        bank.set_index(0x80); // address 0, auto-increment
        for i in 0..PALETTE_BYTES {
            bank.write(i as u8);
        }
        // 64 increments wrap back to 0, and bit 7 survives.
        assert_eq!(bank.index(), 0x80 | 0x40);

        bank.set_index(0x00); // address 0, no auto-increment
        for i in 0..PALETTE_BYTES {
            bank.set_index(i as u8);
            assert_eq!(bank.read(), i as u8, "byte {i}");
        }
    }

    #[test]
    fn without_auto_increment_the_address_stays_put() {
        let mut bank = PaletteBank::default();
        bank.set_index(0x05);
        bank.write(0x11);
        bank.write(0x22);
        assert_eq!(bank.index(), 0x05 | 0x40);
        assert_eq!(bank.read(), 0x22);
    }

    /// The mirror must track the raw bytes on every write, not just on a bulk load.
    #[test]
    fn expanded_mirror_follows_the_raw_bytes() {
        let mut bank = PaletteBank::default();
        // Palette 1, colour 2 = 0x421F -> r=31 g=16 b=16.
        bank.set_index(0x80 | (1 * 8 + 2 * 2) as u8);
        bank.write(0x1F);
        bank.write(0x42);
        assert_eq!(bank.color(1, 2), LcdColor::rgb(0xFF, 0x84, 0x84));
    }

    #[test]
    fn state_round_trips_through_the_savestate_shape() {
        let mut bank = PaletteBank::default();
        bank.set_index(0x80);
        for i in 0..PALETTE_BYTES {
            bank.write((i as u8).wrapping_mul(7));
        }
        let restored: PaletteBank = PaletteBankState::from(&bank).into();
        assert_eq!(restored, bank, "raw bytes, index and rebuilt mirror must all match");
    }
}
