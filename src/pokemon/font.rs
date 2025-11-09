use std::ops::Add;
use crate::mmu::{MMU, ROM_BANK_SIZE};
use crate::pokemon::symbols::pokered_symbols;

const fn font_bytes() -> [u8; 0x800] {
    use crate::pokemon::roms::POKERED;
    use crate::pokemon::symbols::pokered_symbols;
    let font_pointer = pokered_symbols::FontGraphics;
    let end_pointer = pokered_symbols::FontGraphicsEnd;

    let length = end_pointer.address as usize - font_pointer.address as usize;
    if length != 0x400 {
        // compressed 1bpp
        panic!("Font bytes length is incorrect");
    }

    let rom_address = font_pointer.address as usize - ROM_BANK_SIZE + font_pointer.bank.id() as usize * ROM_BANK_SIZE;

    let mut result = [0; 0x800];
    let mut i = 0;
    while i < 0x400 {
        let bpp1 = POKERED[rom_address + i];
        result[i * 2] = bpp1;
        result[i * 2 + 1] = bpp1;
        i += 1;
    }
    result
}

pub const FONT_BYTES: [u8; 0x800] = font_bytes();

pub trait FontAware {
    fn pokemon_font_loaded(&self) -> bool;
}

impl FontAware for MMU {
    fn pokemon_font_loaded(&self) -> bool {
        // TODO refactor the pointer access stuff to use slices like this, would have to assume that all data for a slice is in the same bank
        let loaded = self.read_vram_slice(pokered_symbols::vFont.address, FONT_BYTES.len())
            .expect("Failed to read font from vram");
        loaded == FONT_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_bytes_is_correct_length() {
        assert_eq!(FONT_BYTES.len(), 0x800);
        assert!(FONT_BYTES.iter().any(|&b| b > 0), "font_bytes should not be all zeros");
    }

    #[test]
    fn font_bytes_duplicates_correctly() {
        for i in 0..0x400 {
            assert_eq!(FONT_BYTES[i * 2], FONT_BYTES[i * 2 + 1]);
        }
    }

    #[test]
    fn font_bytes_are_correct() {
        // validate the first 10 bytes
        let expected = [0x10, 0x10, 0x28, 0x28, 0x28, 0x28, 0x44, 0x44, 0x7c, 0x7c];
        for i in 0..10 {
            assert_eq!(FONT_BYTES[i], expected[i], "Byte {} does not match expected value", i);
        }
    }
}
