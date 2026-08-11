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

/// The Game Boy's own four shades, as RGBA: 0 is the white a body is filled with, 3 the black it is
/// outlined in.
///
/// ⚠️ **Do not invert this the way [`crate::web::badges`] inverts the badges.** It shipped inverted
/// once, on the argument that a black outline would vanish against a near-black panel. That argument
/// is wrong, and the sprites say so: an outline is *bounded by the body's own bright fill*, so it
/// reads at full contrast — only the outermost contour meets the panel, which just lets the
/// silhouette sit on the page. What inverting actually does is turn every Pokémon into a
/// photographic negative of itself, which for filled art is not a palette choice but a different
/// picture: Gengar came out white-bodied with a dark grin.
///
/// A badge inverts well because it *is* line art — there is no fill to negate. That is the whole of
/// the difference between the two modules, and it is why the favicon (the Poké Ball, below) keeps
/// its tones too.
///
/// Which makes the flood fill below load-bearing rather than a refinement: shade 0 is now opaque
/// white, so a sprite whose background was not found would be a solid white block.
const INK: [[u8; 4]; 4] = [
    [0xF2, 0xF5, 0xF9, 0xFF],
    [0xB4, 0xBC, 0xC8, 0xFF],
    [0x6B, 0x74, 0x85, 0xFF],
    [0x11, 0x13, 0x18, 0xFF],
];

const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

/// The Poké Ball, whose only difference from [`INK`] is that shade 0 is its background rather than
/// part of it — an overworld sprite is drawn with a transparent colour 0, so there is nothing to
/// flood-fill. It lands on the browser's tab strip rather than on this page, and that is light on
/// some machines and dark on others; the ball has a white fill *and* a black outline, so as drawn it
/// reads on both.
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

    /// ⚠️ **The direction of the ramp, pinned.** It shipped inverted once — a defensible-sounding
    /// change (the badges next door do exactly that) which quietly turns every Pokémon into a
    /// negative of itself. Nothing else here would notice: the sprites stay distinct, opaque and
    /// transparent in all the right places whichever way round the four colours go.
    #[test]
    fn the_ramp_is_not_inverted() {
        let luminance = |c: [u8; 4]| c[0] as u32 * 2 + c[1] as u32 * 3 + c[2] as u32;
        for ramp in [INK, BALL_INK] {
            assert!(
                luminance(ramp[1]) > luminance(ramp[2]) && luminance(ramp[2]) > luminance(ramp[3]),
                "shades 1-3 must darken, as they do on the cartridge: {ramp:?}",
            );
        }
        // …and shade 0, which is a body's white fill here and transparent on an overworld sprite.
        assert!(luminance(INK[0]) > luminance(INK[1]), "shade 0 is the lightest, not the darkest");
        assert_eq!(BALL_INK[0], TRANSPARENT);
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

