use poke_core::map_gfx::tileset_entry;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::{DmgBank, DmgPointer};
use serde::{Deserialize, Serialize};

pub const BLOCK_PX: i32 = 32;

/// The map as the overworld draws it: `wOverworldMap`'s blocks, border included, seen through a
/// camera whose position is the screen's top-left pixel in that buffer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapLayer {
    pub tileset: Option<TileSetId>,
    pub blocks_wide: usize,
    pub blocks: Vec<u8>,
    pub camera: (i32, i32),
    /// Background tiles a script wrote over the blocks' own, as `(row, column, tile)` in the
    /// buffer's tiles, sorted: what the screen keeps showing until the map is loaded again.
    #[serde(default)]
    pub overrides: Vec<(i32, i32, u8)>,
}

impl MapLayer {
    /// The background tile id at `(x, y)` in the buffer, or `None` off its edge.
    pub fn tile_at(&self, x: i32, y: i32) -> Option<u8> {
        let tileset = self.tileset?;
        if x < 0 || y < 0 || self.blocks_wide == 0 {
            return None;
        }
        if !self.overrides.is_empty()
            && let Ok(at) = self.overrides.binary_search_by_key(&(y / 8, x / 8), |&(row, column, _)| (row, column))
        {
            return Some(self.overrides[at].2);
        }
        let (bx, by) = ((x / BLOCK_PX) as usize, (y / BLOCK_PX) as usize);
        if bx >= self.blocks_wide {
            return None;
        }
        let block = *self.blocks.get(by * self.blocks_wide + bx)?;
        let entry = tileset_entry(tileset);
        let blockset = rom_slice(DmgPointer { bank: DmgBank::ROM { bank: entry.bank }, address: entry.blocks });
        let (tx, ty) = (((x % BLOCK_PX) / 8) as usize, ((y % BLOCK_PX) / 8) as usize);
        Some(blockset[block as usize * 16 + ty * 4 + tx])
    }
}

/// One OAM entry, as `PrepareOAMData` writes them: `y` and `x` carry the hardware's 16 and 8
/// pixel offsets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    pub y: u8,
    pub x: u8,
    pub tile: u8,
    pub attributes: u8,
}

impl Object {
    pub const BEHIND_BG: u8 = 1 << 7;
    pub const Y_FLIP: u8 = 1 << 6;
    pub const X_FLIP: u8 = 1 << 5;
    pub const OBP1: u8 = 1 << 4;
}

/// `rBGP`, `rOBP0`, `rOBP1` and the background scroll, with a scroll per line for the effects that
/// change `rSCX` mid-frame.
///
/// The three registers are the DMG's. Where a `wOnSGB` branch picks a different palette for the
/// SGB, the SGB's pick rides beside it, and only the SGB colour modes read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effects {
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub scx: u8,
    pub scy: u8,
    pub line_scx: Option<Vec<u8>>,
    #[serde(default)]
    sgb_bgp: Option<SgbPick>,
    #[serde(default)]
    sgb_obp0: Option<SgbPick>,
}

/// A register written through a `wOnSGB` branch: the DMG's value and the SGB's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SgbPick {
    pub dmg: u8,
    pub sgb: u8,
}

impl SgbPick {
    pub const fn both(value: u8) -> Self {
        Self { dmg: value, sgb: value }
    }

    /// The SGB's value stands only while the register still holds the DMG's, so a write that knows
    /// nothing of the SGB ends it.
    fn over(pick: Option<Self>, register: u8) -> u8 {
        pick.filter(|pick| pick.dmg == register).map_or(register, |pick| pick.sgb)
    }

    fn unless_same(self) -> Option<Self> {
        (self.dmg != self.sgb).then_some(self)
    }
}

impl Effects {
    pub fn pick_bgp(&mut self, pick: SgbPick) {
        self.bgp = pick.dmg;
        self.sgb_bgp = pick.unless_same();
    }

    pub fn pick_obp0(&mut self, pick: SgbPick) {
        self.obp0 = pick.dmg;
        self.sgb_obp0 = pick.unless_same();
    }

    /// `rBGP` on both paths, for a `push af` of it.
    pub fn bgp_pick(&self) -> SgbPick {
        SgbPick { dmg: self.bgp, sgb: SgbPick::over(self.sgb_bgp, self.bgp) }
    }

    pub fn obp0_pick(&self) -> SgbPick {
        SgbPick { dmg: self.obp0, sgb: SgbPick::over(self.sgb_obp0, self.obp0) }
    }
}

/// `GBPalNormal`.
impl Default for Effects {
    fn default() -> Self {
        Self {
            bgp: 0b11100100, obp0: 0b11010000, obp1: 0b11100100, scx: 0, scy: 0, line_scx: None,
            sgb_bgp: None, sgb_obp0: None,
        }
    }
}

/// A background map at the hardware's size, 32 by 32 tiles: `vBGMap0` or `vBGMap1`, for a screen
/// that scrolls past the 20 by 18 the UI surface covers or keeps a second picture for the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileMap(Vec<u8>);

impl TileMap {
    pub const SIZE: usize = 32;

    pub fn filled(tile: u8) -> Self {
        Self(vec![tile; Self::SIZE * Self::SIZE])
    }

    /// Wrapping, as the scroll does.
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.0[(y % Self::SIZE) * Self::SIZE + x % Self::SIZE]
    }

    pub fn set(&mut self, x: usize, y: usize, tile: u8) {
        self.0[(y % Self::SIZE) * Self::SIZE + x % Self::SIZE] = tile;
    }
}

/// `rWX`, `rWY` and the map the window shows, over the background and under the objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub x: u8,
    pub y: u8,
    pub tiles: TileMap,
}
