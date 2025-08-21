use std::collections::BTreeMap;
use sdl2::render::{BlendMode, Texture, TextureCreator, TextureQuery, WindowCanvas};
use fontdue::{Font, FontSettings};
use fontdue::layout::{Layout, LayoutSettings, TextStyle};
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::video::WindowContext;

pub struct FontTextures<'a> {
    layout: Layout,
    fonts: Vec<Font>,
    glyphs: BTreeMap<char, (Texture<'a>, TextureQuery)>,
    size: f32
}

impl<'a> FontTextures<'a> {
    const GLYPHS: &'static str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789,./;:'\"[]{}\\|`~!@#$%^&*()-_=+<>?";

    pub fn new(texture_creator: &'a TextureCreator<WindowContext>, font: Font, size: f32, color: Color) -> Result<Self, String> {
        let mut glyphs = BTreeMap::new();
        for char in Self::GLYPHS.chars() {
            let (metrics, bitmap) = font.rasterize(char, size);
            let mut texture = texture_creator.create_texture_streaming(
                PixelFormatEnum::RGBA8888,
                metrics.width as u32,
                metrics.height as u32
            ).map_err(|e| e.to_string())?;
            texture.set_blend_mode(BlendMode::Blend);

            texture.with_lock(None, |buffer: &mut [u8], pitch: usize| {
                // Clear the entire texture first
                for i in 0..buffer.len() {
                    buffer[i] = 0;
                }

                // Copy glyph data at the correct vertical position
                for y in 0..metrics.height {
                    for x in 0..metrics.width {
                        let src_idx = y * metrics.width + x;
                        let coverage = bitmap[src_idx];
                        let scaled_color = if coverage > 0 {
                            let scale = coverage as f32 / 255.0;
                            Color::RGBA(
                                (color.r as f32 * scale).round() as u8,
                                (color.g as f32 * scale).round() as u8,
                                (color.b as f32 * scale).round() as u8,
                                color.a
                            )
                        } else {
                            Color::RGBA(0, 0, 0, 0)
                        };

                        let dest_idx = (y * (pitch / 4) + x) * 4;
                        buffer[dest_idx] = scaled_color.r;
                        buffer[dest_idx + 1] = scaled_color.g;
                        buffer[dest_idx + 2] = scaled_color.b;
                        buffer[dest_idx + 3] = scaled_color.a;
                    }
                }
            }).map_err(|e| e.to_string())?;
            let query = texture.query();
            glyphs.insert(char, (texture, query));
        }
        let layout = Layout::new(fontdue::layout::CoordinateSystem::PositiveYDown);
        Ok(Self { glyphs, layout, fonts: vec![font], size })
    }

    pub fn roboto_regular(texture_creator: &'a TextureCreator<WindowContext>, size: f32, color: Color) -> Result<Self, String> {
        let font = Font::from_bytes(include_bytes!("./Roboto-Regular.ttf").to_vec(), FontSettings::default()).map_err(|e| e.to_string())?;
        Self::new(texture_creator, font, size, color)
    }

    pub fn render_text(&mut self, canvas: &mut WindowCanvas, text: &str, x: i32, y: i32) -> Result<(), String> {
        if text.is_empty() {
            return Ok(()); // Nothing to render
        }

        self.layout.clear();
        self.layout.append(&self.fonts, &TextStyle::new(text, self.size, 0));

        for glyph in self.layout.glyphs() {
            if let Some((texture, query)) = self.glyphs.get(&glyph.parent) {
                canvas.copy(texture, None, Some(Rect::new(x + glyph.x as i32, y + glyph.y as i32, glyph.width as u32, glyph.height as u32)))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

}