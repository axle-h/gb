//! **W5 / §8** — the `screenshot` tool's payload: one published LCD frame as a `data:` URL.
//!
//! ⚠️ **This runs on the worker thread, never on the emulator's.** The frame is already copied out
//! of the `GameBoy` by [`EmulatorHost::publish_video`](crate::host::EmulatorHost) 30 times a second
//! and parked in [`Published`](crate::web::published::Published); encoding it costs a millisecond or
//! so, and a millisecond spent here is a millisecond the livestream is not being emulated. That is
//! also why `screenshot` is the one read tool the policy never sees: it is answered from a buffer
//! the worker can already reach, with no round trip through `service_tools` at all.
//!
//! The picture is what the *viewer* sees, not what the agent reads. Everything the agent reads —
//! the map grid, the party, the text on screen — is available as structured JSON for a fraction of
//! the tokens, so this is for the cases where the model wants to check the game with its own eyes: a
//! menu the agent does not model, an animation it is not sure has finished, a screen that looks
//! wrong.

use base64::Engine;

use crate::ppu::{LCD_HEIGHT, LCD_WIDTH};
use crate::web::video::Frame;

/// Nearest-neighbour upscale before encoding.
///
/// 3× is 480×432, which is the largest whole multiple that still fits inside the 512×512 box a
/// `detail: "low"` image is fitted to — so the endpoint resizes nothing and the model sees whole
/// pixels rather than a resampler's guess at them. It costs nothing on the wire: a four-shade image
/// scaled by an integer is almost pure PNG filter runs, and the encoded result is a couple of
/// kilobytes either way.
pub const SCALE: usize = 3;

/// The frame as a `data:image/png;base64,…` URL, ready to go straight into an `image_url` part.
pub fn data_url(frame: &Frame) -> String {
    png_data_url(&encode(frame))
}

/// Wrap already-encoded PNG bytes as a `data:` URL.
///
/// Split out because the same picture now has two destinations — the model's message, and the ring
/// in [`Published`](crate::web::published::Published) the page fetches it from — and encoding it
/// twice to serve both would be a second PNG compression per tool call.
pub fn png_data_url(png: &[u8]) -> String {
    let mut url = String::with_capacity(png.len() * 4 / 3 + 32);
    url.push_str("data:image/png;base64,");
    base64::engine::general_purpose::STANDARD.encode_string(png, &mut url);
    url
}

/// The caption that rides beside the picture. A model shown an unlabelled image of a Game Boy screen
/// has to work out what it is looking at; one line of prose is cheaper than the inference.
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
    use crate::lcd_palette::LcdColor;

    /// The URL has to be something an endpoint will accept verbatim, and the pixels inside it have to
    /// be the frame's own — an off-by-one in the upscale is invisible in a thumbnail and shows up as
    /// a model confidently misreading the screen.
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
