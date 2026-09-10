use bincode::{Decode, Encode};

use crate::header::{CGBMode, CartHeader};

/// Which console the cartridge is running on.
///
/// Following gambatte, this is a *predicate* consulted in a few dozen places rather than a
/// parallel code path (`cartridge.cpp:635`, `memptrs.h:100-105`). Resist forking the PPU.
///
/// `Mgb` (pocket) and `Sgb` (Super Game Boy) are deliberately absent rather than stubbed — add
/// them when something needs them, so nothing has to guess at their behaviour in the meantime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
pub enum Model {
    #[default]
    Dmg,
    Cgb,
}

impl Model {
    pub fn is_cgb(self) -> bool {
        self == Model::Cgb
    }
}

/// How the machine actually renders and which registers exist, once the cartridge has had its say.
///
/// A CGB console runs a DMG-only cartridge — cart byte `0x143` with bit 7 clear — in
/// *compatibility* mode: the CGB's colour hardware is present and drives the screen, but the
/// cartridge only ever sees the DMG register set. Pokémon Red is exactly this case, and it is why
/// it comes out red-tinted on a Game Boy Color rather than in greyscale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// DMG hardware. Four shades, `BGP`/`OBP0`/`OBP1`, sprite priority by X coordinate.
    #[default]
    Dmg,
    /// CGB hardware running a DMG cartridge. The DMG palette registers still select a shade, but
    /// the shade then indexes CGB palette RAM, which the boot ROM has pre-loaded from the
    /// cartridge title (see [`crate::boot_palette`]). BG map attributes are ignored.
    CgbCompat,
    /// CGB hardware running a CGB-aware cartridge. Everything is available.
    Cgb,
}

impl ColorMode {
    pub fn of(model: Model, header: &CartHeader) -> Self {
        match (model, header.cgb_mode()) {
            (Model::Dmg, _) => ColorMode::Dmg,
            (Model::Cgb, CGBMode::None) => ColorMode::CgbCompat,
            (Model::Cgb, _) => ColorMode::Cgb,
        }
    }

    /// True when the *cartridge* can see CGB hardware — i.e. the CGB-only registers respond and
    /// BG map attributes are honoured. False in compatibility mode, where the boot ROM has locked
    /// the machine into the DMG register set.
    pub fn cgb_features(self) -> bool {
        self == ColorMode::Cgb
    }
}
