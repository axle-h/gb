//! The `screenshot` tool's payload: one published LCD frame as a `data:` URL.

use base64::Engine;

use gb::ppu::{LCD_HEIGHT, LCD_WIDTH};
use crate::frame::Frame;

/// Nearest-neighbour upscale before encoding.
pub const SCALE: usize = 3;

/// The frame as a `data:image/png;base64,…` URL, ready to go straight into an `image_url` part.
pub fn data_url(frame: &Frame) -> String {
    png_data_url(&encode(frame))
}

/// Wrap already-encoded PNG bytes as a `data:` URL.
pub fn png_data_url(png: &[u8]) -> String {
    let mut url = String::with_capacity(png.len() * 4 / 3 + 32);
    url.push_str("data:image/png;base64,");
    base64::engine::general_purpose::STANDARD.encode_string(png, &mut url);
    url
}

/// The caption that rides beside the picture. A model shown an unlabelled image of a Game Boy
/// screen has to work out what it is looking at; one line of prose is cheaper than the inference.
pub fn caption(seq: u64) -> String {
    format!(
        "Screenshot of the Game Boy screen as it is right now (frame {seq}), {LCD_WIDTH}×{LCD_HEIGHT} \
         upscaled {SCALE}× with no smoothing. The game is still running — by the time you read this \
         it has moved on a little."
    )
}

pub fn encode(frame: &Frame) -> Vec<u8> {
    let mut image =
        image::RgbImage::new((LCD_WIDTH * SCALE) as u32, (LCD_HEIGHT * SCALE) as u32);
    for y in 0..LCD_HEIGHT {
        for x in 0..LCD_WIDTH {
            let colour = frame[y * LCD_WIDTH + x].to_rgb();
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    image.put_pixel((x * SCALE + dx) as u32, (y * SCALE + dy) as u32, colour);
                }
            }
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("a 480×432 image encodes to PNG in memory");
    png.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb::lcd_palette::LcdColor;

    /// The URL has to be something an endpoint will accept verbatim, and the pixels inside it
    /// have to be the frame's own — an off-by-one in the upscale is invisible in a thumbnail and
    /// shows up as a model confidently misreading the screen.
    #[test]
    fn a_frame_becomes_a_data_url_holding_the_same_picture() {
        let mut frame: Box<Frame> = Box::new([LcdColor::WHITE; LCD_WIDTH * LCD_HEIGHT]);
        let ink = LcdColor::rgb(0x11, 0x22, 0x33);
        frame[0] = ink;
        frame[LCD_WIDTH * LCD_HEIGHT - 1] = ink;

        let url = data_url(&frame);
        let payload = url
            .strip_prefix("data:image/png;base64,")
            .expect("an image_url part needs exactly this prefix");
        let png = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .expect("we just encoded this");
        let decoded = image::load_from_memory(&png).expect("a PNG we produced").to_rgb8();

        assert_eq!(decoded.dimensions(), ((LCD_WIDTH * SCALE) as u32, (LCD_HEIGHT * SCALE) as u32));
        // Every pixel of the top-left source pixel's block, and none of its neighbour.
        for dy in 0..SCALE {
            for dx in 0..SCALE {
                assert_eq!(*decoded.get_pixel(dx as u32, dy as u32), ink.to_rgb(), "({dx},{dy})");
            }
        }
        assert_eq!(*decoded.get_pixel(SCALE as u32, 0), LcdColor::WHITE.to_rgb());
        let (w, h) = decoded.dimensions();
        assert_eq!(*decoded.get_pixel(w - 1, h - 1), ink.to_rgb(), "the last source pixel is the last block");
    }
}
