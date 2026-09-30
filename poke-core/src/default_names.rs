//! `data/player/names_list.asm`: the names Oak offers for the player and the rival, as
//! `GetDefaultName` reads them, with `NEW NAME` at index 0.

use crate::charmap::encode;
use crate::tables::{DEFAULT_NAMES_PLAYER_LIST, DEFAULT_NAMES_RIVAL_LIST};

/// `DefaultNamesPlayerList`: `NEW NAME` and the three names, charmap bytes, unterminated.
pub fn player_names() -> Vec<Vec<u8>> {
    names(&DEFAULT_NAMES_PLAYER_LIST)
}

/// `DefaultNamesRivalList`.
pub fn rival_names() -> Vec<Vec<u8>> {
    names(&DEFAULT_NAMES_RIVAL_LIST)
}

fn names(list: &[&str]) -> Vec<Vec<u8>> {
    list.iter().map(|name| encode(name).expect("a default name is in the charmap")).collect()
}

#[cfg(test)]
mod tests {
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
