use serde::{Deserialize, Serialize};

pub const SCREEN_TILES_X: usize = 20;
pub const SCREEN_TILES_Y: usize = 18;

/// `wTileMap`, as an output. Tile ids are the cartridge's, so charmap bytes draw as themselves. A
/// cell nothing has drawn to, or that has been `uncover`ed, shows the map beneath.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurface {
    tiles: Vec<u8>,
    covered: Vec<bool>,
}

impl Default for UiSurface {
    fn default() -> Self {
        let cells = SCREEN_TILES_X * SCREEN_TILES_Y;
        Self { tiles: vec![Self::BLANK; cells], covered: vec![false; cells] }
    }
}

impl UiSurface {
    pub const BLANK: u8 = 0x7F;

    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.tiles[Self::index(x, y)]
    }

    pub fn set(&mut self, x: usize, y: usize, tile: u8) {
        let index = Self::index(x, y);
        self.tiles[index] = tile;
        self.covered[index] = true;
    }

    /// The tile drawn at `(x, y)`, if anything covers the map there.
    pub fn cover(&self, x: usize, y: usize) -> Option<u8> {
        let index = Self::index(x, y);
        self.covered[index].then_some(self.tiles[index])
    }

    pub fn uncover(&mut self, x: usize, y: usize, width: usize, height: usize) {
        for row in y..y + height {
            for column in x..x + width {
                let index = Self::index(column, row);
                self.covered[index] = false;
                self.tiles[index] = Self::BLANK;
            }
        }
    }

    pub fn row(&self, y: usize) -> &[u8] {
        &self.tiles[y * SCREEN_TILES_X..(y + 1) * SCREEN_TILES_X]
    }

    pub fn fill(&mut self, x: usize, y: usize, width: usize, height: usize, tile: u8) {
        for row in y..y + height {
            for column in x..x + width {
                self.set(column, row, tile);
            }
        }
    }

    /// `TextBoxBorder`: a box at `(x, y)` around `width` × `height` blank tiles.
    pub fn text_box_border(&mut self, x: usize, y: usize, width: usize, height: usize) {
        let [top_left, horizontal, top_right, vertical, bottom_left, bottom_right] =
            [0x79, 0x7A, 0x7B, 0x7C, 0x7D, 0x7E];
        self.set(x, y, top_left);
        self.fill(x + 1, y, width, 1, horizontal);
        self.set(x + width + 1, y, top_right);
        for row in y + 1..=y + height {
            self.set(x, row, vertical);
            self.fill(x + 1, row, width, 1, Self::BLANK);
            self.set(x + width + 1, row, vertical);
        }
        self.set(x, y + height + 1, bottom_left);
        self.fill(x + 1, y + height + 1, width, 1, horizontal);
        self.set(x + width + 1, y + height + 1, bottom_right);
    }

    fn index(x: usize, y: usize) -> usize {
        assert!(x < SCREEN_TILES_X && y < SCREEN_TILES_Y, "({x}, {y}) is off the screen");
        y * SCREEN_TILES_X + x
    }
}
