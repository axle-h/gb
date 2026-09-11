//! The graphics a map is drawn from, read out of the cartridge: tileset sheets, overworld sprite
//! sheets, and the game's own font.

use crate::font::FONT_BYTES;
use crate::map_header::TileSetId;
use crate::rom_gfx::{decode_tile, rom_slice, TILE_BYTES};
use crate::sprite::{PictureId, SpriteFacing};
use crate::strings::PokemonString;
use crate::symbols::{pokered_symbols, DmgBank, DmgPointer};

pub const TILE_PX: usize = 8;
/// What `LoadTilesetTilePatternData` copies to `vTileset`.
pub const TILESET_TILES: usize = 0x60;
pub const SPRITE_PX: usize = 16;

/// One row of pokered's `Tilesets`.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TilesetEntry {
    pub bank: u8,
    /// `<Tileset>_Block`: block id to 16 tile ids.
    pub blocks: u16,
    pub gfx: u16,
    /// `<Tileset>_Coll`: the `$FF`-terminated list of walkable tile ids.
    pub coll: u16,
    /// Counter / "talk over" tile ids; `0xFF` where unused.
    pub talking_over: [u8; 3],
    /// The tile id wild encounters happen on, or `0xFF` for a tileset with no grass.
    pub grass_tile: u8,
}

const TILESET_ENTRY_SIZE: u16 = 12;

pub fn tileset_entry(tileset: TileSetId) -> TilesetEntry {
    let row = rom_slice(pokered_symbols::Tilesets + tileset as u16 * TILESET_ENTRY_SIZE);
    let le = |i: usize| u16::from_le_bytes([row[i], row[i + 1]]);
    TilesetEntry {
        bank: row[0],
        blocks: le(1),
        gfx: le(3),
        coll: le(5),
        talking_over: [row[7], row[8], row[9]],
        grass_tile: row[10],
    }
}

/// Clamped to the bank: a sheet can run off its end, because the cartridge copies a fixed count.
pub fn tileset_sheet(tileset: TileSetId) -> &'static [u8] {
    let entry = tileset_entry(tileset);
    let bytes = rom_slice(DmgPointer { bank: DmgBank::ROM { bank: entry.bank }, address: entry.gfx });
    &bytes[..bytes.len().min(TILESET_TILES * TILE_BYTES)]
}

/// One tile of `tileset` as shade indices, blank past the end of the clamped sheet.
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
    const SPRITE_ENTRY_SIZE: u16 = 4;
    let entry = rom_slice(
        pokered_symbols::SpriteSheetPointerTable + (picture as u16 - 1) * SPRITE_ENTRY_SIZE);
    let (gfx, byte_count, bank) = (u16::from_le_bytes([entry[0], entry[1]]), entry[2] as usize, entry[3]);
    if byte_count == 0 {
        return None;
    }
    let sheet = rom_slice(DmgPointer { bank: DmgBank::ROM { bank }, address: gfx });
    let sheet = &sheet[..sheet.len().min(byte_count)];

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

