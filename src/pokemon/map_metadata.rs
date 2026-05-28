use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::{MapConnectionDirection, MapHeader, MapHeaderReader, TileSetId};
use crate::pokemon::sprite::{PictureId, Sprite};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::tile::{JumpDirection, MetaTile, WarpEvent};
use crate::ram::ROM;

#[derive(Clone)]
pub struct MapMetadata {
    pub map: Map,
    pub map_header: MapHeader,
    pub map_data: Vec<u8>,
    pub tileset_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    /// Tile IDs from `wTilesetTalkingOverTiles` — counters/desks the player can
    /// interact through by facing them and pressing A (pokered "talking over" mechanic).
    pub talking_over_tiles: HashSet<u8>,
    pub warp_events: Vec<WarpEvent>,
    /// True if the current tileset is listed in the WaterTilesets table,
    /// meaning tile $14/$32/$48 should be treated as water/shore.
    pub is_water_tileset: bool,
    /// Tile ID for tall grass in the current tileset (from `wGrassTile`).
    /// Zero means this map has no grass tile.
    pub grass_tile_id: u8,
    pub connected_strips: Vec<ConnectedMapStrip>,
    /// Maps ledge tile IDs to their jump direction. Only populated for the Overworld tileset,
    /// which is the only tileset where HandleLedges fires.
    pub ledge_tiles: HashMap<u8, JumpDirection>,
    /// Pre-computed tile grid without sprites. Tiles, warps, and connection strips are all
    /// ROM-derived and never change, so this is computed once at construction and cloned
    /// in `meta_tiles` before the sprite overlay is applied.
    pub meta_tiles_base: Vec<MetaTile>,
}

impl MapMetadata {
    pub const BLOCK_TILE_WIDTH: usize = 4; // a block is 4x4 tiles
    pub const BLOCK_TILES: usize = Self::BLOCK_TILE_WIDTH * Self::BLOCK_TILE_WIDTH;
    pub const TILES_PER_META: usize = 2; // a meta tile on the map is 2x2 graphical tiles

    pub fn tile_id(&self, tile_x: usize, tile_y: usize) -> u8 {
        let block_x = tile_x / Self::BLOCK_TILE_WIDTH;
        let block_y = tile_y / Self::BLOCK_TILE_WIDTH;
        let block_index = self.map_data[block_x + block_y * self.map_header.width as usize] as usize;
        let block_offset = block_index * Self::BLOCK_TILES;
        let tile_offset = (tile_x % Self::BLOCK_TILE_WIDTH) + (tile_y % Self::BLOCK_TILE_WIDTH) * Self::BLOCK_TILE_WIDTH;
        self.tileset_data[block_offset + tile_offset]
    }

    pub fn is_empty(&self, tile_x: usize, tile_y: usize) -> bool {
        self.collision_tiles.contains(&self.tile_id(tile_x, tile_y))
    }

    /// Returns true if this sub-tile is a water or shore tile.
    ///
    /// Mirrors `IsNextTileShoreOrWater` from `engine/items/item_effects.asm`.
    pub fn is_water(&self, tile_x: usize, tile_y: usize) -> bool {
        is_water_tile_id(self.tile_id(tile_x, tile_y), self.is_water_tileset, self.map_header.tileset)
    }

    pub fn dimensions(&self) -> MapDimensions {
        MapDimensions {
            meta_height: self.map_header.height as usize * Self::TILES_PER_META,
            meta_width: self.map_header.width as usize * Self::TILES_PER_META,
            north_extra: self.map_header.north_connection.is_some() as usize,
            east_extra: self.map_header.east_connection.is_some() as usize,
            south_extra: self.map_header.south_connection.is_some() as usize,
            west_extra: self.map_header.west_connection.is_some() as usize,
        }
    }

    pub fn meta_tiles(&self, sprites: &[Sprite]) -> Vec<MetaTile> {
        let mut result = self.meta_tiles_base.clone();
        let dimensions = self.dimensions();
        let exp_width = dimensions.full_width();
        let exp_height = dimensions.full_height();

        for sprite in sprites.iter().filter(|s| !s.hidden) {
            let mx = sprite.position.x as usize + dimensions.west_extra;
            let my = sprite.position.y as usize + dimensions.north_extra;
            if mx < exp_width && my < exp_height {
                let idx = mx + my * exp_width;
                // Warps are already in the base and take priority over sprites,
                // matching the original ordering where warps were applied after sprites.
                if !matches!(result[idx], MetaTile::Warp { .. }) {
                    result[idx] = MetaTile::Sprite(sprite.name);
                }
            }
        }

        result
    }

