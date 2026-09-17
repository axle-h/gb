//! `UsedCut`'s two pure parts: what CUT can take down, and the block a cut one leaves behind.

use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_symbols;
use crate::systems::map_data::MAP_BORDER;
use super::map_view::MapView;

/// `wCutTile` for grass, the one cuttable tile that is not a tree.
pub const CUT_GRASS: u8 = 0x52;

/// `UsedCut`'s check on `wTileInFrontOfPlayer`, answering `wCutTile`. Only the overworld has grass,
/// and only the overworld and the gyms have a tree.
pub fn cut_tile(tileset: TileSetId, in_front: u8) -> Option<u8> {
    if tileset == TileSetId::Overworld && in_front == CUT_GRASS {
        return Some(in_front);
    }
    (tileset.cut_tree_tile_id() == Some(in_front)).then_some(in_front)
}

/// `CutTreeBlockSwaps`: the block a block holding a cut tree becomes. A block that is not in the
/// table is left as it is, which is how the gym's tree and the grass keep theirs.
pub fn cut_tree_block_swap(block: u8) -> Option<u8> {
    rom_slice(pokered_symbols::CutTreeBlockSwaps)
        .chunks_exact(2)
        .take_while(|row| row[0] != 0xFF)
        .find(|row| row[0] == block)
        .map(|row| row[1])
}

/// `ReplaceTreeTileBlock`: the block holding the tile in front of the player, as an index into the
/// view's buffer. The view pointer's own block is the one under the player's top-left quarter, so
/// which of the four blocks around it is meant depends on the half of it the player stands in.
pub fn tree_tile_block(view: &MapView, facing: SpriteFacing) -> usize {
    let stride = view.stride() as usize;
    let (rows, column) = match facing {
        SpriteFacing::Down if view.y_block == 0 => (2, 2),
        SpriteFacing::Down => (3, 2),
        SpriteFacing::Up if view.y_block == 0 => (1, 2),
        SpriteFacing::Up => (2, 2),
        SpriteFacing::Left if view.x_block == 0 => (2, 1),
        SpriteFacing::Left => (2, 2),
        SpriteFacing::Right if view.x_block == 0 => (2, 2),
        SpriteFacing::Right => (2, 3),
    };
    view.view as usize + stride * rows + column
}

/// `ReplaceTreeTileBlock` and the `RedrawMapView` after it: the tree's block swapped for the one
/// without it.
pub fn replace_tree_tile_block(view: &mut MapView, facing: SpriteFacing) {
    let at = tree_tile_block(view, facing);
    if let Some(block) = view.blocks.get(at).copied().and_then(cut_tree_block_swap) {
        view.blocks[at] = block;
    }
}

/// Where a block sits in the buffer, counted in blocks from the map's own top-left corner.
pub fn block_coords(view: &MapView, at: usize) -> (u8, u8) {
    let stride = view.stride() as usize;
    ((at % stride - MAP_BORDER as usize) as u8, (at / stride - MAP_BORDER as usize) as u8)
}

#[cfg(test)]
mod tests {
    use poke_core::map::Map;
    use poke_core::map_header::MapHeader;
    use super::*;
    use crate::systems::map_data::tile_block_map;

    fn view_at(map: Map, x: u8, y: u8) -> MapView {
        let header = MapHeader::read(map).unwrap();
        let width = header.width as u16;
        MapView {
            tileset: header.tileset,
            width: header.width,
            height: header.height,
            blocks: tile_block_map(map).unwrap(),
            view: 7 + width + (width + 6) * (y >> 1) as u16 + (x >> 1) as u16,
            x_block: x & 1,
            y_block: y & 1,
        }
    }

    #[test]
    fn only_a_tree_or_grass_can_be_cut() {
        assert_eq!(cut_tile(TileSetId::Overworld, 0x3D), Some(0x3D));
        assert_eq!(cut_tile(TileSetId::Overworld, CUT_GRASS), Some(CUT_GRASS));
        assert_eq!(cut_tile(TileSetId::Overworld, 0x50), None);
        assert_eq!(cut_tile(TileSetId::Gym, 0x50), Some(0x50));
        // The gym's tree is the only thing a gym can cut: its grass tile means nothing there.
        assert_eq!(cut_tile(TileSetId::Gym, CUT_GRASS), None);
        assert_eq!(cut_tile(TileSetId::Cavern, 0x3D), None);
    }

    #[test]
    fn a_cut_tree_s_block_is_the_one_beside_it_in_the_table() {
        assert_eq!(cut_tree_block_swap(0x32), Some(0x6D));
        assert_eq!(cut_tree_block_swap(0x0B), Some(0x0A));
        assert_eq!(cut_tree_block_swap(0x3D), Some(0x36));
        assert_eq!(cut_tree_block_swap(0xFF), None);
        assert_eq!(cut_tree_block_swap(0x00), None);
    }

    /// One block of a map approached from each of the four sides, from either half of the square
    /// alongside: every approach names that one block.
    #[test]
    fn every_approach_to_one_block_names_it() {
        let (bx, by) = (5u8, 5u8);
        let approaches = [
            (2 * bx, 2 * by + 2, SpriteFacing::Up), (2 * bx + 1, 2 * by + 2, SpriteFacing::Up),
            (2 * bx, 2 * by - 1, SpriteFacing::Down), (2 * bx + 1, 2 * by - 1, SpriteFacing::Down),
            (2 * bx - 1, 2 * by, SpriteFacing::Right), (2 * bx - 1, 2 * by + 1, SpriteFacing::Right),
            (2 * bx + 2, 2 * by, SpriteFacing::Left), (2 * bx + 2, 2 * by + 1, SpriteFacing::Left),
        ];
        for (x, y, facing) in approaches {
            let view = view_at(Map::Route8, x, y);
            assert_eq!(block_coords(&view, tree_tile_block(&view, facing)), (bx, by), "({x}, {y}) {facing}");
        }
    }

    /// Route 8's trees, whose blocks all have a swap: cutting one really does take the tree out.
    #[test]
    fn route_8_s_cut_trees_all_have_a_block_without_the_tree() {
        let view = view_at(Map::Route8, 0, 0);
        let trees: Vec<u8> = view.blocks.iter().copied()
            .filter(|&block| cut_tree_block_swap(block).is_some())
            .collect();
        assert!(!trees.is_empty(), "Route 8 has cuttable trees");
        for block in trees {
            assert_ne!(cut_tree_block_swap(block), Some(block), "block {block:#X}");
        }
    }
}

