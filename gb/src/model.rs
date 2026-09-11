use bincode::{Decode, Encode};

use crate::header::{CGBMode, CartHeader};

/// Which console the cartridge is running on.
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

/// How the machine actually renders and which registers exist, once the cartridge has had its
/// say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// DMG hardware.
    #[default]
    Dmg,
    /// CGB hardware running a DMG cartridge.
    CgbCompat,
    /// CGB hardware running a CGB-aware cartridge.
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