/// The four tile ids and each one's `(y, x, attributes)` for a standing frame, read from the ROM
/// rather than mirrored by hand.
fn facing_frame(facing: SpriteFacing) -> ([u8; 4], [(u8, u8, u8); 4]) {
    let entry = rom_slice(pokered_symbols::SpriteFacingAndAnimationTable + facing as u16 * 4);
    let bank = pokered_symbols::SpriteFacingAndAnimationTable.bank;
    let at = |address: u16| rom_slice(DmgPointer { bank, address });

    let tiles = at(u16::from_le_bytes([entry[0], entry[1]]));
    let oam = at(u16::from_le_bytes([entry[2], entry[3]]));
    let mut tile_ids = [0u8; 4];
    let mut layout = [(0u8, 0u8, 0u8); 4];
    for quadrant in 0..4 {
        tile_ids[quadrant] = tiles[quadrant];
        layout[quadrant] = (oam[quadrant * 3], oam[quadrant * 3 + 1], oam[quadrant * 3 + 2]);
    }
    (tile_ids, layout)
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
    use crate::symbols::pokered_symbols;

    fn all_tilesets() -> impl Iterator<Item = TileSetId> {
        (0..=23u8).map(|id| TileSetId::from_repr(id).expect("24 tilesets"))
    }

    /// Every `Tilesets` row's three pointers match the linker's own symbols.
    #[test]
    fn the_tileset_table_agrees_with_the_generated_symbols() {
        let expected: Vec<(TileSetId, DmgPointer, DmgPointer, DmgPointer)> = vec![
            (TileSetId::Overworld,  pokered_symbols::Overworld_GFX,  pokered_symbols::Overworld_Block,  pokered_symbols::Overworld_Coll),
            (TileSetId::RedsHouse1, pokered_symbols::RedsHouse1_GFX, pokered_symbols::RedsHouse1_Block, pokered_symbols::RedsHouse1_Coll),
            (TileSetId::Mart,       pokered_symbols::Mart_GFX,       pokered_symbols::Mart_Block,       pokered_symbols::Mart_Coll),
            (TileSetId::Forest,     pokered_symbols::Forest_GFX,     pokered_symbols::Forest_Block,     pokered_symbols::Forest_Coll),
            (TileSetId::Dojo,       pokered_symbols::Dojo_GFX,       pokered_symbols::Dojo_Block,       pokered_symbols::Dojo_Coll),
            (TileSetId::Pokecenter, pokered_symbols::Pokecenter_GFX, pokered_symbols::Pokecenter_Block, pokered_symbols::Pokecenter_Coll),
            (TileSetId::Gym,        pokered_symbols::Gym_GFX,        pokered_symbols::Gym_Block,        pokered_symbols::Gym_Coll),
            (TileSetId::House,      pokered_symbols::House_GFX,      pokered_symbols::House_Block,      pokered_symbols::House_Coll),
            (TileSetId::Underground, pokered_symbols::Underground_GFX, pokered_symbols::Underground_Block, pokered_symbols::Underground_Coll),
            (TileSetId::Ship,       pokered_symbols::Ship_GFX,       pokered_symbols::Ship_Block,       pokered_symbols::Ship_Coll),
            (TileSetId::ShipPort,   pokered_symbols::ShipPort_GFX,   pokered_symbols::ShipPort_Block,   pokered_symbols::ShipPort_Coll),
            (TileSetId::Cemetery,   pokered_symbols::Cemetery_GFX,   pokered_symbols::Cemetery_Block,   pokered_symbols::Cemetery_Coll),
            (TileSetId::Interior,   pokered_symbols::Interior_GFX,   pokered_symbols::Interior_Block,   pokered_symbols::Interior_Coll),
            (TileSetId::Cavern,     pokered_symbols::Cavern_GFX,     pokered_symbols::Cavern_Block,     pokered_symbols::Cavern_Coll),
            (TileSetId::Lobby,      pokered_symbols::Lobby_GFX,      pokered_symbols::Lobby_Block,      pokered_symbols::Lobby_Coll),
            (TileSetId::Mansion,    pokered_symbols::Mansion_GFX,    pokered_symbols::Mansion_Block,    pokered_symbols::Mansion_Coll),
            (TileSetId::Lab,        pokered_symbols::Lab_GFX,        pokered_symbols::Lab_Block,        pokered_symbols::Lab_Coll),
            (TileSetId::Club,       pokered_symbols::Club_GFX,       pokered_symbols::Club_Block,       pokered_symbols::Club_Coll),
            (TileSetId::Facility,   pokered_symbols::Facility_GFX,   pokered_symbols::Facility_Block,   pokered_symbols::Facility_Coll),
            (TileSetId::Plateau,    pokered_symbols::Plateau_GFX,    pokered_symbols::Plateau_Block,    pokered_symbols::Plateau_Coll),
        ];
        for (tileset, gfx, blocks, coll) in expected {
            let entry = tileset_entry(tileset);
            assert_eq!((entry.bank, entry.gfx), (gfx.bank.id(), gfx.address), "{tileset} gfx");
            assert_eq!((entry.bank, entry.blocks), (blocks.bank.id(), blocks.address), "{tileset} blockset");
            assert_eq!(entry.coll, coll.address, "{tileset} collision list");
        }
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

    /// `Underground`'s graphics run off the end of their bank, which is clamped.
    #[test]
    fn a_tileset_that_overruns_its_bank_is_clamped_not_panicked() {
        let sheet = tileset_sheet(TileSetId::Underground);
        assert!(sheet.len() < TILESET_TILES * TILE_BYTES,
                "Underground is the short one — if this stops being true the clamp is untested");
        assert_eq!(tileset_tile(TileSetId::Underground, 0xFF), [0; 64]);
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
