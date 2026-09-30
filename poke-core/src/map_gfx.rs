//! The graphics a map is drawn from: tileset sheets and blocksets, overworld sprite sheets, and
//! the game's own font.

use crate::font::FONT_BYTES;
use crate::gfx::{blocksets, tilesets};
use crate::map_header::TileSetId;
use crate::rom_gfx::{decode_tile, TILE_BYTES};
use crate::sprite::{PictureId, SpriteFacing};
use crate::strings::PokemonString;

pub const TILE_PX: usize = 8;
/// What `LoadTilesetTilePatternData` copies to `vTileset`.
pub const TILESET_TILES: usize = 0x60;
pub const SPRITE_PX: usize = 16;

/// One row of pokered's `Tilesets`, bar the graphics, blocks and collision list it points at.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TilesetEntry {
    /// Counter / "talk over" tile ids; `0xFF` where unused.
    pub talking_over: [u8; 3],
    /// The tile id wild encounters happen on, or `0xFF` for a tileset with no grass.
    pub grass_tile: u8,
    /// `hTileAnimations`: 0 still, 1 water, 2 water and flowers.
    pub animation: u8,
}

pub fn tileset_entry(tileset: TileSetId) -> TilesetEntry {
    let row = crate::tables::TILESETS[tileset as usize];
    TilesetEntry { talking_over: row.counter_tiles, grass_tile: row.grass_tile, animation: row.animation }
}

/// `<Tileset>_GFX` and `<Tileset>_Block`, which `gfx/tilesets.asm` shares between aliases.
fn tileset_files(tileset: TileSetId) -> (&'static [u8], &'static [u8]) {
    use TileSetId::*;
    match tileset {
        Overworld => (tilesets::OVERWORLD, blocksets::OVERWORLD),
        RedsHouse1 | RedsHouse2 => (tilesets::REDS_HOUSE, blocksets::REDS_HOUSE),
        Mart | Pokecenter => (tilesets::POKECENTER, blocksets::POKECENTER),
        Forest => (tilesets::FOREST, blocksets::FOREST),
        Dojo | Gym => (tilesets::GYM, blocksets::GYM),
        House => (tilesets::HOUSE, blocksets::HOUSE),
        ForestGate | Museum | Gate => (tilesets::GATE, blocksets::GATE),
        Underground => (tilesets::UNDERGROUND, blocksets::UNDERGROUND),
        Ship => (tilesets::SHIP, blocksets::SHIP),
        ShipPort => (tilesets::SHIP_PORT, blocksets::SHIP_PORT),
        Cemetery => (tilesets::CEMETERY, blocksets::CEMETERY),
        Interior => (tilesets::INTERIOR, blocksets::INTERIOR),
        Cavern => (tilesets::CAVERN, blocksets::CAVERN),
        Lobby => (tilesets::LOBBY, blocksets::LOBBY),
        Mansion => (tilesets::MANSION, blocksets::MANSION),
        Lab => (tilesets::LAB, blocksets::LAB),
        Club => (tilesets::CLUB, blocksets::CLUB),
        Facility => (tilesets::FACILITY, blocksets::FACILITY),
        Plateau => (tilesets::PLATEAU, blocksets::PLATEAU),
    }
}

/// The tileset's own tiles, as few as `--trim-whitespace` left: often short of the `TILESET_TILES`
/// the cartridge copies, which then fills the rest from whatever follows the sheet in its bank.
pub fn tileset_sheet(tileset: TileSetId) -> &'static [u8] {
    tileset_files(tileset).0
}

/// Block id to its 16 tile ids, row-major.
pub fn blockset(tileset: TileSetId) -> &'static [u8] {
    tileset_files(tileset).1
}

/// One tile of `tileset` as shade indices, blank past the end of the sheet.
pub fn tileset_tile(tileset: TileSetId, tile_id: u8) -> [u8; 64] {
    sheet_tile(tileset_sheet(tileset), tile_id as usize)
}

fn sheet_tile(sheet: &[u8], index: usize) -> [u8; 64] {
    match sheet.get(index * TILE_BYTES..(index + 1) * TILE_BYTES) {
        Some(tile) => decode_tile(tile),
        None => [0; 64],
    }
}