    pub fn build_meta_tiles_base(&self) -> Vec<MetaTile> {
        let dimensions = self.dimensions();

        // Connection strips expand the map by one meta-tile row/column per direction.
        let exp_width = dimensions.full_width();
        let exp_height = dimensions.full_height();

        let mut result = vec![MetaTile::Obstacle; exp_width * exp_height];

        // Fill current map tiles, shifted by the connection offsets.
        //
        // pokered collision checking reads the bottom-left raw tile of the destination 2×2
        // meta-tile in all four movement directions (GetTileAndCoordsInFrontOfPlayer uses
        // lda_coord(8,11), (8,7), (6,9), (10,9) — always the bottom-left of the target
        // meta-tile).  All classification checks therefore use only that sub-tile.
        //
        // Exception: water detection scans all four sub-tiles so that shore-transition blocks
        // (which mix passable and water tile IDs) are conservatively treated as Water.
        let width_tiles  = self.map_header.width  as usize * Self::BLOCK_TILE_WIDTH;
        let height_tiles = self.map_header.height as usize * Self::BLOCK_TILE_WIDTH;
        for tile_y in 0..height_tiles {
            let my = tile_y / Self::TILES_PER_META + dimensions.north_extra;
            for tile_x in 0..width_tiles {
                let mx    = tile_x / Self::TILES_PER_META + dimensions.west_extra;
                let index = mx + my * exp_width;
                if result[index] != MetaTile::Water {
                    if self.is_water(tile_x, tile_y) {
                        result[index] = MetaTile::Water;
                    } else if tile_x % Self::TILES_PER_META == 0 && tile_y % Self::TILES_PER_META == 1 {
                        // Bottom-left sub-tile: the one pokered actually checks.
                        let tile_id = self.tile_id(tile_x, tile_y);
                        if self.map_header.tileset.cut_tree_tile_id() == Some(tile_id) {
                            result[index] = MetaTile::CutTree;
                        } else if let Some(&dir) = self.ledge_tiles.get(&tile_id) {
                            result[index] = MetaTile::Jump(dir);
                        } else if self.grass_tile_id != 0 && tile_id == self.grass_tile_id
                            && self.is_empty(tile_x, tile_y)
                            && matches!(result[index], MetaTile::Obstacle | MetaTile::Empty)
                        {
                            result[index] = MetaTile::Grass;
                        } else if result[index] == MetaTile::Obstacle && self.is_empty(tile_x, tile_y) {
                            result[index] = MetaTile::Empty;
                        } else if result[index] == MetaTile::Obstacle && self.talking_over_tiles.contains(&tile_id) {
                            result[index] = MetaTile::Counter;
                        }
                    }
                }
            }
        }

        for warp in &self.warp_events {
            let mx = warp.position.x as usize + dimensions.west_extra;
            let my = warp.position.y as usize + dimensions.north_extra;
            if mx < exp_width && my < exp_height {
                // Only register the warp if the position is physically accessible
                // (has at least one walkable raw sub-tile).  Warp positions with no
                // walkable sub-tiles are impassable in the game (e.g. the duplicate
                // exit tile at x=4 in ViridianForestSouthGate) and must stay as
                // Obstacle so the BFS does not route to them.
                if result[mx + my * exp_width] != MetaTile::Obstacle {
                    result[mx + my * exp_width] = warp.tile();
                }
            }
        }

        // Fill the extra border rows/columns from connected map strips.
        for strip in &self.connected_strips {
            let strip_meta = strip.strip_length as usize * Self::TILES_PER_META;
            match strip.direction {
                MapConnectionDirection::North | MapConnectionDirection::South => {
                    let x_start = strip.meta_align_offset + dimensions.west_extra;
                    let row = if strip.direction == MapConnectionDirection::North { 0 } else { dimensions.north_extra + dimensions.meta_height };
                    for i in 0..strip_meta {
                        let mx = x_start + i;
                        if mx >= exp_width { break; }
                        result[mx + row * exp_width] = strip.meta_tile_at(i);
                    }
                }
                MapConnectionDirection::West | MapConnectionDirection::East => {
                    let y_start = strip.meta_align_offset + dimensions.north_extra;
                    let col = if strip.direction == MapConnectionDirection::West { 0 } else { dimensions.west_extra + dimensions.meta_width };
                    for i in 0..strip_meta {
                        let my = y_start + i;
                        if my >= exp_height { break; }
                        result[col + my * exp_width] = strip.meta_tile_at(i);
                    }
                }
            }
        }

        result
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MapDimensions {
    pub meta_height: usize,
    pub meta_width: usize,

    /// Extra meta-tile rows/columns added for each connected direction.
    pub north_extra: usize,
    pub east_extra: usize,
    pub south_extra: usize,
    pub west_extra: usize,
}

impl MapDimensions {
    pub fn full_width(&self) -> usize {
        self.meta_width + self.west_extra + self.east_extra
    }

    pub fn full_height(&self) -> usize {
        self.meta_height + self.north_extra + self.south_extra
    }
}

/// Loaded border strip from an adjacent connected map, used to expand the rendered tilemap
/// by one meta-tile row (N/S connections) or column (E/W connections).
#[derive(Clone)]
pub struct ConnectedMapStrip {
    pub direction: MapConnectionDirection,
    pub map: Map,
    /// Block IDs along the single-block-deep border row/column of the connected map.
    pub border_blocks: Vec<u8>,
    pub tileset_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    pub is_water_tileset: bool,
    pub tileset: TileSetId,
    /// Sub-position within each block:
    /// N/S: 0 = top meta-row (south connection), 1 = bottom meta-row (north connection).
    /// E/W: 0 = left meta-col (east connection),  1 = right meta-col (west connection).
    pub block_sub_offset: u8,
    pub strip_length: u8,
    /// Offset (in meta-tiles) along the perpendicular axis where the strip begins.
    /// x-offset for N/S connections, y-offset for E/W connections.
    pub meta_align_offset: usize,
    /// The coordinate in the connected map on the *fixed* axis (i.e. the border itself):
    ///   North → y = connected_height * 2 - 1  (bottom row of connected map)
    ///   South → y = 0                          (top row)
    ///   East  → x = 0                          (left column)
    ///   West  → x = connected_width  * 2 - 1  (right column)
    pub to_border_coord: u8,
    /// Where the strip begins along the *perpendicular* axis in the connected map (meta-tiles).
    /// Strip tile `i` (0-based meta-tile index) lands at perpendicular coord `to_strip_start + i`.
    ///   N/S → x start in connected map = max(0,  x_alignment)
    ///   E/W → y start in connected map = max(0,  y_alignment)
    pub to_strip_start: u8,
    /// Number of blocks to skip at the start of `border_blocks` before applying strip meta-tile
    /// index `i`.  Arises when the strip source pointer (`strip_src`) begins earlier in the
    /// connected map than the tile that aligns with the current map's edge.
    ///
    /// The overworld buffer has a 3-block margin on each side; the strip is placed at buffer
    /// column `_tgt = max(0, alignment + 3)`.  When `_tgt < 3` the strip data starts to the
    /// *left* of the current map's column 0, so the first `(3 - _tgt)` blocks in
    /// `border_blocks` are off-screen and must be skipped.
    ///   N/S: `max(0, min(3, x_alignment / 2))`
    ///   E/W: `max(0, min(3, y_alignment / 2))`
    pub border_blocks_start_offset: usize,
}

impl ConnectedMapStrip {
    /// Returns the MetaTile for position `strip_idx` (0..strip_length*2) in this border strip.
    fn meta_tile_at(&self, strip_idx: usize) -> MetaTile {
        let block_idx = strip_idx / 2 + self.border_blocks_start_offset;
        if block_idx >= self.border_blocks.len() {
            return MetaTile::Obstacle;
        }
        let block_id = self.border_blocks[block_idx];

        // For N/S: strip_idx selects left(0)/right(1) within each block; sub_offset is the row.
        // For E/W: strip_idx selects top(0)/bottom(1) within each block; sub_offset is the col.
        let (sub_col, sub_row) = match self.direction {
            MapConnectionDirection::North | MapConnectionDirection::South =>
                (strip_idx % 2, self.block_sub_offset as usize),
            MapConnectionDirection::East | MapConnectionDirection::West =>
                (self.block_sub_offset as usize, strip_idx % 2),
        };

        // Indices of the four graphical tiles that form this 2×2 meta-tile within the block.
        // tile_offset = tx_in_block + ty_in_block * BLOCK_TILE_WIDTH (4)
        let base = sub_col * 2 + sub_row * 8;
        let tile_indices = [base, base + 1, base + 4, base + 5];
        let block_start = block_id as usize * MapMetadata::BLOCK_TILES;

        let mut has_water = false;
        let mut has_walkable = false;
        for &idx in &tile_indices {
            let pos = block_start + idx;
            if pos >= self.tileset_data.len() {
                continue;
            }
            let tile_id = self.tileset_data[pos];
            if self.is_water_tile(tile_id) {
                has_water = true;
            }
            if self.collision_tiles.contains(&tile_id) {
                has_walkable = true;
            }
        }

        if has_water {
            MetaTile::ConnectionWater(self.map)
        } else if has_walkable {
            // Compute the exact tile in the connected map that this strip position leads to.
            // `strip_idx` is a meta-tile index along the strip (0 = first meta-tile).
            // The perpendicular coordinate in the connected map is `to_strip_start + strip_idx`.
            let perp = self.to_strip_start.saturating_add(strip_idx as u8);
            let to_position = match self.direction {
                MapConnectionDirection::North | MapConnectionDirection::South =>
                    Point8 { x: perp, y: self.to_border_coord },
                MapConnectionDirection::East | MapConnectionDirection::West =>
                    Point8 { x: self.to_border_coord, y: perp },
            };
            MetaTile::Connection { to_map: self.map, to_position }
        } else {
            MetaTile::Obstacle
        }
    }

    fn is_water_tile(&self, tile_id: u8) -> bool {
        is_water_tile_id(tile_id, self.is_water_tileset, self.tileset)
    }
}

/// Returns true if `tile_id` is a water or shore tile for the given tileset.
///
/// Mirrors `IsNextTileShoreOrWater` from `engine/items/item_effects.asm`:
/// - Tile `$14` is the universal water tile.
/// - Tiles `$32` (eastern shore) and `$48` (Safari Zone shore) are shore tiles,
///   **unless** the tileset is `ShipPort` (Vermilion Dock), where `$32` is dock planks.
/// - Returns false for any tileset not listed in the `WaterTilesets` ROM table.
fn is_water_tile_id(tile_id: u8, is_water_tileset: bool, tileset: TileSetId) -> bool {
    const WATER: u8 = 0x14;
    const EASTERN_SHORE: u8 = 0x32;
    const SAFARI_ZONE_EASTERN_SHORE: u8 = 0x48;
    if !is_water_tileset {
        return false;
    }
    if tileset != TileSetId::ShipPort && (tile_id == EASTERN_SHORE || tile_id == SAFARI_ZONE_EASTERN_SHORE) {
        return true;
    }
    tile_id == WATER
}

pub trait MapMetadataReader {
    fn read_map_metadata(&self, map: Map) -> Result<MapMetadata, String>;

    fn read_current_map(&self) -> Result<CurrentMap, String>;
}

/// Pokémon-layer cache for `read_map_metadata`. ROM data never changes during a session,
/// so results are deterministic per map and safe to cache indefinitely.
#[derive(Default)]
pub struct MapMetadataCache(RefCell<HashMap<Map, Arc<MapMetadata>>>);

impl std::fmt::Debug for MapMetadataCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapMetadataCache({} entries)", self.0.borrow().len())
    }
}

