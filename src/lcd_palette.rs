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
        match self {
            DMGColor::White => Rgb([0xFF, 0xFF, 0xFF]),      // Pure white
            DMGColor::LightGray => Rgb([0xAA, 0xAA, 0xAA]),  // Light gray
            DMGColor::DarkGray => Rgb([0x55, 0x55, 0x55]),      // Dark gray
            DMGColor::Black => Rgb([0x00, 0x00, 0x00]),            // Pure black
        }
    }
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