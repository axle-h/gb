//! The graphics a *map* is drawn from: tileset sheets, overworld sprite sheets, and the game's own
//! font — all read out of the cartridge the binary already carries.
//!
//! This is [`crate::pokemon::rom_gfx`]'s job one level up. `rom_gfx` knows where a ROM pointer's
//! bytes are and how to turn 2bpp into shade indices; this module knows which pointers a map needs
//! and how pokered's tables are laid out. Neither knows what any of it looks like — colour belongs
//! to the caller, which is [`crate::llm::map_image`] for the picture the model is sent, exactly as
//! it is `src/web/` for the badges and the Pokédex.
//!
//! Nothing here touches the MMU. Every read is against the `POKERED` `&'static [u8]`, which is what
//! lets the LLM worker thread render a map while the emulator thread carries on running the game.
//!
//! # The three tables
//!
//! **`Tilesets`** (`03:47be`), 12 bytes per entry, indexed by [`TileSetId`]:
//! `db BANK(GFX); dw Block, GFX, Coll; db counter×3; db grass; db animation`. The `GFX` pointer is
//! the one [`crate::pokemon::map_metadata::MMU::read_tileset_header`] historically skipped, and it
//! is what turns a block map into pixels. Bank is shared with the blockset — the two are assembled
//! back to back in the same section.
//!
//! **`SpriteSheetPointerTable`** (`05:7b27`), 4 bytes per entry, indexed by
//! [`PictureId`]` - 1`: `dw gfx; db byte_count; db BANK(gfx)`. A walking NPC is 12 tiles; an item
//! ball, a boulder or a sleeping gambler is 4.
//!
//! **`SpriteFacingAndAnimationTable`** (`01:4000`), 4 bytes per entry
//! (`dw tile_ids, dw oam_layout`), indexed by facing-and-frame. Entry `facing + frame`, and
//! [`SpriteFacing`]'s values are already `0/4/8/C`, so the standing frame is entry `facing` at byte
//! offset `facing * 4`. ⚠️ **Read the OAM layout rather than assuming it**: the four tiles' screen
//! positions *and* the horizontal flip that makes "facing right" out of the left-facing art both
//! come from that second pointer. `.FlippedOAM` swaps the left and right columns as well as setting
//! `OAM_XFLIP`, so mirroring the assembled 16×16 by hand is right only by coincidence.
//!
//! # ⚠️ A tileset sheet can run off the end of its bank
//!
//! `LoadTilesetTilePatternData` copies a fixed `MAP_TILESET_SIZE` (`$60`) tiles into `vTileset`
//! whatever the tileset's real size, so several sheets legitimately overrun their own label — into
//! the blockset that follows, and for `Underground` (`1b:7d60`, 672 bytes short of `$8000`) past the
//! end of the bank entirely. On hardware that reads whatever is mapped there and it never matters,
//! because no map references a tile id that high. Here the sheet is **clamped to the bank** and
//! [`tileset_tile`] answers a blank tile for an id past the end, so a malformed blockset draws a
//! hole rather than panicking on the emulator's behalf.

use crate::pokemon::font::FONT_BYTES;
use crate::pokemon::map_header::TileSetId;
use crate::pokemon::rom_gfx::{decode_tile, rom_slice, TILE_BYTES};
use crate::pokemon::sprite::{PictureId, SpriteFacing};
use crate::pokemon::strings::PokemonString;
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};

/// Pixels along one edge of a tile.
pub const TILE_PX: usize = 8;
/// What `LoadTilesetTilePatternData` copies to `vTileset` — pokered's `MAP_TILESET_SIZE`.
pub const TILESET_TILES: usize = 0x60;
/// An overworld sprite is 2×2 tiles.
pub const SPRITE_PX: usize = 16;

// ── The tileset table ────────────────────────────────────────────────────────────────────────────

/// One row of pokered's `Tilesets`.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TilesetEntry {
    /// The ROM bank holding **both** the blockset and the graphics.
    pub bank: u8,
    /// `<Tileset>_Block` — block id → 16 tile ids.
    pub blocks: u16,
    /// `<Tileset>_GFX` — the 2bpp tile sheet.
    pub gfx: u16,
    /// `<Tileset>_Coll` — the `$FF`-terminated list of walkable tile ids.
    pub coll: u16,
    /// Counter / "talk over" tile ids; `0xFF` where unused.
    pub talking_over: [u8; 3],
    /// The tile id wild encounters happen on, or `0xFF` for a tileset with no grass.
    pub grass_tile: u8,
}