impl MapMetadataCache {
    pub fn read_current_map(&self, mmu: &MMU) -> Result<CurrentMap, String> {
        let map = Map::from_repr(mmu.read_pointer(&pokered_symbols::wCurMap))
            .ok_or_else(|| "Invalid map number".to_string())?;
        let player_direction_raw = mmu.read_pointer(&pokered_symbols::wPlayerDirection);
        Ok(CurrentMap {
            metadata: self.read_map(mmu, map)?,
            player_position: Point8 {
                x: mmu.read_pointer(&pokered_symbols::wXCoord),
                y: mmu.read_pointer(&pokered_symbols::wYCoord),
            },
            player_direction: PlayerFacingDirection::from_repr(player_direction_raw)
                .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?,
            sprites: mmu.read_sprites()?,
        })
    }

    pub fn read_map(&self, mmu: &MMU, map: Map) -> Result<Arc<MapMetadata>, String> {
        if let Some(arc) = self.0.borrow().get(&map) {
            return Ok(Arc::clone(arc));
        }
        let arc = Arc::new(mmu.read_map_metadata(map)?);
        self.0.borrow_mut().insert(map, Arc::clone(&arc));
        Ok(arc)
    }
}

impl MapMetadataReader for MMU {

    fn read_map_metadata(&self, map: Map) -> Result<MapMetadata, String> {
        let map_header = self.read_map_header(map)?;

        let ts = self.read_tileset_header(map_header.tileset);
        let collision_tiles = self.read_collision_tiles(ts.coll_ptr);

        let map_data = self.rom_data_from_rom_pointer(&map_header.blocks_pointer(), map_header.height as usize * map_header.width as usize).to_vec();

        let max_block_id = *map_data.iter().max().unwrap() as usize;
        let tileset_data = self.rom_data_from_pointer(ts.bank, ts.blocks_ptr, (max_block_id + 1) * MapMetadata::BLOCK_TILES).to_vec();

        let warp_events = self.read_warp_events(map, &map_header)?;

        let tileset_id = map_header.tileset as u8;
        let water_tilesets = self.rom_data_from_rom_pointer(&pokered_symbols::WaterTilesets, 16);
        let is_water_tileset = water_tilesets.iter()
            .take_while(|&&b| b != 0xFF)
            .any(|&b| b == tileset_id);

        let connected_strips = self.load_connected_strips(&map_header);

        // HandleLedges in pokered only fires for tileset 0 (Overworld); other tilesets have no ledges.
        let ledge_tiles = if map_header.tileset == TileSetId::Overworld {
            self.read_ledge_tiles()
        } else {
            HashMap::new()
        };

        let mut metadata = MapMetadata {
            map,
            map_header,
            map_data,
            tileset_data,
            collision_tiles,
            talking_over_tiles: ts.talking_over_tiles,
            warp_events,
            is_water_tileset,
            grass_tile_id: ts.grass_tile,
            connected_strips,
            ledge_tiles,
            meta_tiles_base: Vec::new(),
        };
        metadata.meta_tiles_base = metadata.build_meta_tiles_base();
        Ok(metadata)
    }

