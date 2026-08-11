//! The two PNG endpoints that are neither the badge sheet nor a built asset: a Pokémon's front
//! sprite, and the favicon.
//!
//! Both come out of the cartridge at run time, the same way [`crate::web::badges`] does, and both
//! are functions of the ROM — so both are `immutable` and encoded exactly once.
//!
//! ⚠️ **`/api/pokemon/{dex}/front.png`, not `/api/pokemon/{dex}.png`.** axum's router (matchit) says
//! it plainly: "Dynamic suffixes are not currently supported" — a parameter has to own its whole
//! path segment. The shorter form fails when the router is *built*, i.e. at startup, so it would
//! take the server down rather than 404.

use std::sync::OnceLock;

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use strum::IntoEnumIterator;

use crate::pokemon::mon_gfx::{PIC_PX, front_pic_shades};
use crate::pokemon::rom_gfx::{BALL_PX, poke_ball_shades};
use crate::pokemon::species::PokemonSpecies;

pub const SPECIES_COUNT: usize = 151;

/// The favicon is drawn at 2× so it stays crisp on a hidpi tab strip; 16 px doubled is 32.
const FAVICON_SCALE: usize = 2;

/// Tone-**inverted**, as RGBA, for the same reason [`crate::web::badges`] inverts the badges: this
/// page is dark by decision, and a Gen 1 pic is black line art over a white body on a white
/// background. Left alone it is a white box; with its background merely made transparent it is a
/// black outline on a near-black panel, i.e. invisible.
///
/// So shade 3 — the outline — comes out brightest, and shade 0 — the body's own fill — becomes the
/// darkest *visible* tone rather than nothing. Which is the point of the flood fill below: only the
/// shade-0 pixels that the border can reach are background. An interior shade 0 that went
/// transparent too would leave the sprite hollow, showing the panel through the middle of the
/// Pokémon.
const INK: [[u8; 4]; 4] = [
    [0x2E, 0x34, 0x40, 0xFF],
    [0x6B, 0x74, 0x85, 0xFF],
    [0xB4, 0xBC, 0xC8, 0xFF],
    [0xF2, 0xF5, 0xF9, 0xFF],
];

const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

/// The Poké Ball keeps the Game Boy's own tones. It is the browser's tab strip rather than this
/// page, and that is light on some machines and dark on others — the ball has a white fill *and* a
/// black outline, so as drawn it reads on both. Inverting it would cost one of the two.
const BALL_INK: [[u8; 4]; 4] = [
    TRANSPARENT,
    [0xF2, 0xF5, 0xF9, 0xFF],
    [0x7D, 0x83, 0x8F, 0xFF],
    [0x11, 0x13, 0x18, 0xFF],
];

// ── The party sprites ────────────────────────────────────────────────────────────────────────────

/// `GET /api/pokemon/{dex}/front.png` — one 56×56 sprite, keyed on the National Pokédex number
/// because that is the id a viewer recognises and the one the status heartbeat carries.
pub async fn front_pic(Path(dex): Path<u16>) -> Response {
    match sprites().get(dex.wrapping_sub(1) as usize) {
        Some(png) => png_response(png).into_response(),
        // Dex numbers are 1..=151 and nothing else on this cartridge is a Pokémon.
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Every front sprite, encoded once, in Pokédex order.
///
/// All 151 together, rather than a per-species cache behind a lock: decompressing and encoding the
/// lot costs a few hundred kilobytes and a few tens of milliseconds *once*, and a party is six of
/// them within a second of each other anyway. `badges::sheet()` makes the same trade for the same
/// reason — the ROM does not change while the process is running.
fn sprites() -> &'static [Vec<u8>] {
    static SPRITES: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    SPRITES.get_or_init(|| {
        let mut by_dex = vec![Vec::new(); SPECIES_COUNT];
        for species in PokemonSpecies::iter() {
            by_dex[species.metadata().pokedex_number as usize - 1] = encode_front_pic(species);
        }
        by_dex
    })
}

fn encode_front_pic(species: PokemonSpecies) -> Vec<u8> {
    let shades = front_pic_shades(species);
    let background = background_mask(&shades);
    let mut image = image::RgbaImage::new(PIC_PX as u32, PIC_PX as u32);
    for y in 0..PIC_PX {
        for x in 0..PIC_PX {
            let at = y * PIC_PX + x;
            let colour = if background[at] { TRANSPARENT } else { INK[shades[at] as usize] };
            image.put_pixel(x as u32, y as u32, image::Rgba(colour));
        }
    }
    encode(&image)
}

/// Which pixels are *behind* the Pokémon rather than part of it: shade 0, and reachable from the
/// edge of the canvas through other shade-0 pixels.
///
/// ⚠️ **Four-way, not eight-way.** A diagonal step leaks through the single-pixel gap left by any
/// outline drawn on the diagonal, and the background floods the body — which shows up as one or two
/// Pokémon out of 151 rendered as an outline, so it is not something a spot check finds.
fn background_mask(shades: &[u8; PIC_PX * PIC_PX]) -> Vec<bool> {
    let mut background = vec![false; PIC_PX * PIC_PX];
    let mut queue = Vec::new();
    let visit = |x: usize, y: usize, background: &mut Vec<bool>, queue: &mut Vec<(usize, usize)>| {
        let at = y * PIC_PX + x;
        if shades[at] == 0 && !background[at] {
            background[at] = true;
            queue.push((x, y));
        }
    };
    for edge in 0..PIC_PX {
        visit(edge, 0, &mut background, &mut queue);
        visit(edge, PIC_PX - 1, &mut background, &mut queue);
        visit(0, edge, &mut background, &mut queue);
        visit(PIC_PX - 1, edge, &mut background, &mut queue);
    }
    while let Some((x, y)) = queue.pop() {
        if x > 0 {
            visit(x - 1, y, &mut background, &mut queue);
        }
        if x + 1 < PIC_PX {
            visit(x + 1, y, &mut background, &mut queue);
        }
        if y > 0 {
            visit(x, y - 1, &mut background, &mut queue);
        }
        if y + 1 < PIC_PX {
            visit(x, y + 1, &mut background, &mut queue);
        }
    }
    background
}

// ── The favicon ──────────────────────────────────────────────────────────────────────────────────

/// `GET /favicon.png` and `GET /favicon.ico` — the overworld Poké Ball, the sprite an item lying on
/// the floor is drawn with.
///
/// Both paths serve the same PNG bytes. Browsers that get the `<link rel="icon">` in `index.html`
/// ask for the first; anything that asks by convention alone — a bookmark, a feed reader, a scanner
/// — asks for the second, and would otherwise fall through to the SPA catch-all's 404.
pub async fn favicon() -> Response {
    png_response(icon()).into_response()
}

fn icon() -> &'static [u8] {
    static ICON: OnceLock<Vec<u8>> = OnceLock::new();
    ICON.get_or_init(|| {
        let shades = poke_ball_shades();
        let size = (BALL_PX * FAVICON_SCALE) as u32;
        let mut image = image::RgbaImage::new(size, size);
        for y in 0..size as usize {
            for x in 0..size as usize {
                let shade = shades[(y / FAVICON_SCALE) * BALL_PX + x / FAVICON_SCALE];
                image.put_pixel(x as u32, y as u32, image::Rgba(BALL_INK[shade as usize]));
            }
        }
        encode(&image)
    })
}

