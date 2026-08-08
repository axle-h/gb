//! **W3** — the badge sprite sheet, `GET /api/badges.png`.
//!
//! Eight 16×16 badges side by side, 128×16, decoded out of the cartridge by
//! [`crate::pokemon::badge_gfx`] the first time the endpoint is asked for. Nothing is committed and
//! nothing is read from disk: the art comes from the same ROM bytes the emulator boots.
//!
//! **The four shades are inverted on the way out**, and that is a decision rather than an accident.
//! On the trainer card a badge is dark line art on a *white* background; dropped onto this page's
//! near-black panel it would be either a bright white chip or — if the background were simply made
//! transparent — a black outline on black, i.e. invisible. Inverting the tone ramp and making the
//! background transparent gives light line art on a dark page, which is what the rest of the UI
//! looks like. The page is dark by decision (`color-scheme: dark`), so there is no second theme this
//! has to serve.
//!
//! Earned vs. not is the client's business — same sheet, less opacity.

use std::sync::OnceLock;

use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::pokemon::badge_gfx::{BADGE_COUNT, BADGE_PX, badge_shades};

/// Tone-inverted, as RGBA. Index 0 is the badge's background and goes fully transparent; 3 is its
/// outline and comes out brightest. The three visible levels are the page's own greys.
const SHADES: [[u8; 4]; 4] = [
    [0x00, 0x00, 0x00, 0x00],
    [0x8B, 0x94, 0xA2, 0xFF],
    [0xC6, 0xCC, 0xD6, 0xFF],
    [0xFF, 0xFF, 0xFF, 0xFF],
];

/// The sheet, encoded once. A few kilobytes, immutable for the life of the process — the ROM does
/// not change while it is running.
pub fn sheet() -> &'static [u8] {
    static SHEET: OnceLock<Vec<u8>> = OnceLock::new();
    SHEET.get_or_init(encode)
}

fn encode() -> Vec<u8> {
    let mut image = image::RgbaImage::new((BADGE_PX * BADGE_COUNT) as u32, BADGE_PX as u32);
    for badge in 0..BADGE_COUNT {
        let shades = badge_shades(badge);
        for y in 0..BADGE_PX {
            for x in 0..BADGE_PX {
                let colour = SHADES[shades[y * BADGE_PX + x] as usize];
                image.put_pixel((badge * BADGE_PX + x) as u32, y as u32, image::Rgba(colour));
            }
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("a 128×16 image encodes to PNG in memory");
    png.into_inner()
}

/// `GET /api/badges.png`. Immutable: the sheet is a function of the cartridge, so a viewer may cache
/// it for as long as it likes.
pub async fn badges() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        sheet(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sheet is what the UI slices by `background-position`, so its geometry is a contract: 8
    /// badges of `BADGE_PX`, in bit order, left to right.
    #[test]
    fn the_sheet_is_eight_distinct_badges_side_by_side() {
        let decoded = image::load_from_memory(sheet()).expect("we just encoded this").to_rgba8();
        assert_eq!(decoded.dimensions(), ((BADGE_PX * BADGE_COUNT) as u32, BADGE_PX as u32));

        let sprites: Vec<Vec<[u8; 4]>> = (0..BADGE_COUNT)
            .map(|badge| {
                let mut pixels = Vec::with_capacity(BADGE_PX * BADGE_PX);
                for y in 0..BADGE_PX {
                    for x in 0..BADGE_PX {
                        pixels.push(decoded.get_pixel((badge * BADGE_PX + x) as u32, y as u32).0);
                    }
                }
                pixels
            })
            .collect();

        for (badge, pixels) in sprites.iter().enumerate() {
            assert!(
                pixels.iter().any(|p| p[3] == 0),
                "badge {badge} has no transparent background — it would render as a filled block",
            );
            assert!(pixels.iter().any(|p| p[3] == 0xFF), "badge {badge} is entirely transparent");
            for (other, second) in sprites.iter().enumerate().skip(badge + 1) {
                assert_ne!(pixels, second, "badges {badge} and {other} are the same sprite");
            }
        }
    }

    #[test]
    fn the_sheet_is_encoded_once() {
        assert!(std::ptr::eq(sheet(), sheet()), "the sheet should be cached, not re-encoded");
    }
}