    fn read_current_map(&self) -> Result<CurrentMap, String> {
        let map = Map::from_repr(self.read_pointer(&pokered_symbols::wCurMap))
            .ok_or_else(|| "Invalid map number".to_string())?;
        let player_direction_raw = self.read_pointer(&pokered_symbols::wPlayerDirection);

        Ok(
            CurrentMap {
                metadata: Arc::new(self.read_map_metadata(map)?),
                player_position: Point8 {
                    x: self.read_pointer(&pokered_symbols::wXCoord),
                    y: self.read_pointer(&pokered_symbols::wYCoord),
                },
                player_direction: PlayerFacingDirection::from_repr(player_direction_raw)
                    .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?,
                sprites: self.read_sprites()?,
            }
        )
    }
}

struct TilesetHeader {
    bank: usize,
    blocks_ptr: u16,
    coll_ptr: u16,
    talking_over_tiles: HashSet<u8>,
    grass_tile: u8,
}

impl MMU {
    fn read_warp_events(&self, cur_map: Map, map_header: &MapHeader) -> Result<Vec<WarpEvent>, String> {
        // Read directly from ROM so we get the raw destination byte (including 0xFF / self-ref)
        // before pokémon Red's runtime wLastMap resolution, which becomes stale when
        // navigating between indoor floors (e.g. Red's House 1F → 2F → 1F).
        //
        // Object-data layout: [border_block(1), warp_count(1), warp_entries(4 each), ...]
        // Each warp entry is: [Y, X, dest_warp_id, dest_map_id]
        //   dest_warp_id is stored 0-indexed (the ASM source uses 1-based but subtracts 1 via
        //   the `warp_event` macro: `db \4 - 1`).
        let objects_pointer = map_header.objects_pointer();
        let warp_count = self.read_pointer(&(objects_pointer + 1)) as u16;
        let mut result = vec![];
        for index in 0..warp_count {
            let base = objects_pointer + (2 + index * 4);
            let entry = self.rom_data_from_rom_pointer(&base, 4);
            let raw_map_id = entry[3];
            let dest_warp_id = entry[2] as u16; // 0-indexed into destination map's warp table
            // 0xFF = LAST_MAP sentinel; self-referential (raw_map_id == cur_map) is
            // pokémon Red's building-exit convention meaning the same thing.
            // In both cases the true destination is the outdoor map whose warp table
            // points back to this indoor map.
            let map_id = if raw_map_id == 0xFF || raw_map_id == cur_map as u8 {
                self.find_outdoor_entry_map(cur_map)
                    .ok_or_else(|| format!("No outdoor map found for {cur_map}"))?
            } else {
                Map::from_repr(raw_map_id)
                    .ok_or_else(|| format!("Invalid map number {raw_map_id}"))?
            };
            let destination_position = self.read_destination_warp_position(map_id, dest_warp_id)?;
            result.push(WarpEvent {
                position: Point8 { y: entry[0], x: entry[1] },
                destination_map: map_id,
                destination_position,
            });
        }
        Ok(result)
    }

    /// Reads the ROM `Tilesets` table entry for `tileset` and returns the derived header fields.
    ///
    /// Tilesets entry layout (12 bytes each):
    ///   [0]     bank
    ///   [1..2]  blocks_ptr (LE)
    ///   [3..4]  gfx_ptr    (LE, unused)
    ///   [5..6]  coll_ptr   (LE)
    ///   [7..9]  3 counter/talking-over tile IDs (0xFF = unused)
    ///   [10]    grass tile ID (0xFF = no grass)
    ///   [11]    animation type (unused here)
    fn read_tileset_header(&self, tileset: TileSetId) -> TilesetHeader {
        const TILESET_ENTRY_SIZE: u16 = 12;
        let entry = pokered_symbols::Tilesets + tileset as u16 * TILESET_ENTRY_SIZE;
        let bank       = self.read_pointer(&entry) as usize;
        let blocks_ptr = self.read_pointer_u16_le(&(entry + 1));
        let coll_ptr   = self.read_pointer_u16_le(&(entry + 5));
        let talking_over_tiles = (0u16..3)
            .map(|i| self.read_pointer(&(entry + 7 + i)))
            .filter(|&b| b != 0xFF)
            .collect();
        let grass_tile = self.read_pointer(&(entry + 10));
        TilesetHeader { bank, blocks_ptr, coll_ptr, talking_over_tiles, grass_tile }
    }

    /// Returns the tile position (`Point8 { y, x }`) that the player lands on after taking a warp
    /// that targets `dest_map` at warp-table index `dest_warp_id` (0-indexed).
    ///
    /// The destination position is simply the `[Y, X]` of the `dest_warp_id`-th entry in
    /// `dest_map`'s object data warp table — the same value the game engine reads via
    /// `LoadDestinationWarpPosition` (home/overworld.asm) indexed by `wDestinationWarpID`.
    ///
    /// The `warp_event` ASM macro emits `\4 - 1` for the dest-warp field, so the raw ROM byte
    /// is already 0-indexed.  This function accepts it as-is.
    fn read_destination_warp_position(&self, dest_map: Map, dest_warp_id: u16) -> Result<Point8, String> {
        let header = self.read_map_header(dest_map)?;
        let objects_pointer = header.objects_pointer();
        let warp_count = self.read_pointer(&(objects_pointer + 1)) as u16;
        if dest_warp_id >= warp_count {
            return Err(format!(
                "dest_warp_id {dest_warp_id} out of range (map {dest_map} has {warp_count} warps)"
            ));
        }
        let base = objects_pointer + (2 + dest_warp_id * 4);
        let dest_entry = self.rom_data_from_rom_pointer(&base, 4);
        Ok(Point8 { y: dest_entry[0], x: dest_entry[1] })
    }

