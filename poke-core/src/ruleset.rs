use serde::{Deserialize, Serialize};

/// Which rules a battle and the world play by: the cartridge's, every mechanical and audio bug
/// kept, or the same game with each of them fixed. The emulated game is always Gen 1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ruleset {
    Gen1,
    #[default]
    Modern,
}

impl Ruleset {
    pub fn is_gen1(self) -> bool {
        self == Ruleset::Gen1
    }
}