/// One NPC standing still, 16×16 shade indices, row-major.
#[derive(Copy, Clone)]
pub struct NpcSprite {
    pub shades: [u8; SPRITE_PX * SPRITE_PX],
}

/// The standing frame of `picture` facing `facing`, or `None` if the picture id has no sheet.
pub fn npc_sprite(picture: PictureId, facing: SpriteFacing) -> Option<NpcSprite> {
    let sheet = crate::map_objects::sprite_sheet(picture as u8)?;
    let sheet = &sheet.tiles[..sheet.bytes];

    // An immobile sprite falls back to facing down wholesale, layout included.
    let fits = |frame: &([u8; 4], _)| frame.0.iter().all(|&id| (id as usize + 1) * TILE_BYTES <= sheet.len());
    let frame = facing_frame(facing);
    let (tile_ids, layout) = match fits(&frame) {
        true => frame,
        false => facing_frame(SpriteFacing::Down),
    };

    let mut shades = [0u8; SPRITE_PX * SPRITE_PX];
    for (quadrant, &tile_id) in tile_ids.iter().enumerate() {
        let (dy, dx, attributes) = layout[quadrant];
        let pixels = sheet_tile(sheet, tile_id as usize);
        for y in 0..TILE_PX {
            for x in 0..TILE_PX {
                // The flip is per tile, and the layout has already swapped the columns.
                let source = match attributes & OAM_XFLIP {
                    0 => pixels[y * TILE_PX + x],
                    _ => pixels[y * TILE_PX + (TILE_PX - 1 - x)],
                };
                shades[(dy as usize + y) * SPRITE_PX + dx as usize + x] = source;
            }
        }
    }
    Some(NpcSprite { shades })
}

const OAM_XFLIP: u8 = 0x20;

/// The four tile ids and each one's `(y, x, attributes)` for a standing frame, read from
/// `SpriteFacingAndAnimationTable` rather than mirrored by hand.
fn facing_frame(facing: SpriteFacing) -> ([u8; 4], [(u8, u8, u8); 4]) {
    let (tiles, oam) = crate::tables::SPRITE_FACING_AND_ANIMATION_TABLE[facing as usize];
    (*tiles, oam.map(|[y, x, attributes]| (y, x, attributes)))
}

pub const GLYPHS: usize = FONT_BYTES.len() / TILE_BYTES;
/// Character code `C` is font tile `C - FIRST_GLYPH`.
const FIRST_GLYPH: u8 = 0x80;

/// The font tile that draws `c`, or `None` for a blank cell, a space included.
pub fn glyph_index(c: char) -> Option<u8> {
    let code = *PokemonString::from_string(&c.to_string()).0.first()?;
    code.checked_sub(FIRST_GLYPH)
}

pub fn glyph_mask(index: u8) -> [bool; 64] {
    debug_assert!((index as usize) < GLYPHS, "the font has {GLYPHS} glyphs, not {index}");
    let pixels = sheet_tile(&FONT_BYTES, index as usize);
    std::array::from_fn(|i| pixels[i] != 0)
}

pub fn glyphs(text: &str) -> Vec<Option<u8>> {
    text.chars().map(glyph_index).collect()
}

