//! What the four shades of a composed frame are painted with, and the one piece of the graphics
//! engine a player can switch at run time.
//!
//! Input: a [`Screen`], whose composed [`Framebuffer`] has already been through `BGP`, `OBP0` and
//! `OBP1`. Output: RGB, three bytes a pixel, of [`ColourMode::size`].
//!
//! Exact: the DMG ramp `gb` puts on the LCD; the CGB's compatibility mapping, which takes a shade
//! back through the register that produced it and into CGB palette 0 for the background and
//! palette 0 or 1 for objects; the SGB's per-cell palettes, which are [`crate::gfx::sgb`]'s.
//!
//! Nothing here is state: a mode is a choice made at the moment of painting, so a host can offer
//! all four and switch between them mid-frame.

use serde::{Deserialize, Serialize};
use crate::gfx::compose::{Framebuffer, HEIGHT, WIDTH};
use crate::gfx::sgb::border;
use crate::gfx::Screen;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColourMode {
    #[default]
    Dmg,
    /// Pokémon Red on a Game Boy Color, which is the DMG picture through the boot ROM's
    /// title-derived palette. There is no CGB code in the cartridge: the red tint is the console's.
    Gbc,
    Sgb,
    SgbBorder,
}

/// Which palette register a composed pixel came through. The CGB's compatibility mode gives the
/// three of them different colours; the SGB cannot tell them apart, because what it sees is the
/// picture the DMG finished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Background,
    Object0,
    Object1,
}

/// The shades as they reach the LCD, which is the ramp every committed screenshot depends on.
const DMG: [[u8; 3]; 4] = [[0xFF; 3], [0xAA; 3], [0x55; 3], [0x00; 3]];

/// What a CGB boot ROM installs for `POKEMON RED`: combination 13 of `gb::boot_palette`, which is
/// `palette_comb 3, 4, 4`, so the background and the `OBP1` objects share the red ramp and the
/// `OBP0` objects are green. A lockstep pins these against the emulator's own tables.
pub const GBC_BACKGROUND: [u16; 4] = [0x7FFF, 0x421F, 0x1CF2, 0x0000];
pub const GBC_OBJECT0: [u16; 4] = [0x7FFF, 0x1BEF, 0x0200, 0x0000];
pub const GBC_OBJECT1: [u16; 4] = GBC_BACKGROUND;

impl ColourMode {
    /// The picture this mode paints. Only the border is bigger than the screen.
    pub const fn size(self) -> (usize, usize) {
        match self {
            ColourMode::SgbBorder => (border::WIDTH, border::HEIGHT),
            _ => (WIDTH, HEIGHT),
        }
    }

    /// RGB, three bytes a pixel, row by row.
    pub fn rgb(self, screen: &Screen) -> Vec<u8> {
        let frame = screen.frame();
        match self {
            ColourMode::Dmg => frame.shades.iter().flat_map(|&shade| DMG[shade as usize]).collect(),
            ColourMode::Gbc => gbc(&frame),
            ColourMode::Sgb => sgb(screen, &frame),
            ColourMode::SgbBorder => border::rgb(&sgb(screen, &frame)),
        }
    }
}

fn gbc(frame: &Framebuffer) -> Vec<u8> {
    frame.shades.iter().zip(&frame.sources)
        .flat_map(|(&shade, source)| {
            let palette = match source {
                Source::Background => GBC_BACKGROUND,
                Source::Object0 => GBC_OBJECT0,
                Source::Object1 => GBC_OBJECT1,
            };
            rgb555(palette[shade as usize])
        })
        .collect()
}

fn sgb(screen: &Screen, frame: &Framebuffer) -> Vec<u8> {
    let mut out = Vec::with_capacity(WIDTH * HEIGHT * 3);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let palette = screen.sgb.cell_palette(x / 8, y / 8);
            out.extend_from_slice(&rgb555(palette[frame.shade(x, y) as usize]));
        }
    }
    out
}

/// A `0bBBBBBGGGGGRRRRR` colour widened to 24 bits the way the hardware does, each channel's top
/// bits replicated into its low ones.
pub(crate) fn rgb555(colour: u16) -> [u8; 3] {
    let expand = |channel: u16| ((channel << 3) | (channel >> 2)) as u8;
    [expand(colour & 0x1F), expand((colour >> 5) & 0x1F), expand((colour >> 10) & 0x1F)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::sgb::{super_palette, PaletteCommand, PAL_BLACK, PAL_GRAYMON};
    use crate::systems::hp_bar::HpBarColour;

    #[test]
    fn a_colour_widens_the_way_the_hardware_widens_it() {
        assert_eq!(rgb555(0x7FFF), [0xFF; 3]);
        assert_eq!(rgb555(0x0000), [0x00; 3]);
        assert_eq!(rgb555(0x421F), [0xFF, 0x84, 0x84], "the boot palette's salmon");
    }

    #[test]
    fn only_the_border_is_bigger_than_the_screen() {
        assert_eq!(ColourMode::Dmg.size(), (WIDTH, HEIGHT));
        assert_eq!(ColourMode::Sgb.size(), (WIDTH, HEIGHT));
        assert_eq!(ColourMode::SgbBorder.size(), (256, 224));
    }

    /// A blank screen is shade 0 everywhere, which every mode paints with its own white.
    #[test]
    fn each_mode_paints_the_whole_screen_it_says_it_will() {
        let screen = Screen::default();
        for mode in [ColourMode::Dmg, ColourMode::Gbc, ColourMode::Sgb, ColourMode::SgbBorder] {
            let (width, height) = mode.size();
            let rgb = mode.rgb(&screen);
            assert_eq!(rgb.len(), width * height * 3, "{mode:?}");
        }
        assert_eq!(&ColourMode::Dmg.rgb(&screen)[..3], &[0xFF; 3]);
        assert_eq!(&ColourMode::Gbc.rgb(&screen)[..3], &[0xFF; 3]);
        assert_eq!(&ColourMode::Sgb.rgb(&screen)[..3], &[0xFF, 0xEF, 0xFF], "31,29,31");
    }

    /// The SGB colours the screen rather than the game's layers, so what a cell's palette is
    /// applies to whatever stands in it.
    #[test]
    fn an_sgb_cell_is_painted_by_its_own_palette() {
        let mut screen = Screen::default();
        // Every background pixel the darkest shade, so what is left to see is the cell's palette.
        screen.effects.bgp = 0xFF;
        screen.sgb.run(&PaletteCommand::Battle {
            player_hp_bar: HpBarColour::Green,
            enemy_hp_bar: HpBarColour::Green,
            player: PAL_BLACK,
            enemy: PAL_GRAYMON,
        });
        let rgb = ColourMode::Sgb.rgb(&screen);
        let at = |x: usize, y: usize| {
            let i = (y * WIDTH + x) * 3;
            [rgb[i], rgb[i + 1], rgb[i + 2]]
        };
        assert_eq!(at(4 * 8, 8 * 8), rgb555(super_palette(PAL_BLACK)[3]), "the player's mon");
        assert_eq!(at(15 * 8, 3 * 8), rgb555(super_palette(PAL_GRAYMON)[3]), "the enemy's");
    }
}
