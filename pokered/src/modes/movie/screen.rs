//! What the movie screens keep of the hardware: both background maps, `AutoBgMapTransfer`'s
//! destination, and the registers VBlank latches from their `h` copies.
//!
//! The movie is the one part of the game that plays the two maps and the window against each other,
//! so it holds them itself and hands the compositor a finished `background` and `window` each
//! frame. A latched register, and the shadow OAM, written in one frame reach the screen in the
//! next, as `VBlank` copies them; `rBGP`, `rWX` and a mid-frame `rSCX` land at once.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::symbols::DmgPointer;
use serde::{Deserialize, Serialize};
use crate::gfx::layers::{Object, TileMap, Window};
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::mode::Ctx;
use crate::systems::pokedex::{front_pic_tiles, pic_tiles};

/// `hWY` past the last line.
pub const WINDOW_HIDDEN: u8 = 0x90;
/// `rWX` for a window flush with the left edge.
pub const WX_LEFT: u8 = 7;
/// `%11100100`, what `GBPalNormal` puts in `rBGP`.
pub const BGP_NORMAL: u8 = 0b1110_0100;
pub const OBP0_NORMAL: u8 = 0b1101_0000;
const OAM_OBJECTS: usize = 40;

/// `hAutoBGTransferDest`: a map and a tile offset into it. An offset past the map's end runs on
/// into the next map, as the address does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dest {
    pub map: usize,
    pub offset: usize,
}

impl Dest {
    /// `vBGMap0`.
    pub const BG_MAP0: Self = Self { map: 0, offset: 0 };
    /// `vBGMap1`.
    pub const BG_MAP1: Self = Self { map: 1, offset: 0 };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovieScreen {
    /// `vBGMap0` and `vBGMap1`.
    pub maps: [TileMap; 2],
    /// `hAutoBGTransferEnabled` and `hAutoBGTransferDest`.
    pub transfer: Option<Dest>,
    /// `LCDC`'s background map bit: which of `maps` the background shows.
    pub bg_map: usize,
    /// `hSCX`, `hSCY` and `hWY`, which `VBlank` copies into the registers.
    pub scx: u8,
    pub scy: u8,
    pub wy: u8,
    /// `rWY` as the last `VBlank` left it.
    latched_wy: u8,
    /// `rWX`, written directly.
    pub wx: u8,
    /// `rWX` written after the lines it matters to have been drawn, so it shows from the next frame.
    pub wx_next: Option<u8>,
    /// `wShadowOAM`, which `VBlank` copies into OAM.
    pub oam: Vec<Object>,
}

impl Default for MovieScreen {
    /// `Init`: the maps cleared with the rest of VRAM, the window moved off the screen.
    fn default() -> Self {
        Self {
            maps: [TileMap::filled(0), TileMap::filled(0)],
            transfer: Some(Dest::BG_MAP1),
            bg_map: 0,
            scx: 0,
            scy: 0,
            wy: 144,
            latched_wy: 144,
            wx: WX_LEFT,
            wx_next: None,
            oam: vec![Object::default(); OAM_OBJECTS],
        }
    }
}

impl MovieScreen {
    /// `VBlank`'s copies, at the top of a frame: `wTileMap` as the last frame left it carried to
    /// the transfer's destination, and the latched registers.
    pub fn vblank(&mut self, ctx: &mut Ctx) {
        if let Some(dest) = self.transfer {
            self.copy_tile_map(&ctx.screen.ui, dest);
        }
        ctx.screen.effects.scx = self.scx;
        ctx.screen.effects.scy = self.scy;
        ctx.screen.effects.line_scx = None;
        self.latched_wy = self.wy;
        if let Some(wx) = self.wx_next.take() {
            self.wx = wx;
        }
        ctx.screen.sprites = self.oam.clone();
    }

    /// The end of a frame: the compositor given both maps as they now stand.
    pub fn present(&mut self, ctx: &mut Ctx) {
        ctx.screen.background = Some(self.maps[self.bg_map].clone());
        ctx.screen.window = (self.latched_wy < 144 && self.wx < 167)
            .then(|| Window { x: self.wx, y: self.latched_wy, tiles: self.maps[1].clone() });
    }

