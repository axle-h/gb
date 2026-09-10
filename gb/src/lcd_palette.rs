use std::ops::{Deref, DerefMut};
use bincode::{Decode, Encode};
use image::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::FromRepr, Default, Decode, Encode)]
#[repr(u8)]
pub enum DMGColor {
    #[default]
    White = 0,
    LightGray = 1,
    DarkGray = 2,
    Black = 3,
}

impl DMGColor {

    pub fn to_rgb(self) -> Rgb<u8> {
        self.to_lcd().to_rgb()
    }

    /// The shade as it reaches the frame buffer on **DMG**. `FF/AA/55/00` is what `gb` has always
    /// emitted, and it is also the ramp `c-sp/game-boy-test-roms` documents for its reference
    /// screenshots, so every committed PNG depends on these four values exactly.
    ///
    /// Note that `0xAA` is **not** expressible as a 5-bit channel widened back to 8
    /// (`(21 << 3) | (21 >> 2)` is `0xAD`), which is why the frame buffer holds 24-bit colour
    /// rather than RGB555 — see [`LcdColor`].
    pub const fn to_lcd(self) -> LcdColor {
        match self {
            DMGColor::White => LcdColor::rgb(0xFF, 0xFF, 0xFF),
            DMGColor::LightGray => LcdColor::rgb(0xAA, 0xAA, 0xAA),
            DMGColor::DarkGray => LcdColor::rgb(0x55, 0x55, 0x55),
            DMGColor::Black => LcdColor::rgb(0x00, 0x00, 0x00),
        }
    }
}

/// One frame-buffer pixel: 24-bit colour packed as `0x00RRGGBB`.
///
/// B4 widened the frame buffer from a 2-bit DMG shade to this. 24 bits rather than the CGB's
/// native RGB555 for one reason: DMG's `0xAA` grey has no 5-bit representation, so storing RGB555
/// would silently shift every existing reference screenshot by three units. Widening at the point
/// the colour is *chosen* keeps DMG output bit-identical and costs nothing — the buffer is derived
/// state and is not serialised.
///
/// gambatte's output buffer is 32-bit for the same reason (`video.h`, `gambatte::uint_least32_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct LcdColor(u32);

impl LcdColor {
    pub const WHITE: Self = Self::rgb(0xFF, 0xFF, 0xFF);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self((r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    /// Widen a CGB `0bBBBBBGGGGGRRRRR` colour to 24 bits by replicating each channel's top bits
    /// into the low ones — `(c << 3) | (c >> 2)`, so `31` maps to `0xFF` and `0` to `0x00`.
    ///
    /// This is the plain expansion, **not** a colour-correction curve. It is what `cgb-acid2`'s
    /// committed reference image uses: its palette resolves to `0x6B/0xBD/0xFF`, `0x9C`, `0x73`
    /// and `0xAD`, each of which is exactly this formula applied to 13, 23, 31, 19, 14 and 21.
    /// A corrected curve (gambatte's `gbcToRgb32`) would not match it.
    pub const fn from_rgb555(value: u16) -> Self {
        let r = (value & 0x1F) as u8;
        let g = ((value >> 5) & 0x1F) as u8;
        let b = ((value >> 10) & 0x1F) as u8;
        Self::rgb(expand5(r), expand5(g), expand5(b))
    }

    pub const fn to_rgb(self) -> Rgb<u8> {
        Rgb([(self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8])
    }
}

const fn expand5(channel: u8) -> u8 {
    (channel << 3) | (channel >> 2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
pub struct DMGPaletteRegister([DMGColor; 4]);

impl Deref for DMGPaletteRegister {
    type Target = [DMGColor; 4];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for DMGPaletteRegister {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl DMGPaletteRegister {
    pub fn set_from_byte(&mut self, value: u8) {
        self.0 = [
            DMGColor::from_repr(value & 0x03).unwrap_or(DMGColor::White),
            DMGColor::from_repr((value >> 2) & 0x03).unwrap_or(DMGColor::White),
            DMGColor::from_repr((value >> 4) & 0x03).unwrap_or(DMGColor::White),
            DMGColor::from_repr((value >> 6) & 0x03).unwrap_or(DMGColor::White),
        ];
    }

    pub fn to_byte(&self) -> u8 {
        self[0] as u8
            | ((self[1] as u8) << 2)
            | ((self[2] as u8) << 4)
            | ((self[3] as u8) << 6)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Decode, Encode)]
pub struct LcdPalette {
    background: DMGPaletteRegister,
    object0: DMGPaletteRegister,
    object1: DMGPaletteRegister,
}

impl LcdPalette {
    pub fn background(&self) -> &DMGPaletteRegister {
        &self.background
    }

    pub fn background_mut(&mut self) -> &mut DMGPaletteRegister {
        &mut self.background
    }

    pub fn object0(&self) -> &DMGPaletteRegister {
        &self.object0
    }

    pub fn object0_mut(&mut self) -> &mut DMGPaletteRegister {
        &mut self.object0
    }

    pub fn object1(&self) -> &DMGPaletteRegister {
        &self.object1
    }

    pub fn object1_mut(&mut self) -> &mut DMGPaletteRegister {
        &mut self.object1
    }
}