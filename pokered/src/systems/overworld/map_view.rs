use poke_core::map_gfx::tileset_entry;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::{pokered_symbols, DmgBank, DmgPointer};
use serde::{Deserialize, Serialize};
use crate::gfx::layers::{MapLayer, BLOCK_PX};
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::systems::map_data::MAP_BORDER;

/// `wOverworldMap` and the view into it: `wCurrentTileBlockMapViewPointer`, kept as an offset into
/// the buffer, and `wXBlockCoord`/`wYBlockCoord`, which half of the block the player stands in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapView {
    pub tileset: TileSetId,
    /// `wCurMapWidth` and `wCurMapHeight`, in blocks.
    pub width: u8,
    pub height: u8,
    pub blocks: Vec<u8>,
    pub view: u16,
    pub x_block: u8,
    pub y_block: u8,
    /// [`MapLayer::overrides`], which a load of the blocks clears.
    #[serde(default)]
    pub tile_overrides: Vec<(i32, i32, u8)>,
}

/// `SURROUNDING_WIDTH` and `SURROUNDING_HEIGHT`, in tiles.
const SURROUNDING_COLUMNS: usize = 24;
const SURROUNDING_ROWS: usize = 20;

/// The screen's tiles as `LoadCurrentMapView` leaves `wTileMap`, twenty to a row.
pub type TileMap = [u8; SCREEN_TILES_X * SCREEN_TILES_Y];

impl MapView {
    pub fn stride(&self) -> u16 {
        self.width as u16 + 2 * MAP_BORDER as u16
    }

    /// `wCurrentTileBlockMapViewPointer` from the address a `warp_to` or connection names.
    pub fn view_from_address(address: u16) -> u16 {
        address.wrapping_sub(pokered_symbols::wOverworldMap.address)
    }

    /// The top-left pixel of the screen in the buffer.
    pub fn camera(&self) -> (i32, i32) {
        let stride = self.stride() as i32;
        let view = self.view as i32;
        (view % stride * BLOCK_PX + self.x_block as i32 * 16, view / stride * BLOCK_PX + self.y_block as i32 * 16)
    }

    pub fn layer(&self) -> MapLayer {
        MapLayer { tileset: Some(self.tileset), blocks_wide: self.stride() as usize, blocks: self.blocks.clone(), camera: self.camera(),
            overrides: self.tile_overrides.clone() }
    }

    /// `LoadCurrentMapView`: the tile the view puts at screen cell `(column, row)`. Past the end of
    /// the buffer the cartridge reads on into WRAM; the border block stands in for it.
    pub fn tile(&self, column: usize, row: usize) -> u8 {
        let (x, y) = (column + 2 * self.x_block as usize, row + 2 * self.y_block as usize);
        let at = self.view as usize + y / 4 * self.stride() as usize + x / 4;
        let block = self.blocks.get(at).copied().unwrap_or(0);
        let entry = tileset_entry(self.tileset);
        let blockset = rom_slice(DmgPointer { bank: DmgBank::ROM { bank: entry.bank }, address: entry.blocks });
        blockset[block as usize * 16 + y % 4 * 4 + x % 4]
    }

    /// `wSurroundingTiles`: the six by five blocks from the view pointer, which `LoadCurrentMapView`
    /// leaves in the bytes just past `wTileMap`.
    pub fn surrounding_tiles(&self) -> Vec<u8> {
        let entry = tileset_entry(self.tileset);
        let blockset = rom_slice(DmgPointer { bank: DmgBank::ROM { bank: entry.bank }, address: entry.blocks });
        (0..SURROUNDING_ROWS * SURROUNDING_COLUMNS).map(|i| {
            let (x, y) = (i % SURROUNDING_COLUMNS, i / SURROUNDING_COLUMNS);
            let block = self.blocks.get(self.view as usize + y / 4 * self.stride() as usize + x / 4).copied().unwrap_or(0);
            blockset[block as usize * 16 + y % 4 * 4 + x % 4]
        }).collect()
    }

