use serde::{Deserialize, Serialize};
use crate::gfx::colour::Source;
use crate::gfx::layers::Object;
use crate::gfx::tiles::pixel;
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::gfx::Screen;

pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 144;
const OBJECTS_PER_LINE: usize = 10;

/// Shades after the palettes, 0 white to 3 black, a row at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Framebuffer {
    pub shades: Vec<u8>,
    /// Which palette register each shade came through, one to a pixel: what a colour mode needs to
    /// tell a sprite from the background, since the shade itself no longer says.
    pub sources: Vec<Source>,
}

impl Framebuffer {
    pub fn shade(&self, x: usize, y: usize) -> u8 {
        self.shades[y * WIDTH + x]
    }
}

fn apply(palette: u8, colour: u8) -> u8 {
    (palette >> (colour * 2)) & 3
}

impl Screen {
    /// The background's colour index at a screen pixel: the UI where it covers, else the map.
    fn background(&self, x: usize, y: usize) -> u8 {
        if let Some(window) = &self.window
            && y >= window.y as usize
            && x + 7 >= window.x as usize
        {
            let (wx, wy) = (x + 7 - window.x as usize, y - window.y as usize);
            return pixel(self.tiles.bg(window.tiles.get(wx / 8, wy / 8)), wx % 8, wy % 8);
        }
        let scx = self.effects.line_scx.as_ref().map_or(self.effects.scx, |lines| lines[y]);
        let bx = (x + scx as usize) & 0xFF;
        let by = (y + self.effects.scy as usize) & 0xFF;
        if let Some(map) = &self.background {
            return pixel(self.tiles.bg(map.get(bx / 8, by / 8)), bx % 8, by % 8);
        }
        let (column, row) = (bx / 8, by / 8);
        let tile = if column < SCREEN_TILES_X && row < SCREEN_TILES_Y {
            self.ui.cover(column, row)
        } else {
            None
        };
        let (tile, tx, ty) = match tile {
            Some(tile) => (Some(tile), bx % 8, by % 8),
            None => {
                let (mx, my) = (self.map.camera.0 + bx as i32, self.map.camera.1 + by as i32);
                (self.map.tile_at(mx, my), mx.rem_euclid(8) as usize, my.rem_euclid(8) as usize)
            }
        };
        tile.map_or(0, |tile| pixel(self.tiles.bg(tile), tx, ty))
    }

    /// The hardware's rules: ten objects a line in OAM order, the leftmost drawn on top and OAM
    /// order breaking ties, and `BEHIND_BG` losing to any background colour but 0.
    pub fn frame(&self) -> Framebuffer {
        let mut shades = vec![0; WIDTH * HEIGHT];
        let mut sources = vec![Source::Background; WIDTH * HEIGHT];
        for y in 0..HEIGHT {
            let mut line: Vec<(usize, &Object)> = self.sprites.iter().enumerate()
                .filter(|(_, o)| (o.y as usize) <= y + 16 && y + 16 < o.y as usize + 8)
                .take(OBJECTS_PER_LINE)
                .collect();
            line.sort_by_key(|&(index, o)| (o.x, index));
            for x in 0..WIDTH {
                let background = self.background(x, y);
                let mut shade = apply(self.effects.bgp, background);
                let mut source = Source::Background;
                for &(_, object) in &line {
                    let ox = x + 8;
                    if ox < object.x as usize || ox >= object.x as usize + 8 {
                        continue;
                    }
                    let mut px = ox - object.x as usize;
                    let mut py = y + 16 - object.y as usize;
                    if object.attributes & Object::X_FLIP != 0 { px = 7 - px; }
                    if object.attributes & Object::Y_FLIP != 0 { py = 7 - py; }
                    let colour = pixel(self.tiles.obj(object.tile), px, py);
                    if colour == 0 {
                        continue;
                    }
                    if object.attributes & Object::BEHIND_BG == 0 || background == 0 {
                        let (palette, from) = if object.attributes & Object::OBP1 != 0 {
                            (self.effects.obp1, Source::Object1)
                        } else {
                            (self.effects.obp0, Source::Object0)
                        };
                        shade = apply(palette, colour);
                        source = from;
                    }
                    break;
                }
                shades[y * WIDTH + x] = shade;
                sources[y * WIDTH + x] = source;
            }
        }
        Framebuffer { shades, sources }
    }
}