// ── Shared ───────────────────────────────────────────────────────────────────────────────────────

fn encode(image: &image::RgbaImage) -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).expect("a small image encodes to PNG in memory");
    png.into_inner()
}

/// Immutable: every one of these is a function of the cartridge, so a viewer may cache it for as
/// long as it likes.
fn png_response(png: &'static [u8]) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        png,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(png: &[u8]) -> image::RgbaImage {
        image::load_from_memory(png).expect("we just encoded this").to_rgba8()
    }

    #[test]
    fn every_species_encodes_to_a_distinct_transparent_backed_png() {
        let sprites = sprites();
        assert_eq!(sprites.len(), SPECIES_COUNT);

        let images: Vec<_> = sprites.iter().map(|png| decoded(png)).collect();
        for (index, image) in images.iter().enumerate() {
            let dex = index + 1;
            assert_eq!(image.dimensions(), (PIC_PX as u32, PIC_PX as u32), "#{dex}");
            assert!(image.pixels().any(|p| p.0[3] == 0xFF), "#{dex} is entirely transparent");
            assert!(image.pixels().any(|p| p.0[3] == 0), "#{dex} has no transparent background");
        }

        for (index, first) in images.iter().enumerate() {
            for (other, second) in images.iter().enumerate().skip(index + 1) {
                assert_ne!(first, second, "#{} and #{} are the same image", index + 1, other + 1);
            }
        }
    }

    /// The whole point of flood-filling from the border rather than simply calling shade 0
    /// transparent: a Pokémon with a white belly must have a belly, not a hole.
    ///
    /// It is not a corner case. **Every one of the 151** has shade-0 pixels inside itself — Gen 1
    /// art fills a body with the background tone and relies on the outline to bound it — so the
    /// naïve rule would render the entire Pokédex as wireframes.
    #[test]
    fn the_flood_fill_keeps_every_bodys_own_white_opaque() {
        for species in PokemonSpecies::iter() {
            let shades = front_pic_shades(species);
            let background = background_mask(&shades);
            let interior = (0..PIC_PX * PIC_PX).filter(|&at| shades[at] == 0 && !background[at]).count();
            assert!(interior > 0, "{species} has no shade-0 pixel the border cannot reach");
        }
    }

    /// The other half: the fill has to *reach* the surround. A mask that found nothing would pass
    /// the test above trivially, and every sprite would be a 56×56 opaque block.
    #[test]
    fn the_background_is_actually_found() {
        let smallest = sprites()
            .iter()
            .map(|png| decoded(png).pixels().filter(|p| p.0[3] == 0).count())
            .min()
            .expect("151 sprites");
        // The emptiest of them (Slowbro, which fills its box) leaves 579 of 3136 pixels behind it.
        assert!(smallest > 400, "the emptiest sprite has only {smallest} transparent pixels of {}", PIC_PX * PIC_PX);
    }

    #[test]
    fn the_favicon_is_a_thirty_two_pixel_ball() {
        let image = decoded(icon());
        assert_eq!(image.dimensions(), (32, 32));
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "the corners are outside the ball");
        assert!(image.pixels().any(|p| p.0[3] == 0xFF), "the ball is entirely transparent");
        // Both ends of the ramp are present, which is what lets it read on a light *and* a dark tab
        // strip. Inverting it, or dropping to two tones, would lose one of them.
        assert!(image.pixels().any(|p| p.0 == BALL_INK[1]), "no white fill");
        assert!(image.pixels().any(|p| p.0 == BALL_INK[3]), "no dark outline");
    }

    #[test]
    fn everything_is_encoded_once() {
        assert!(std::ptr::eq(icon(), icon()), "the favicon should be cached, not re-encoded");
        assert!(std::ptr::eq(sprites(), sprites()), "the sprites should be cached, not re-encoded");
    }
}