    pub fn tile_map(&self) -> TileMap {
        std::array::from_fn(|i| self.tile(i % SCREEN_TILES_X, i / SCREEN_TILES_X))
    }

    /// `AdvancePlayerSprite`'s first frame of a step: half a block along, and the view pointer a
    /// block along when that crosses into the next one.
    pub fn step(&mut self, dx: i8, dy: i8) {
        let x_block = self.x_block.wrapping_add(dx as u8);
        if x_block == 2 {
            self.x_block = 0;
            self.view = self.view.wrapping_add(1);
            return;
        }
        if x_block == 0xFF {
            self.x_block = 1;
            self.view = self.view.wrapping_sub(1);
            return;
        }
        self.x_block = x_block;
        let y_block = self.y_block.wrapping_add(dy as u8);
        if y_block == 2 {
            self.y_block = 0;
            self.view = self.view.wrapping_add(self.stride());
        } else if y_block == 0xFF {
            self.y_block = 1;
            self.view = self.view.wrapping_sub(self.stride());
        } else {
            self.y_block = y_block;
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::map::Map;
    use poke_core::map_header::MapHeader;
    use poke_core::map_objects::MapObjects;
    use super::*;
    use crate::systems::map_data::tile_block_map;

    fn pallet_town_at_the_door() -> MapView {
        let header = MapHeader::read(Map::PalletTown).unwrap();
        let warp = MapObjects::read(Map::PalletTown).unwrap().warp_to[0];
        MapView {
            tileset: header.tileset,
            width: header.width,
            height: header.height,
            blocks: tile_block_map(Map::PalletTown).unwrap(),
            view: MapView::view_from_address(warp.view),
            x_block: warp.x & 1,
            y_block: warp.y & 1,
            tile_overrides: Vec::new(),
        }
    }

    /// The view from a `warp_to` is the camera `map_data::camera` works out from the coordinates.
    #[test]
    fn a_warp_s_view_is_the_camera_on_its_square() {
        let view = pallet_town_at_the_door();
        assert_eq!(view.camera(), crate::systems::map_data::camera(5, 5));
        // The player's own square is cells (8, 8) to (9, 9), and a door is below Red's house roof.
        assert_eq!(view.tile(8, 9), view.layer().tile_at(view.camera().0 + 64, view.camera().1 + 72).unwrap());
    }

    /// The map loaded and the view built for a square, then the tile in front, across maps.
    #[test]
    fn the_tile_in_front_matches_the_cartridge() {
        use poke_core::sprite::SpriteFacing;
        use crate::systems::overworld::collision::in_front;
        for ((map, x, y, facing), [tile, front_x, front_y, standing], _) in
            crate::fixtures::cases::<(u8, u8, u8, u8), [u8; 4]>(include_str!("../../../fixtures/overworld/tile_in_front.jsonl"))
        {
            let map = Map::from_repr(map).unwrap();
            let header = MapHeader::read(map).unwrap();
            let width = header.width as u16;
            let view = MapView {
                tileset: header.tileset,
                width: header.width,
                height: header.height,
                blocks: tile_block_map(map).unwrap(),
                view: 7 + width + (width + 6) * (y >> 1) as u16 + (x >> 1) as u16,
                x_block: x & 1,
                y_block: y & 1,
                tile_overrides: Vec::new(),
            };
            let tiles = view.tile_map();
            let front = in_front(&tiles, x, y, SpriteFacing::from_repr(facing).unwrap());
            assert_eq!([front.tile, front.x, front.y, tiles[9 * 20 + 8]], [tile, front_x, front_y, standing], "{map} ({x}, {y}) {facing}");
        }
    }

    #[test]
    fn stepping_round_a_square_comes_back_to_the_same_view() {
        let mut view = pallet_town_at_the_door();
        let start = view.clone();
        for (dx, dy) in [(1, 0), (0, 1), (-1, 0), (0, -1), (1, 0), (1, 0), (-1, 0), (-1, 0)] {
            view.step(dx, dy);
        }
        assert_eq!(view, start);
    }
}