    /// Scans every Overworld-tileset map in ROM for a warp tile whose destination map
    /// equals `indoor_map`.  Returns the first match — this is the outdoor map the
    /// player should be returned to when they step on a self-referential / LAST_MAP exit warp.
    fn find_outdoor_entry_map(&self, indoor_map: Map) -> Option<Map> {
        let map_banks = self.rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT);
        (0..Map::COUNT)
            .filter_map(|id| {
                let outdoor_map = Map::from_repr(id as u8)?;
                let header      = self.read_map_header(outdoor_map).ok()?;
                if header.tileset != TileSetId::Overworld { return None; }
                let bank        = map_banks[id] as usize;
                let warp_count  = self.rom_data_from_pointer(bank, header.objects_address + 1, 1)[0] as u16;
                for wi in 0..warp_count {
                    let entry = self.rom_data_from_pointer(bank, header.objects_address + 2 + wi * 4, 4);
                    if entry[3] == indoor_map as u8 {
                        return Some(outdoor_map);
                    }
                }
                None
            })
            .next()
    }

    /// Reads the `LedgeTiles` ROM table and returns a map of tile_id → JumpDirection.
    ///
    /// The table has 4-byte entries terminated by 0xFF:
    ///   [facing_direction, tile_under_player, tile_in_front, jump_direction_flags]
    /// `tile_in_front` (byte 2) is the ledge tile ID; `jump_direction_flags` (byte 3) encodes
    /// the direction: 0x80=south, 0x20=west, 0x10=east (same bit layout as wPlayerMovingDirection).
    fn read_ledge_tiles(&self) -> HashMap<u8, JumpDirection> {
        let data = self.rom_data_from_rom_pointer(&pokered_symbols::LedgeTiles, 64);
        let mut result = HashMap::new();
        let mut i = 0;
        while i + 3 < data.len() {
            if data[i] == 0xFF { break; }
            let tile_in_front   = data[i + 2];
            let dir_flags       = data[i + 3];
            let dir = match dir_flags {
                0x80 => JumpDirection::South,
                0x20 => JumpDirection::West,
                0x10 => JumpDirection::East,
                _    => { i += 4; continue; }
            };
            result.insert(tile_in_front, dir);
            i += 4;
        }
        result
    }

    /// Reads an FF-terminated list of walkable tile IDs from a bank-0 ROM address.
    fn read_collision_tiles(&self, ptr: u16) -> HashSet<u8> {
        let mut tiles = HashSet::new();
        for index in 0..256u16 {
            let byte = self.read(ptr + index);
            if byte == 0xFF { break; }
            tiles.insert(byte);
        }
        tiles
    }

    fn load_connected_strips(&self, map_header: &MapHeader) -> Vec<ConnectedMapStrip> {
        // Each entry in the Tilesets ROM table is 12 bytes:
        let all_map_banks: Vec<u8> = self
            .rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT)
            .to_vec();
        let water_tilesets: Vec<u8> = self
            .rom_data_from_rom_pointer(&pokered_symbols::WaterTilesets, 16)
            .to_vec();

        map_header.connections()
            .into_iter()
            .filter_map(|connection| {
                let connected_map_bank = all_map_banks[connection.map as usize] as usize;
                let connected_header = self.read_map_header(connection.map).ok()?;

                // Read the single block row/column that borders the current map.
                let (border_blocks, block_sub_offset): (Vec<u8>, u8) = match connection.direction {
                    MapConnectionDirection::South => {
                        // First block row of the connected map.
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            connection.strip_src,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 0)
                    }
                    MapConnectionDirection::North => {
                        // strip_src points to the start of the 3-block-deep strip
                        // (connected_height − 3 rows in). The border row is the 3rd row.
                        let addr = connection.strip_src + 2 * connection.strip_length as u16;
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            addr,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 1)
                    }
                    MapConnectionDirection::East => {
                        // strip_src points to column 0 of each row; stride by connected_map_width.
                        let blocks = (0..connection.strip_length as u16)
                            .map(|row| {
                                self.rom_data_from_pointer(
                                    connected_map_bank,
                                    connection.strip_src + row * connection.connected_map_width as u16,
                                    1,
                                )[0]
                            })
                            .collect();
                        (blocks, 0)
                    }
                    MapConnectionDirection::West => {
                        // strip_src points to column (width−3); border column is +2 (column width−1).
                        let blocks = (0..connection.strip_length as u16)
                            .map(|row| {
                                self.rom_data_from_pointer(
                                    connected_map_bank,
                                    connection.strip_src + row * connection.connected_map_width as u16 + 2,
                                    1,
                                )[0]
                            })
                            .collect();
                        (blocks, 1)
                    }
                };

                if border_blocks.is_empty() {
                    return None;
                }
                let max_block_id = *border_blocks.iter().max().unwrap() as usize;

                let tileset = connected_header.tileset;
                let ts = self.read_tileset_header(tileset);

                let tileset_data = self.rom_data_from_pointer(
                    ts.bank,
                    ts.blocks_ptr,
                    (max_block_id + 1) * MapMetadata::BLOCK_TILES,
                ).to_vec();

                let collision_tiles = self.read_collision_tiles(ts.coll_ptr);

                let tileset_id_byte = tileset as u8;
                let is_water_tileset = water_tilesets.iter()
                    .take_while(|&&b| b != 0xFF)
                    .any(|&b| b == tileset_id_byte);

                let meta_align_offset = match connection.direction {
                    MapConnectionDirection::North | MapConnectionDirection::South =>
                        (-(connection.x_alignment as i32)).max(0) as usize,
                    MapConnectionDirection::East | MapConnectionDirection::West =>
                        (-(connection.y_alignment as i32)).max(0) as usize,
                };

                // When `strip_src` points into the connected map at an offset > 0 (i.e. some blocks
                // to the left/above the current map's edge are included), the strip data starts
                // earlier than the tile that aligns with the current map's column/row 0.
                // The overworld buffer has a 3-block margin, so `_tgt = max(0, alignment+3)` blocks
                // of the strip precede the current map.  When _tgt < 3, the first (3-_tgt) blocks
                // in border_blocks are off-screen and must be skipped when looking up meta tiles.
                //   N/S: offset = max(0, min(3, x_alignment / 2))
                //   E/W: offset = max(0, min(3, y_alignment / 2))
                let border_blocks_start_offset = match connection.direction {
                    MapConnectionDirection::North | MapConnectionDirection::South =>
                        (connection.x_alignment as i32 / 2).max(0).min(3) as usize,
                    MapConnectionDirection::East | MapConnectionDirection::West =>
                        (connection.y_alignment as i32 / 2).max(0).min(3) as usize,
                };

                // After a connection transition the game engine sets wYCoord/wXCoord to the raw
                // alignment values.  However, `MetaTileMap::new()` converts raw wXCoord/wYCoord
                // into **expanded** meta-tile coordinates by adding `north_extra` and `west_extra`
                // of the destination map (one extra row/column per connection strip).  To make
                // `to_position` match the expanded coordinate that `MetaTileMap.player_position`
                // reports after landing, we must add the connected map's extras here.
                //
                // `to_border_coord` is the fixed-axis coordinate in the connected map's **expanded**
                // coordinate space where the player lands:
                //   North → y = y_alignment + connected_north_extra
                //   South → y = connected_north_extra   (= 0 if no north connection, else 1)
                //   East  → x = connected_west_extra    (= 0 if no west connection, else 1)
                //   West  → x = x_alignment + connected_west_extra
                //
                // `to_strip_start` is where the strip begins along the perpendicular axis in the
                // connected map's expanded coordinate space (raw start + connected extra offset):
                //   N/S   → x_start = max(0, x_alignment) + connected_west_extra
                //   E/W   → y_start = max(0, y_alignment) + connected_north_extra
                let connected_north_extra = if connected_header.north_connection.is_some() { 1u8 } else { 0u8 };
                let connected_west_extra  = if connected_header.west_connection.is_some()  { 1u8 } else { 0u8 };
                let (to_border_coord, to_strip_start) = match connection.direction {
                    MapConnectionDirection::North =>
                        (connection.y_alignment as u8 + connected_north_extra,
                         connection.x_alignment.max(0) as u8 + connected_west_extra),
                    MapConnectionDirection::South =>
                        (connected_north_extra,
                         connection.x_alignment.max(0) as u8 + connected_west_extra),
                    MapConnectionDirection::East =>
                        (connected_west_extra,
                         connection.y_alignment.max(0) as u8 + connected_north_extra),
                    MapConnectionDirection::West =>
                        (connection.x_alignment as u8 + connected_west_extra,
                         connection.y_alignment.max(0) as u8 + connected_north_extra),
                };

                Some(ConnectedMapStrip {
                    direction: connection.direction,
                    map: connection.map,
                    border_blocks,
                    tileset_data,
                    collision_tiles,
                    is_water_tileset,
                    tileset,
                    block_sub_offset,
                    strip_length: connection.strip_length,
                    meta_align_offset,
                    to_border_coord,
                    to_strip_start,
                    border_blocks_start_offset,
                })
            })
            .collect()
    }

    fn read_sprites(&self) -> Result<Vec<Sprite>, String> {
        let map = Map::from_repr(self.read_pointer(&pokered_symbols::wCurMap)).ok_or_else(|| "Invalid map number".to_string())?;
        let map_sprites = map.sprites();

        let missable_objects = self.read_pointer_vec(
            &pokered_symbols::wMissableObjectFlags,
            (pokered_symbols::wMissableObjectFlagsEnd.address - pokered_symbols::wMissableObjectFlags.address) as usize
        );

        let mut sprites: Vec<Sprite> = Vec::new();
        for index in 1..=0xFu16 { // do not read index=0 as it is always the player
            let offset = index << 4;
            let picture_id = match PictureId::from_repr(self.read(pokered_symbols::wSpriteDataStart.address | offset)) {
                Some(picture_id) => picture_id,
                None => continue
            };
            let map_sprite = match map_sprites.get(index as usize - 1) {
                Some(map_sprite) => map_sprite,
                None => continue
            };

            let hidden = match map_sprite.hidden_object_id {
                Some(hidden_object_bit) => {
                    let mask = 1 << hidden_object_bit % 8;
                    (missable_objects[(hidden_object_bit / 8) as usize] & mask) == mask
                }
                None => false,
            };

            let sprite_image_index = self.read(pokered_symbols::wSpritePlayerStateData1ImageIndex.address | offset);

            let sprite = Sprite {
                index: index as u8,
                picture_id,
                position: if picture_id == PictureId::Red {
                    // Read player position from the map state
                    Point8 {
                        x: self.read_pointer(&pokered_symbols::wXCoord),
                        y: self.read_pointer(&pokered_symbols::wYCoord)
                    }
                } else {
                    Point8 {
                        x: self.read(pokered_symbols::wSpritePlayerStateData2MapX.address | offset) - 4,
                        y: self.read(pokered_symbols::wSpritePlayerStateData2MapY.address | offset) - 4
                    }
                },
                on_screen: sprite_image_index != 0xFF,
                hidden,
                name: map_sprite.name
            };
            sprites.push(sprite);
        }
        Ok(sprites)
    }
}