pub fn text_width(text: &str) -> usize {
    text.chars().count() * TILE_PX
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::render_font_string;

    fn all_tilesets() -> impl Iterator<Item = TileSetId> {
        (0..=23u8).map(|id| TileSetId::from_repr(id).expect("24 tilesets"))
    }

    #[test]
    fn every_tileset_sheet_is_drawn_art() {
        for tileset in all_tilesets() {
            let sheet = tileset_sheet(tileset);
            assert!(!sheet.is_empty(), "{tileset} has no sheet");
            assert_eq!(sheet.len() % TILE_BYTES, 0, "{tileset} sheet is not whole tiles");

            let mut histogram = [0usize; 4];
            for index in 0..sheet.len() / TILE_BYTES {
                for shade in sheet_tile(sheet, index) {
                    histogram[shade as usize] += 1;
                }
            }
            let pixels: usize = histogram.iter().sum();
            assert!(histogram.iter().filter(|&&n| n > 0).count() >= 3,
                    "{tileset} uses fewer than three shades: {histogram:?}");
            assert!(histogram.iter().all(|&n| n * 100 < pixels * 96),
                    "{tileset} is almost entirely one shade: {histogram:?}");
        }
    }

    /// Past a trimmed sheet is blank, and no block draws from there: below `TILESET_TILES` a
    /// blockset names only tiles its sheet has.
    #[test]
    fn a_short_sheet_is_blank_past_its_end_and_no_block_reaches_it() {
        let sheet = tileset_sheet(TileSetId::Underground);
        assert_eq!(sheet.len(), 25 * TILE_BYTES);
        assert_eq!(tileset_tile(TileSetId::Underground, 25), [0; 64]);
        assert_eq!(tileset_tile(TileSetId::Underground, 0xFF), [0; 64]);
        for tileset in all_tilesets() {
            let tiles = tileset_sheet(tileset).len() / TILE_BYTES;
            assert!(blockset(tileset).iter().all(|&id| (id as usize) < tiles || id as usize >= TILESET_TILES),
                    "{tileset} has a block drawing past its {tiles}-tile sheet");
        }
    }

    /// A walking NPC has four distinct facings and an immobile sprite one picture for all four.
    #[test]
    fn every_sprite_sheet_decodes_and_only_people_have_four_facings() {
        use SpriteFacing::*;
        let mut walkers = 0;
        let mut immobile = 0;
        for id in 1..=0x48u8 {
            let Some(picture) = PictureId::from_repr(id) else { continue };
            let frames: Vec<_> = [Down, Up, Left, Right].iter()
                .map(|&f| npc_sprite(picture, f).unwrap_or_else(|| panic!("{picture:?} has no sheet")).shades)
                .collect();
            assert!(frames[0].iter().any(|&s| s != 0), "{picture:?} facing down is blank");

            let distinct = frames.iter().collect::<std::collections::HashSet<_>>().len();
            match distinct {
                1 => immobile += 1,
                // Right is left mirrored, and no sprite is symmetric.
                4 => walkers += 1,
                n => panic!("{picture:?} has {n} distinct facings — expected 1 or 4"),
            }
        }
        assert!(walkers > 40 && immobile > 5, "{walkers} walkers, {immobile} immobile");
    }

    /// Right is left, mirrored, as the cartridge's table says.
    #[test]
    fn right_is_left_mirrored() {
        let left = npc_sprite(PictureId::Oak, SpriteFacing::Left).expect("Oak walks").shades;
        let right = npc_sprite(PictureId::Oak, SpriteFacing::Right).expect("Oak walks").shades;
        for y in 0..SPRITE_PX {
            for x in 0..SPRITE_PX {
                assert_eq!(right[y * SPRITE_PX + x], left[y * SPRITE_PX + (SPRITE_PX - 1 - x)],
                           "({x}, {y})");
            }
        }
        assert_ne!(left, right, "a symmetric sprite would pass the above vacuously");
    }

    /// The reverse charmap round-trips through the forward one.
    #[test]
    fn the_font_round_trips_through_the_decoder() {
        let mut checked = 0;
        for c in ('A'..='Z').chain('a'..='z').chain('0'..='9').chain("():;[]'-?!./,".chars()) {
            let index = glyph_index(c).unwrap_or_else(|| panic!("no glyph for {c:?}"));
            assert_eq!(render_font_string(&[index as usize], false), c.to_string(),
                       "{c:?} is glyph {index}");
            assert!(glyph_mask(index).iter().any(|&ink| ink), "{c:?} draws nothing");
            checked += 1;
        }
        assert_eq!(checked, 26 + 26 + 10 + 13);

        // A space lives in `TextBoxGraphics`, so the caller blanks it.
        assert_eq!(glyph_index(' '), None);
        assert_eq!(text_width("AB C"), 4 * TILE_PX);
    }

    /// `0` is not `O`, and `1` is not `I` or `l`.
    #[test]
    fn digits_are_not_the_letters_that_look_like_them() {
        let glyph = |c: char| glyph_mask(glyph_index(c).expect("has a glyph"));
        assert_ne!(glyph('0'), glyph('O'));
        assert_ne!(glyph('1'), glyph('I'));
        assert_ne!(glyph('1'), glyph('l'));
        assert_eq!(GLYPHS, 128, "FontGraphics is 128 tiles");
    }
}
