//! `data/player/names_list.asm`: the names Oak offers for the player and the rival, as
//! `GetDefaultName` reads them, with `NEW NAME` at index 0.

use crate::rom_gfx::rom_slice;
use crate::symbols::{pokered_symbols, DmgPointer};

/// `NUM_PLAYER_NAMES`, and the `NEW NAME` row before them.
const ENTRIES: usize = 3 + 1;
const TERMINATOR: u8 = 0x50;

/// `DefaultNamesPlayerList`: `NEW NAME` and the three names, charmap bytes, unterminated.
pub fn player_names() -> Vec<Vec<u8>> {
    names(pokered_symbols::DefaultNamesPlayerList)
}

/// `DefaultNamesRivalList`.
pub fn rival_names() -> Vec<Vec<u8>> {
    names(pokered_symbols::DefaultNamesRivalList)
}

/// `GetDefaultName`'s walk: each entry runs to its `@`.
fn names(list: DmgPointer) -> Vec<Vec<u8>> {
    rom_slice(list).split(|&byte| byte == TERMINATOR).take(ENTRIES).map(<[u8]>::to_vec).collect()
}

#[cfg(test)]
mod tests {
    use crate::charmap::encode;
    use super::*;

    fn words(names: &[&str]) -> Vec<Vec<u8>> {
        names.iter().map(|name| encode(name).unwrap()).collect()
    }

    #[test]
    fn red_offers_the_red_names_and_the_blue_ones_for_the_rival() {
        assert_eq!(player_names(), words(&["NEW NAME", "RED", "ASH", "JACK"]));
        assert_eq!(rival_names(), words(&["NEW NAME", "BLUE", "GARY", "JOHN"]));
    }
}