#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PlayerFacingDirection {
    #[default]
    Up = 8,
    Down = 4,
    Left = 2,
    Right = 1,
}

impl Into<JoypadButton> for PlayerFacingDirection {
    fn into(self) -> JoypadButton {
        match self {
            PlayerFacingDirection::Up => JoypadButton::Up,
            PlayerFacingDirection::Down => JoypadButton::Down,
            PlayerFacingDirection::Left => JoypadButton::Left,
            PlayerFacingDirection::Right => JoypadButton::Right,
        }
    }
}

pub struct CurrentMap {
    pub player_position: Point8,
    pub player_direction: PlayerFacingDirection,
    pub sprites: Vec<Sprite>,
    pub metadata: Arc<MapMetadata>,
}

impl CurrentMap {
    pub fn meta_tiles(&self) -> Vec<MetaTile> {
        self.metadata.meta_tiles(&self.sprites)
    }
}

#[cfg(test)]
mod test {
    use crate::pokemon::roms::POKERED;
    use crate::pokemon::tile_map::MetaTileMap;
    use super::*;

    /// Verifies that `WarpEvent::destination_position` is resolved correctly from ROM data by
    /// cross-checking with known map objects in the pokered disassembly.
    ///
    /// Key: the `warp_event` ASM macro takes args `(x, y, map, warp_id)` and emits `db y, x, …`,
    /// so the ROM byte order is [Y, X, dest_warp_id, dest_map].  All expected values are derived
    /// directly from pokered/data/maps/objects/*.asm.
    ///
    /// Data sources:
    ///   PalletTown.asm:
    ///     warp_event  5,  5, REDS_HOUSE_1F, 1  →  x=5,  y=5  → ROM [Y=5,  X=5,  dest_warp=0]
    ///     warp_event 13,  5, BLUES_HOUSE,   1  →  x=13, y=5  → ROM [Y=5,  X=13, dest_warp=0]
    ///     warp_event 12, 11, OAKS_LAB,      2  →  x=12, y=11 → ROM [Y=11, X=12, dest_warp=1]
    ///   RedsHouse1F.asm:
    ///     warp_event  2,  7, LAST_MAP,       1  → x=2, y=7 → ROM [Y=7, X=2, dest_warp=0]
    ///     warp_event  3,  7, LAST_MAP,       1  → x=3, y=7 → ROM [Y=7, X=3, dest_warp=0]
    ///     warp_event  7,  1, REDS_HOUSE_2F,  1  → x=7, y=1 → ROM [Y=1, X=7, dest_warp=0]
    ///   RedsHouse2F.asm:
    ///     warp_event  7,  1, REDS_HOUSE_1F,  3  → x=7, y=1 → ROM [Y=1, X=7, dest_warp=2]
    ///   OaksLab.asm:
    ///     warp_event  4, 11, LAST_MAP, 3  → x=4,  y=11 → ROM [Y=11, X=4, dest_warp=2]
    ///     warp_event  5, 11, LAST_MAP, 3  → x=5,  y=11 → ROM [Y=11, X=5, dest_warp=2]  ← index 1
    #[test]
    fn test_warp_event_destination_position() {

        let mmu = MMU::from_rom(POKERED).unwrap();

        // ── Pallet Town ──────────────────────────────────────────────────────────
        use crate::pokemon::symbols::pokered_symbols;
        let pt_header = mmu.read_map_header(Map::PalletTown).unwrap();
        let pt_warps  = mmu.read_warp_events(Map::PalletTown, &pt_header).unwrap();
        assert_eq!(pt_warps.len(), 3);

        // warp 0: x=5, y=5 → source tile (y=5, x=5); dest=RedsHouse1F[0]
        // RedsHouse1F warp[0] = warp_event 2, 7, … → ROM [Y=7, X=2] → {y:7, x:2}
        assert_eq!(pt_warps[0].position,             Point8 { y: 5, x: 5  });
        assert_eq!(pt_warps[0].destination_map,      Map::RedsHouse1F);
        assert_eq!(pt_warps[0].destination_position, Point8 { y: 7, x: 2  });

        // warp 1: x=13, y=5 → source tile (y=5, x=13); dest=BluesHouse[0]
        assert_eq!(pt_warps[1].position,        Point8 { y: 5,  x: 13 });
        assert_eq!(pt_warps[1].destination_map, Map::BluesHouse);

        // warp 2: x=12, y=11 → source tile (y=11, x=12); dest=OaksLab[1]
        // OaksLab warp[1] = warp_event 5, 11, … → ROM [Y=11, X=5] → {y:11, x:5}
        assert_eq!(pt_warps[2].position,             Point8 { y: 11, x: 12 });
        assert_eq!(pt_warps[2].destination_map,      Map::OaksLab);
        assert_eq!(pt_warps[2].destination_position, Point8 { y: 11, x: 5  });

        // ── Red's House 1F ────────────────────────────────────────────────────────
        let rh1_header = mmu.read_map_header(Map::RedsHouse1F).unwrap();
        let rh1_warps  = mmu.read_warp_events(Map::RedsHouse1F, &rh1_header).unwrap();
        assert_eq!(rh1_warps.len(), 3);

        // warps 0 & 1: LAST_MAP exits → PalletTown[0]
        // PalletTown warp[0] = warp_event 5, 5, … → ROM [Y=5, X=5] → {y:5, x:5}
        assert_eq!(rh1_warps[0].position,             Point8 { y: 7, x: 2 });
        assert_eq!(rh1_warps[0].destination_map,      Map::PalletTown);
        assert_eq!(rh1_warps[0].destination_position, Point8 { y: 5, x: 5 });

        assert_eq!(rh1_warps[1].position,             Point8 { y: 7, x: 3 });
        assert_eq!(rh1_warps[1].destination_map,      Map::PalletTown);
        assert_eq!(rh1_warps[1].destination_position, Point8 { y: 5, x: 5 });

        // warp 2: x=7, y=1 → source tile (y=1, x=7); dest=RedsHouse2F[0]
        // RedsHouse2F warp[0] = warp_event 7, 1, … → ROM [Y=1, X=7] → {y:1, x:7}
        assert_eq!(rh1_warps[2].position,             Point8 { y: 1, x: 7 });
        assert_eq!(rh1_warps[2].destination_map,      Map::RedsHouse2F);
        assert_eq!(rh1_warps[2].destination_position, Point8 { y: 1, x: 7 });

        // ── Red's House 2F ────────────────────────────────────────────────────────
        // warp_event 7, 1, REDS_HOUSE_1F, 3 → stored dest_warp_id=2
        // RedsHouse1F[2] = warp_event 7, 1, REDS_HOUSE_2F, 1 → ROM [Y=1, X=7] → {y:1, x:7}
        let rh2_header = mmu.read_map_header(Map::RedsHouse2F).unwrap();
        let rh2_warps  = mmu.read_warp_events(Map::RedsHouse2F, &rh2_header).unwrap();
        assert_eq!(rh2_warps.len(), 1);
        assert_eq!(rh2_warps[0].position,             Point8 { y: 1, x: 7 });
        assert_eq!(rh2_warps[0].destination_map,      Map::RedsHouse1F);
        assert_eq!(rh2_warps[0].destination_position, Point8 { y: 1, x: 7 });
    }