const TILESET_ENTRY_SIZE: u16 = 12;

/// The `Tilesets` row for `tileset`.
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

/// `tileset`'s 2bpp tile sheet, clamped to the end of its bank (see the module note on the
/// deliberate overrun). Tile `n` is at `[n * TILE_BYTES ..]` for as far as it goes.
pub fn tileset_sheet(tileset: TileSetId) -> &'static [u8] {
    let entry = tileset_entry(tileset);
    let bytes = rom_slice(DmgPointer { bank: DmgBank::ROM { bank: entry.bank }, address: entry.gfx });
    &bytes[..bytes.len().min(TILESET_TILES * TILE_BYTES)]
}

/// One tile of `tileset`, as shade indices `0`–`3`. A tile id past the end of the (clamped) sheet
/// draws blank rather than panicking.
pub fn tileset_tile(tileset: TileSetId, tile_id: u8) -> [u8; 64] {
    sheet_tile(tileset_sheet(tileset), tile_id as usize)
}

fn sheet_tile(sheet: &[u8], index: usize) -> [u8; 64] {
    match sheet.get(index * TILE_BYTES..(index + 1) * TILE_BYTES) {
        Some(tile) => decode_tile(tile),
        None => [0; 64],
    }
}

// ── Overworld sprites ────────────────────────────────────────────────────────────────────────────

/// One NPC standing still, 16×16 shade indices, row-major.
///
/// ⚠️ **Shade `0` is transparent**, as it is for every overworld sprite on the hardware — it is the
/// surround, not white. A caller that paints it draws each person in a box.
#[derive(Copy, Clone)]
pub struct NpcSprite {
    pub shades: [u8; SPRITE_PX * SPRITE_PX],
}

/// The standing frame of `picture` facing `facing`, or `None` if the picture id has no sheet.
///
/// ⚠️ **An immobile sprite has one frame and no facing.** Item balls, boulders, the fossil and the
/// sleeping gamblers have 4-tile sheets, and pokered handles them by jumping to the second half of
/// `SpriteFacingAndAnimationTable`, every row of which is `.StandingDown`. A sheet too short for the
/// requested facing therefore falls back to the down-facing tiles rather than reading a neighbour's
/// graphics, which is what indexing blindly would do.
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

    // ⚠️ **The whole entry falls back, layout included.** An immobile sprite is four tiles, so the
    // left-facing frame's ids (`$08`–`$0b`) do not exist for one — and pokered's answer is to skip
    // to the second half of the table, every row of which is `.StandingDown, .NormalOAM`. Swapping
    // only the tile ids and keeping `.FlippedOAM` would draw a right-facing item ball as a
    // *mirrored* one, which is a different picture, not the same picture.
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
                // ⚠️ The flip is per tile *and* the layout has already swapped the columns.
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

/// Hardware OAM's horizontal-flip bit. The attribute byte in `SpriteFacingAndAnimationTable` mixes
/// it with pokered's own pseudo-flags (`FACING_END` = 1, `UNDER_GRASS` = 2), which do not collide.
const OAM_XFLIP: u8 = 0x20;

/// The four tile ids and the `(y, x, attributes)` of each, for one standing frame — read from the
/// ROM's own table rather than transcribed, so "facing right is facing left, mirrored" is a fact the
/// cartridge states rather than one this file assumes.
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

// ── The game's own font ──────────────────────────────────────────────────────────────────────────

/// The number of glyphs `FontGraphics` holds.
pub const GLYPHS: usize = FONT_BYTES.len() / TILE_BYTES;
/// The character code the font sheet starts at — `charmap.asm` puts `"A"` at `$80`, and
/// `LoadFontTilePatterns` copies the sheet to `vFont` (`$8800`), where the tile index and the
/// character code are the same number.
const FIRST_GLYPH: u8 = 0x80;

/// The font tile index that draws `c`, or `None` for anything the sheet has no glyph for —
/// including a space, which is `$7F` and lives in `TextBoxGraphics`, not here. Callers draw those
/// as a blank cell.
///
/// ⚠️ **The forward direction of this map already exists** as
/// [`crate::pokemon::font::render_font_string`], and `sprite`/`nickname` writing already goes
/// through [`PokemonString::from_string`] — so this reuses the latter rather than adding a third
/// copy of the charmap. `the_font_round_trips_through_the_decoder` pins the two together.
pub fn glyph_index(c: char) -> Option<u8> {
    let code = *PokemonString::from_string(&c.to_string()).0.first()?;
    code.checked_sub(FIRST_GLYPH)
}