    /// `hAutoBGTransferDest` moved and the transfer let run until it has copied all of `wTileMap`
    /// there: `TitleScreenCopyTileMapToVRAM` and its kind, without their `Delay3`.
    pub fn copy_to(&mut self, ui: &UiSurface, dest: Dest) {
        self.transfer = Some(dest);
        self.copy_tile_map(ui, dest);
    }

    /// `AutoBgMapTransfer`, all three thirds at once: each row is 20 tiles, then 12 skipped.
    fn copy_tile_map(&mut self, ui: &UiSurface, dest: Dest) {
        for row in 0..SCREEN_TILES_Y {
            for column in 0..SCREEN_TILES_X {
                let at = dest.offset + row * TileMap::SIZE + column;
                let (map, at) = (dest.map + at / (TileMap::SIZE * TileMap::SIZE), at % (TileMap::SIZE * TileMap::SIZE));
                if map < 2 {
                    self.maps[map].set(at % TileMap::SIZE, at / TileMap::SIZE, ui.get(column, row));
                }
            }
        }
    }

    /// `ClearSprites`.
    pub fn clear_sprites(&mut self) {
        self.oam.fill(Object::default());
    }

    /// Hands the screen back to the modes that draw only the UI surface.
    pub fn release(ctx: &mut Ctx) {
        ctx.screen.background = None;
        ctx.screen.window = None;
        ctx.screen.effects.scx = 0;
        ctx.screen.effects.scy = 0;
        ctx.screen.effects.line_scx = None;
        ctx.screen.sprites.clear();
    }
}

/// Bytes between two labels.
pub fn between(start: DmgPointer, end: DmgPointer) -> &'static [u8] {
    &rom_slice(start)[..(end.address - start.address) as usize]
}

/// `ClearScreen` without its `Delay3`.
pub fn clear_screen(ui: &mut UiSurface) {
    ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
}

/// A compressed 7x7 pic into `vFrontPic`, as `IntroDisplayPicCenteredOrUpperRight` and
/// `HoFLoadPlayerPics` decompress one.
pub fn load_pic(ctx: &mut Ctx, pic: DmgPointer) {
    let tiles = pic_tiles(&poke_core::mon_gfx::pic_shades(rom_slice(pic)), false);
    load_front_pic_tiles(ctx, &tiles);
}

/// `LoadFrontSpriteByMonIndex`'s load, or `LoadFlippedFrontSpriteByMonIndex`'s.
pub fn load_mon_pic(ctx: &mut Ctx, species: PokemonSpecies, flipped: bool) {
    let tiles = front_pic_tiles(species, flipped);
    load_front_pic_tiles(ctx, &tiles);
}

fn load_front_pic_tiles(ctx: &mut Ctx, tiles: &[[u8; TILE_BYTES]]) {
    let bytes: Vec<u8> = tiles.iter().flatten().copied().collect();
    ctx.screen.tiles.load(V_CHARS2, &bytes);
}

/// `CopyUncompressedPicToHL`: 49 tile ids from `start`, down each column, the columns laid right to
/// left when `flipped`.
pub fn copy_pic_to_tile_map(ui: &mut UiSurface, x: usize, y: usize, start: u8, flipped: bool) {
    for column in 0..7 {
        for row in 0..7 {
            let at_x = if flipped { x + 6 - column } else { x + column };
            ui.set(at_x, y + row, start.wrapping_add((column * 7 + row) as u8));
        }
    }
}

/// `CopyTileIDsFromList` over `TileIDListPointerTable`'s entry: `width` by `height` ids, row by row,
/// each `base` past.
pub fn copy_tile_ids(ui: &mut UiSurface, x: usize, y: usize, entry: usize, base: u8) {
    let pointers = rom_slice(poke_core::symbols::pokered_symbols::TileIDListPointerTable);
    let row = &pointers[entry * 3..entry * 3 + 3];
    let list = DmgPointer { address: u16::from_le_bytes([row[0], row[1]]), ..poke_core::symbols::pokered_symbols::TileIDListPointerTable };
    let (height, width) = ((row[2] >> 4) as usize, (row[2] & 0xF) as usize);
    let ids = rom_slice(list);
    for r in 0..height {
        for c in 0..width {
            ui.set(x + c, y + r, ids[r * width + c].wrapping_add(base));
        }
    }
}