    /// Verifies that every `MetaTile::Connection` tile in a strip carries the correct
    /// `to_position` — i.e. the **expanded** tile coordinate in the connected map the player
    /// lands on (matching what `MetaTileMap.player_position` reports after the transition).
    ///
    /// The expanded coordinate = raw wXCoord/wYCoord + connected_north_extra/west_extra.
    ///
    /// After a map transition `CheckMapConnections` sets `wYCoord`/`wXCoord` to the raw
    /// alignment values.  `MetaTileMap::new()` then adds `north_extra`/`west_extra` for the
    /// destination map (one extra row/column per connected direction) to obtain the expanded
    /// player position.  `to_position` must match that expanded coordinate.
    ///
    /// PalletTown north → Route1 (10×18 blocks, x_alignment=0, y_alignment=35):
    ///   Route1 has north_extra=1 (connects north to ViridianCity), west_extra=0
    ///   raw wYCoord after transition = 35; expanded y = 35 + 1 = 36
    ///   strip_idx 0 → connected x=0, y=36  (expanded bottom row of Route1)
    ///   strip_idx 19 → connected x=19, y=36
    ///
    /// CeladonCity east → Route7 (10×9 blocks, y_alignment=-8, x_alignment=0):
    ///   Route7 has west_extra=1 (west connection to CeladonCity), north_extra=0
    ///   raw wXCoord after transition = 0; expanded x = 0 + 1 = 1
    ///   strip_idx 0 → connected x=1, y=0  (expanded left column of Route7)
    ///   strip_idx 17 → connected x=1, y=17
    ///
    /// CeladonCity west → Route16 (20×9 blocks, y_alignment=-8, x_alignment=39):
    ///   Route16 has west_extra=0 (no west connection), north_extra=0
    ///   raw wXCoord after transition = 39; expanded x = 39 + 0 = 39
    ///   strip_idx 0 → connected x=39, y=0  (unchanged — no extras)
    #[test]
    fn test_connection_tile_to_position() {
        let mmu = MMU::from_rom(POKERED).unwrap();

        // ── PalletTown north → Route1 ─────────────────────────────────────────
        // y_alignment=35; Route1 has north_extra=1 → expanded y = 36.
        // x_alignment=0; Route1 has west_extra=0 → x = strip_idx.
        let pt_meta = mmu.read_map_metadata(Map::PalletTown).unwrap();
        let north_strip = pt_meta.connected_strips.iter()
            .find(|s| s.map == Map::Route1)
            .expect("PalletTown should have a north strip to Route1");

        for i in 0..north_strip.strip_length as usize * 2 {
            let tile = north_strip.meta_tile_at(i);
            if let MetaTile::Connection { to_map, to_position } = tile {
                assert_eq!(to_map, Map::Route1, "strip idx {i}: wrong map");
                assert_eq!(to_position.y, 36, "strip idx {i}: should land on expanded bottom row of Route1 (y_align=35 + north_extra=1)");
                assert_eq!(to_position.x, i as u8, "strip idx {i}: x should equal strip index");
            }
        }

        // ── CeladonCity east → Route7 ─────────────────────────────────────────
        // x_alignment=0; Route7 has west_extra=1 → expanded x = 1.
        // y_alignment=-8 (.max(0)=0); Route7 north_extra=0 → y = strip_idx.
        let celadon_meta = mmu.read_map_metadata(Map::CeladonCity).unwrap();
        let east_strip = celadon_meta.connected_strips.iter()
            .find(|s| s.map == Map::Route7)
            .expect("CeladonCity should have an east strip to Route7");

        for i in 0..east_strip.strip_length as usize * 2 {
            let tile = east_strip.meta_tile_at(i);
            if let MetaTile::Connection { to_map, to_position } = tile {
                assert_eq!(to_map, Map::Route7, "strip idx {i}: wrong map");
                assert_eq!(to_position.x, 1, "strip idx {i}: should land on expanded left column of Route7 (x_align=0 + west_extra=1)");
                assert_eq!(to_position.y, i as u8, "strip idx {i}: y should equal strip index");
            }
        }

        // ── CeladonCity west → Route16 ────────────────────────────────────────
        let west_strip = celadon_meta.connected_strips.iter()
            .find(|s| s.map == Map::Route16)
            .expect("CeladonCity should have a west strip to Route16");

        for i in 0..west_strip.strip_length as usize * 2 {
            let tile = west_strip.meta_tile_at(i);
            if let MetaTile::Connection { to_map, to_position } = tile {
                assert_eq!(to_map, Map::Route16, "strip idx {i}: wrong map");
                assert_eq!(to_position.x, 39, "strip idx {i}: should land on Route16 right column (20*2-1)");
                assert_eq!(to_position.y, i as u8, "strip idx {i}: y should equal strip index");
            }
        }
    }

