use serde::{Deserialize, Serialize};
use crate::gfx::compose::Framebuffer;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColourMode {
    #[default]
    Dmg,
}

impl ColourMode {
    /// RGB, three bytes a pixel.
    pub fn rgb(self, frame: &Framebuffer) -> Vec<u8> {
        const DMG: [[u8; 3]; 4] = [[0xFF; 3], [0xAA; 3], [0x55; 3], [0x00; 3]];
        match self {
            ColourMode::Dmg => frame.shades.iter().flat_map(|&shade| DMG[shade as usize]).collect(),
        }
    }
}
