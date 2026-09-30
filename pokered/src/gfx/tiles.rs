use poke_core::font::FONT_BYTES;
use poke_core::gfx;
use poke_core::map_gfx::{tileset_sheet, TILESET_TILES};
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::TILE_BYTES;
use serde::{Deserialize, Serialize};

/// Tile indices of `vChars0`, `vChars1` (`vFont`) and `vChars2` (`vTileset`).
pub const V_CHARS0: usize = 0;
pub const V_CHARS1: usize = 128;
pub const V_CHARS2: usize = 256;

/// The tile patterns loaded for the screen to draw from: what `vChars0`–`2` hold, 2bpp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileData {
    tiles: Vec<[u8; TILE_BYTES]>,
    pub animation: MovingBgTiles,
}

/// `hTileAnimations` and the two counters `UpdateMovingBgTiles` keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovingBgTiles {
    /// `hTileAnimations`: 0 still, 1 water, 2 water and flowers.
    pub kind: u8,
    /// `hMovingBGTilesCounter1`.
    pub counter1: u8,
    /// `wMovingBGTilesCounter2`.
    pub counter2: u8,
}

impl Default for TileData {
    fn default() -> Self {
        Self { tiles: vec![[0; TILE_BYTES]; 384], animation: MovingBgTiles::default() }
    }
}

impl TileData {
    pub fn load(&mut self, first: usize, bytes: &[u8]) {
        for (i, tile) in bytes.chunks_exact(TILE_BYTES).enumerate() {
            self.tiles[first + i].copy_from_slice(tile);
        }
    }

    /// A 1bpp tile's byte serves as both planes, as `CopyVideoDataDouble` writes it.
    pub fn load_1bpp(&mut self, first: usize, bytes: &[u8]) {
        let doubled: Vec<u8> = bytes.iter().flat_map(|&b| [b, b]).collect();
        self.load(first, &doubled);
    }

    /// A background tile, `LCDC_BLOCK21`: ids from 128 are in `vChars1`, the rest in `vChars2`.
    pub fn bg(&self, id: u8) -> &[u8; TILE_BYTES] {
        &self.tiles[if id >= 128 { V_CHARS1 + (id - 128) as usize } else { V_CHARS2 + id as usize }]
    }

    pub fn obj(&self, id: u8) -> &[u8; TILE_BYTES] {
        &self.tiles[V_CHARS0 + id as usize]
    }

    /// `LoadFontTilePatterns`.
    pub fn load_font(&mut self) {
        self.load(V_CHARS1, &FONT_BYTES);
    }

    /// `LoadTextBoxTilePatterns`.
    pub fn load_text_box_tiles(&mut self) {
        self.load(V_CHARS2 + 0x60, gfx::font::FONT_EXTRA);
    }

    /// `LoadHpBarAndStatusTilePatterns`, which lands where the text box tiles do not overlap it.
    pub fn load_hp_bar_and_status_tiles(&mut self) {
        self.load(V_CHARS2 + 0x62, gfx::font::FONT_BATTLE_EXTRA);
    }

    /// `LoadEDTile`. It lands on the tile `¥` is drawn from, so the naming screen is the one place
    /// a yen sign would come out reading `ED`.
    pub fn load_ed_tile(&mut self) {
        self.load_1bpp(V_CHARS1 + 0x70, gfx::font::ED);
    }

    /// `LoadTilesetTilePatternData`. The cartridge copies `TILESET_TILES` whatever the sheet's
    /// length, so past a short one it copies its blockset and what follows; no block draws those tiles,
    /// and here they are blank.
    pub fn load_tileset(&mut self, tileset: TileSetId) {
        let sheet = tileset_sheet(tileset);
        self.load(V_CHARS2, sheet);
        self.tiles[V_CHARS2 + sheet.len() / TILE_BYTES..V_CHARS2 + TILESET_TILES].fill([0; TILE_BYTES]);
    }

    /// `UpdateMovingBgTiles`, from VBlank: the water tile shifts every 20 frames and, with
    /// flowers, the flower tile changes on the 21st.
    pub fn update_moving_bg_tiles(&mut self) {
        let animation = &mut self.animation;
        if animation.kind == 0 {
            return;
        }
        animation.counter1 += 1;
        if animation.counter1 < 20 {
            return;
        }
        if animation.counter1 == 21 {
            animation.counter1 = 0;
            let frame = match animation.counter2 & 3 {
                0 | 1 => gfx::tilesets::flower::FLOWER1,
                2 => gfx::tilesets::flower::FLOWER2,
                _ => gfx::tilesets::flower::FLOWER3,
            };
            self.tiles[V_CHARS2 + 0x03] = *frame;
            return;
        }
        animation.counter2 = (animation.counter2 + 1) & 7;
        let water = &mut self.tiles[V_CHARS2 + 0x14];
        for byte in water.iter_mut() {
            *byte = if animation.counter2 & 4 != 0 { byte.rotate_left(1) } else { byte.rotate_right(1) };
        }
        if animation.kind & 1 != 0 {
            animation.counter1 = 0;
        }
    }
}

/// The colour index, 0 to 3, of pixel `(x, y)` of a 2bpp tile.
pub fn pixel(tile: &[u8; TILE_BYTES], x: usize, y: usize) -> u8 {
    let (low, high) = (tile[y * 2], tile[y * 2 + 1]);
    let bit = 7 - x;
    ((low >> bit) & 1) | (((high >> bit) & 1) << 1)
}