    #[test]
    fn dump_route1_viridian_connection_data() {
        let mmu = MMU::from_rom(POKERED).unwrap();
        let r1 = mmu.read_map_header(Map::Route1).unwrap();
        println!("Route1: height={} width={}", r1.height, r1.width);
        if let Some(c) = r1.north_connection {
            println!("  Route1 north→{:?}: y_align={} x_align={} strip_len={} connected_w={}", c.map, c.y_alignment, c.x_alignment, c.strip_length, c.connected_map_width);
        }
        if let Some(c) = r1.south_connection {
            println!("  Route1 south→{:?}: y_align={} x_align={} strip_len={} connected_w={}", c.map, c.y_alignment, c.x_alignment, c.strip_length, c.connected_map_width);
        }
        let vc = mmu.read_map_header(Map::ViridianCity).unwrap();
        println!("ViridianCity: height={} width={}", vc.height, vc.width);
        if let Some(c) = vc.north_connection {
            println!("  ViridianCity north→{:?}: y_align={} x_align={} strip_len={}", c.map, c.y_alignment, c.x_alignment, c.strip_length);
        }
        if let Some(c) = vc.south_connection {
            println!("  ViridianCity south→{:?}: y_align={} x_align={} strip_len={}", c.map, c.y_alignment, c.x_alignment, c.strip_length);
        }
        let r2 = mmu.read_map_header(Map::Route2).unwrap();
        println!("Route2: height={} width={}", r2.height, r2.width);
        if let Some(c) = r2.south_connection {
            println!("  Route2 south→{:?}: y_align={} x_align={} strip_len={}", c.map, c.y_alignment, c.x_alignment, c.strip_length);
        }
    }

    #[test]
    fn test_route2_connection_to_viridian_city() {
        let mmu = MMU::from_rom(POKERED).unwrap();
        let map = mmu.read_map_metadata(Map::Route2).unwrap();
        let current_map = CurrentMap {
            player_position: Point8 { x: 8, y: 72 },
            player_direction: PlayerFacingDirection::Up,
            sprites: vec![],
            metadata: Arc::new(map),
        };
        let tile_map = MetaTileMap::new(&current_map);
        println!("{}", tile_map);

        tile_map.actions().into_iter()
            .find(|a| matches!(a.tile, MetaTile::Connection { to_map, .. } if to_map == Map::ViridianCity))
            .expect("Route2 should have a connection to ViridianCity");

    }
}

