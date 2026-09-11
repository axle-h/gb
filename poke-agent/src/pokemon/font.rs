use gb::mmu::MMU;
use crate::pokemon::symbols::pokered_symbols;
pub use poke_core::font::*;

pub trait FontAware {
    fn pokemon_font_loaded(&self) -> bool;
}

impl FontAware for MMU {
    fn pokemon_font_loaded(&self) -> bool {
        // TODO refactor the pointer access stuff to use slices like this, would have to assume
        // that all data for a slice is in the same bank
        let loaded = self.read_vram_slice(pokered_symbols::vFont.address, FONT_BYTES.len())
            .expect("Failed to read font from vram");
        loaded == FONT_BYTES
    }
}
