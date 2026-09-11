//! The badge sprite sheet, `GET /api/badges.png`.

use std::sync::OnceLock;

use axum::http::header;
use axum::response::{IntoResponse, Response};

use poke_agent::pokemon::badge_gfx::{BADGE_COUNT, BADGE_PX, badge_shades};

/// Tone-inverted, as RGBA.
const SHADES: [[u8; 4]; 4] = [
    [0x00, 0x00, 0x00, 0x00],
    [0x8B, 0x94, 0xA2, 0xFF],
    [0xC6, 0xCC, 0xD6, 0xFF],
    [0xFF, 0xFF, 0xFF, 0xFF],
];

/// The sheet, encoded once: the ROM does not change while the process runs.
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

/// `GET /api/badges.png`. Immutable: the sheet is a function of the cartridge.
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