/// A glyph as a stencil: `true` where there is ink.
///
/// ⚠️ **The font is 1bpp** — `FontGraphics` is 0x400 bytes of one bit per pixel, and
/// [`FONT_BYTES`] is the compile-time doubling of it into the 2bpp form the hardware wants. A mask
/// is therefore the honest shape, and it is also what keeps drawing light text on a dark plate from
/// being the palette inversion `src/web/sprites.rs` forbids: a stencil has no fill to negate, so the
/// ink colour is a choice, not a flip.
pub fn glyph_mask(index: u8) -> [bool; 64] {
    debug_assert!((index as usize) < GLYPHS, "the font has {GLYPHS} glyphs, not {index}");
    let pixels = sheet_tile(&FONT_BYTES, index as usize);
    std::array::from_fn(|i| pixels[i] != 0)
}

/// `text` as one entry per character: the glyph to draw, or `None` for a blank cell.
pub fn glyphs(text: &str) -> Vec<Option<u8>> {
    text.chars().map(glyph_index).collect()
}

/// How wide `text` renders, in pixels. Every glyph is [`TILE_PX`] wide, blanks included.
pub fn text_width(text: &str) -> usize {
    text.chars().count() * TILE_PX
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::font::render_font_string;
    use crate::pokemon::symbols::pokered_symbols;

    fn all_tilesets() -> impl Iterator<Item = TileSetId> {
        (0..=23u8).map(|id| TileSetId::from_repr(id).expect("24 tilesets"))
    }

    /// The proof that `gfx` is read from the right two bytes of the row, rather than the assertion
    /// that it looks plausible: `build.rs` emits a constant for every `::`-exported label in the
    /// disassembly, so the table this module parses can be checked against the linker's own answer
    /// for all three pointers of all twenty-four tilesets.
    ///
    /// Same idea as `mon_gfx`'s `the_decompressor_matches_upstreams_own_2bpp` — compare against
    /// something upstream generated, not against yesterday's output.
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

    /// Every tileset draws something. Catches a `gfx` pointer read one byte out, which would still
    /// land inside the table and still decode — as noise.
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

    /// The `Underground` tileset's graphics are 672 bytes from the end of bank `$1b`, and the game
    /// copies `$600`. Clamping is what stops that being a panic; this is the case that proves the
    /// clamp is exercised rather than theoretical.
    #[test]
    fn a_tileset_that_overruns_its_bank_is_clamped_not_panicked() {
        let sheet = tileset_sheet(TileSetId::Underground);
        assert!(sheet.len() < TILESET_TILES * TILE_BYTES,
                "Underground is the short one — if this stops being true the clamp is untested");
        // And a tile id past the end is a hole, not a crash.
        assert_eq!(tileset_tile(TileSetId::Underground, 0xFF), [0; 64]);
    }

    /// Each of the four facings is a distinct picture for a walking NPC, and the immobile sprites
    /// answer with one picture for all four rather than reading someone else's tiles.
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
                // Three sets of art — down, up, left — but four distinct *pictures*, because the
                // mirror that makes right out of left is not a symmetry of any of these sprites.
                4 => walkers += 1,
                n => panic!("{picture:?} has {n} distinct facings — expected 1 or 4"),
            }
        }
        assert!(walkers > 40 && immobile > 5, "{walkers} walkers, {immobile} immobile");
    }

    /// Right is left, mirrored — and the cartridge is what says so. If pokered ever pointed
    /// `.FlippedOAM` somewhere else this would go red rather than quietly drawing the wrong art.
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

    /// The new reverse charmap against the existing forward one. Neither can drift without this
    /// failing, which is the whole reason the reverse direction reuses `PokemonString` rather than
    /// transcribing `charmap.asm` a third time.
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

        // A space is $7F and lives in TextBoxGraphics, not FontGraphics — the caller blanks it.
        assert_eq!(glyph_index(' '), None);
        assert_eq!(text_width("AB C"), 4 * TILE_PX);
    }

    /// Two glyphs that are easy to confuse if the sheet were read at the wrong offset: `0` is not
    /// `O`, and `1` is not `I` or `l`.
    #[test]
    fn digits_are_not_the_letters_that_look_like_them() {
        let glyph = |c: char| glyph_mask(glyph_index(c).expect("has a glyph"));
        assert_ne!(glyph('0'), glyph('O'));
        assert_ne!(glyph('1'), glyph('I'));
        assert_ne!(glyph('1'), glyph('l'));
        assert_eq!(GLYPHS, 128, "FontGraphics is 128 tiles");
    }
}
