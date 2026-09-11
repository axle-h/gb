//! The recreation and the emulator fed the same buttons and compared.

mod audio;
mod list_menu;
mod screen;
mod text_box;

use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::cycles::MachineCycles;
use gb::ram::ROM;
use crate::pokemon::symbols::{DmgBank, DmgPointer};

pub(crate) fn breakpoint(pointer: DmgPointer) -> Breakpoint {
    match pointer.bank {
        DmgBank::ROM { bank } => Breakpoint::new(bank, pointer.address),
        _ => Breakpoint::new(0, pointer.address),
    }
}

/// Runs to the start of the next VBlank handler, when the frame before it has done its work.
pub(crate) fn to_vblank(gb: &mut GameBoy) {
    let vblank = breakpoint(crate::pokemon::symbols::pokered_symbols::VBlank);
    let (stop, _) = gb.run_until(&[vblank], MachineCycles::PER_FRAME * 2);
    assert_eq!(stop, Stop::Breakpoint(vblank), "no VBlank within two frames");
}

pub(crate) fn tile_row(gb: &GameBoy, y: u16) -> Vec<u8> {
    let tile_map = crate::pokemon::symbols::pokered_symbols::wTileMap.address;
    (0..20).map(|x| gb.core().mmu().read(tile_map + y * 20 + x)).collect()
}
