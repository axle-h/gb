//! Text drawn into [`Pixels`]: one line at a time, clipped to a rectangle, each glyph rasterised the
//! first time it is drawn.

use std::collections::HashMap;
use fontdue::{Font, FontSettings, Metrics};
use crate::sdl::pixels::{Pixels, Rect, Rgb};

pub struct Text {
    /// Searched in order for a glyph, so the first that has one draws it.
    fonts: Vec<Font>,
    size: f32,
    ascent: f32,
    line_height: u32,
    /// `None` for a character no font has, an emoji in an event say, which is left out rather than
    /// drawn as the missing-glyph box.
    glyphs: HashMap<char, Option<(Metrics, Vec<u8>)>>,
}

impl Text {
    pub fn roboto(size: f32) -> Result<Self, String> {
        let roboto = Font::from_bytes(include_bytes!("./Roboto-Regular.ttf").as_slice(), FontSettings::default())?;
        let lines = roboto.horizontal_line_metrics(size).ok_or("Roboto has no horizontal metrics")?;
        // Roboto has no →, ✓, ✗, ♂ or ♀; this is Adwaita Mono cut down to those five.
        let symbols = Font::from_bytes(include_bytes!("./AdwaitaMono-Symbols.ttf").as_slice(), FontSettings::default())?;
        Ok(Self {
            fonts: vec![roboto, symbols],
            size,
            ascent: lines.ascent,
            line_height: lines.new_line_size.ceil() as u32,
            glyphs: HashMap::new(),
        })
    }

    pub fn line_height(&self) -> u32 {
        self.line_height
    }

    fn glyph(&mut self, c: char) -> Option<&(Metrics, Vec<u8>)> {
        let fonts = &self.fonts;
        let size = self.size;
        self.glyphs.entry(c).or_insert_with(|| {
            fonts.iter().find(|font| font.lookup_glyph_index(c) != 0).map(|font| font.rasterize(c, size))
        }).as_ref()
    }

    /// Whether some font draws `c` rather than the missing-glyph box.
    #[cfg(test)]
    pub fn has_glyph(&self, c: char) -> bool {
        self.fonts.iter().any(|font| font.lookup_glyph_index(c) != 0)
    }

    pub fn width(&mut self, text: &str) -> u32 {
        text.chars().filter_map(|c| self.glyph(c).map(|(metrics, _)| metrics.advance_width)).sum::<f32>().ceil() as u32
    }

    /// `text` with its top-left at (`x`, `y`), drawn only inside `clip`.
    pub fn draw(&mut self, pixels: &mut Pixels, text: &str, x: i32, y: i32, colour: Rgb, clip: Rect) {
        let baseline = y as f32 + self.ascent;
        let mut pen = x as f32;
        for c in text.chars() {
            let Some((metrics, coverage)) = self.glyph(c) else { continue };
            let left = (pen + metrics.xmin as f32).round() as i32;
            let top = (baseline - metrics.height as f32 - metrics.ymin as f32).round() as i32;
            for row in 0..metrics.height {
                for column in 0..metrics.width {
                    let alpha = coverage[row * metrics.width + column];
                    pixels.blend(left + column as i32, top + row as i32, colour, alpha, &clip);
                }
            }
            pen += metrics.advance_width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The characters events and the game's own text use beyond ASCII.
    #[test]
    fn every_character_an_event_uses_has_a_glyph() {
        let text = Text::roboto(16.0).unwrap();
        let missing: String = "→✓✗é♂♀".chars().filter(|&c| !text.has_glyph(c)).collect();
        assert_eq!(missing, "");
    }

    #[test]
    fn a_character_no_font_has_is_left_out() {
        let mut text = Text::roboto(16.0).unwrap();
        assert_eq!(text.width("🏆 won"), text.width(" won"));
        let mut pixels = Pixels::new(64, 32);
        text.draw(&mut pixels, "📖", 0, 0, [255, 255, 255], Rect::new(0, 0, 64, 32));
        assert!(pixels.rgba.chunks(4).all(|pixel| pixel[0] == 0), "no missing-glyph box");
    }

    #[test]
    fn text_is_clipped_to_its_rectangle() {
        let mut text = Text::roboto(16.0).unwrap();
        let mut pixels = Pixels::new(64, 32);
        text.draw(&mut pixels, "MMMMMMMM", 0, 0, [255, 255, 255], Rect::new(0, 0, 10, 32));
        let lit = |x: usize| (0..32).any(|y| pixels.rgba[(y * 64 + x) * 4] != 0);
        assert!((0..10).any(lit));
        assert!(!(10..64).any(lit));
    }
}
