//! The current map as a picture, for a model that can see one.

use image::RgbaImage;

use poke_core::geometry::Point8;
use crate::pokemon::map::Map;
use crate::pokemon::map_gfx::{self, NpcSprite, SPRITE_PX, TILE_PX};
use crate::pokemon::map_metadata::{MapMetadata, PlayerFacingDirection};
use crate::pokemon::sprite::{PictureId, SpriteFacing};
use crate::pokemon::tile::{JumpDirection, MetaTile};
use crate::pokemon::tile_map::MetaTileMap;

/// One meta-tile, in pixels: two graphical tiles each way.
pub const CELL_PX: usize = TILE_PX * 2;
/// Room for a three-digit coordinate down the left edge.
pub const RULER_LEFT: usize = 3 * TILE_PX;
/// Room for one row of coordinates along the top.
pub const RULER_TOP: usize = TILE_PX + 2;
/// A coordinate is printed every this many meta-tiles.
const RULER_EVERY: usize = 4;

// ── Palette ──────────────────────────────────────────────────────────────────────────────────────

/// The emulator's own four shades (`DMGColor::to_rgb`), so the map matches the screenshot.
const SHADE: [[u8; 4]; 4] = [
    [0xFF, 0xFF, 0xFF, 0xFF],
    [0xAA, 0xAA, 0xAA, 0xFF],
    [0x55, 0x55, 0x55, 0xFF],
    [0x00, 0x00, 0x00, 0xFF],
];

/// `(r, g, b, alpha)` washed over a meta-tile to say what it means.
type Tint = ([u8; 3], u8);
const GRASS: Tint = ([0x3C, 0xB0, 0x4A], 72);
const WATER: Tint = ([0x2E, 0x74, 0xE0], 82);
const LEDGE: Tint = ([0xF0, 0x9E, 0x14], 88);
const WARP: Tint = ([0xD0, 0x46, 0xE8], 56);
const CONNECTION: Tint = ([0x18, 0xC8, 0xC0], 64);
const CUT_TREE: Tint = ([0x7A, 0xD0, 0x2E], 80);
const COUNTER: Tint = ([0xC8, 0xA0, 0x60], 60);
const PC: Tint = ([0x60, 0xC8, 0xF0], 84);
/// Ground the player cannot reach from where they are standing.
const UNREACHABLE: Tint = ([0x08, 0x0C, 0x14], 120);
/// An unlit cave.
const DARK: Tint = ([0x04, 0x08, 0x18], 96);

const PLAYER_INK: [u8; 4] = [0xFF, 0x28, 0x28, 0xFF];
const LABEL_PLATE: [u8; 4] = [0x0A, 0x0C, 0x12, 0xE6];
const LABEL_INK: [u8; 4] = [0xF4, 0xF7, 0xFB, 0xFF];
/// A label naming somewhere the player cannot get to from where they are standing.
const LABEL_PLATE_OUT_OF_REACH: [u8; 4] = [0x0A, 0x0C, 0x12, 0xB4];
const LABEL_INK_OUT_OF_REACH: [u8; 4] = [0x7C, 0x84, 0x94, 0xFF];
/// The last line of an out-of-reach label.
const NO_ROUTE: &str = "no route";
const RULER_INK: [u8; 4] = [0x9A, 0xA4, 0xB4, 0xFF];
const GUTTER: [u8; 4] = [0x12, 0x14, 0x1A, 0xFF];
const GRID_LINE: Tint = ([0x00, 0x00, 0x00], 28);

/// `Empty` and `Obstacle` are untinted: the art tells them apart, and an unwashed majority is
/// what makes the washed squares read.
fn tint_for(tile: MetaTile) -> Option<Tint> {
    Some(match tile {
        MetaTile::Grass => GRASS,
        MetaTile::Water => WATER,
        MetaTile::ConnectionWater(_) => WATER,
        MetaTile::Jump(_) => LEDGE,
        MetaTile::Warp { .. } => WARP,
        MetaTile::Connection { .. } => CONNECTION,
        MetaTile::CutTree => CUT_TREE,
        MetaTile::Counter => COUNTER,
        // Never in `meta_tiles`: a PC and a hidden object are looked up per map.
        MetaTile::Pc | MetaTile::Switch { .. } => PC,
        // Nor these: both are actions on ordinary floor beside a tree or boulder drawn as art.
        MetaTile::Cut { .. } | MetaTile::BoulderGoal { .. } => return None,
        // Nor `Fish` or `Pace`: actions on floor or water that is already drawn as itself.
        MetaTile::Fish { .. } | MetaTile::Pace { .. } => return None,
        MetaTile::Empty | MetaTile::Obstacle | MetaTile::Sprite(_) => return None,
    })
}

// ── Rendering ────────────────────────────────────────────────────────────────────────────────────

/// The whole of `map` as an RGBA image at one pixel per game pixel.
pub fn render(map: &MetaTileMap) -> Option<RgbaImage> {
    let metadata = map.metadata.as_deref()?;
    let dimensions = metadata.dimensions();
    let width = (RULER_LEFT + map.width * CELL_PX) as u32;
    let height = (RULER_TOP + map.height * CELL_PX) as u32;
    let mut canvas = RgbaImage::from_pixel(width, height, image::Rgba(GUTTER));

    draw_terrain(&mut canvas, metadata, &dimensions);
    draw_tints(&mut canvas, map);
    draw_grid(&mut canvas, map);
    draw_people(&mut canvas, map);
    draw_unreachable(&mut canvas, map);
    draw_ruler(&mut canvas, map);
    draw_labels(&mut canvas, map);
    Some(canvas)
}

/// Wash an unlit map; apart from [`render`] because darkness is a `GameState` fact.
pub fn darken(canvas: &mut RgbaImage) {
    let (width, height) = canvas.dimensions();
    for y in 0..height {
        for x in 0..width {
            blend(canvas, x, y, DARK);
        }
    }
}

/// The base layer: the cartridge's own tiles, for the map proper and for each connection strip.
fn draw_terrain(
    canvas: &mut RgbaImage,
    metadata: &MapMetadata,
    dimensions: &crate::pokemon::map_metadata::MapDimensions,
) {
    let tileset = metadata.map_header.tileset;
    for my in 0..dimensions.meta_height {
        for mx in 0..dimensions.meta_width {
            // The four graphical tiles of one meta-tile, in reading order.
            for (sub_x, sub_y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let tile_id = metadata.tile_id(mx * 2 + sub_x, my * 2 + sub_y);
                blit_tile(
                    canvas,
                    &map_gfx::tileset_tile(tileset, tile_id),
                    cell_x(mx + dimensions.west_extra) + sub_x * TILE_PX,
                    cell_y(my + dimensions.north_extra) + sub_y * TILE_PX,
                );
            }
        }
    }

    // Each strip has its own tileset; `strip_cells` and `tile_ids_at` are shared with the
    // classification pass so the two cannot place or read a strip differently.
    for (strip, strip_idx, mx, my) in metadata.strip_cells() {
        let Some(tile_ids) = strip.tile_ids_at(strip_idx) else { continue };
        for (quadrant, tile_id) in tile_ids.into_iter().enumerate() {
            let Some(tile_id) = tile_id else { continue };
            let (sub_x, sub_y) = (quadrant % 2, quadrant / 2);
            blit_tile(
                canvas,
                &map_gfx::tileset_tile(strip.tileset, tile_id),
                cell_x(mx) + sub_x * TILE_PX,
                cell_y(my) + sub_y * TILE_PX,
            );
        }
    }
}

fn draw_tints(canvas: &mut RgbaImage, map: &MetaTileMap) {
    for (index, &tile) in map.meta_tiles.iter().enumerate() {
        let (mx, my) = (index % map.width, index / map.width);
        if let Some(tint) = tint_for(tile) {
            fill_cell(canvas, mx, my, tint);
        }
        match tile {
            MetaTile::Jump(direction) => draw_ledge_arrow(canvas, mx, my, direction),
            MetaTile::Warp { .. } => outline_cell(canvas, mx, my, WARP.0),
            _ => {}
        }
    }
}

/// A faint rule every four meta-tiles, to carry the ruler's numbers into a wide map.
fn draw_grid(canvas: &mut RgbaImage, map: &MetaTileMap) {
    let (width, height) = canvas.dimensions();
    for mx in (0..=map.width).step_by(RULER_EVERY) {
        let x = cell_x(mx) as u32;
        if x < width {
            for y in RULER_TOP as u32..height {
                blend(canvas, x, y, GRID_LINE);
            }
        }
    }
    for my in (0..=map.height).step_by(RULER_EVERY) {
        let y = cell_y(my) as u32;
        if y < height {
            for x in RULER_LEFT as u32..width {
                blend(canvas, x, y, GRID_LINE);
            }
        }
    }
}

fn draw_people(canvas: &mut RgbaImage, map: &MetaTileMap) {
    // Sorted, so two renders of one state are byte-identical whatever order `sprites` is in.
    let mut people: Vec<_> = map.sprites.iter().filter(|s| !s.hidden).collect();
    people.sort_by_key(|s| (s.position.y, s.position.x, s.index));
    for sprite in people {
        if let Some(art) = map_gfx::npc_sprite(sprite.picture_id, sprite.facing) {
            blit_sprite(canvas, &art, sprite.position);
        }
    }

    // The player last, so nobody standing on the same square hides them.
    let facing = match map.player_direction {
        PlayerFacingDirection::Down => SpriteFacing::Down,
        PlayerFacingDirection::Up => SpriteFacing::Up,
        PlayerFacingDirection::Left => SpriteFacing::Left,
        PlayerFacingDirection::Right => SpriteFacing::Right,
    };
    if let Some(art) = map_gfx::npc_sprite(PictureId::Red, facing) {
        blit_sprite(canvas, &art, map.player_position);
    }
    outline_cell(canvas, map.player_position.x as usize, map.player_position.y as usize, [PLAYER_INK[0], PLAYER_INK[1], PLAYER_INK[2]]);
    draw_facing_pip(canvas, map.player_position, facing);
}

/// Dim every square the player can neither stand on nor act on. `reachable_tiles` is routable-to,
/// so walls it ends routes at are subtracted here.
fn draw_unreachable(canvas: &mut RgbaImage, map: &MetaTileMap) {
    let routable = map.reachable_tiles();
    for my in 0..map.height {
        for mx in 0..map.width {
            let tile = map.meta_tiles[mx + my * map.width];
            let deep_water = matches!(tile, MetaTile::Water | MetaTile::ConnectionWater(_))
                && !map.can_surf;
            let usable = routable.contains(&Point8 { x: mx as u8, y: my as u8 })
                && !matches!(tile, MetaTile::Obstacle)
                && !deep_water;
            if !usable {
                fill_cell(canvas, mx, my, UNREACHABLE);
            }
        }
    }
}

fn draw_ruler(canvas: &mut RgbaImage, map: &MetaTileMap) {
    for mx in (0..map.width).step_by(RULER_EVERY) {
        draw_text(canvas, &mx.to_string(), cell_x(mx) + 1, 1, RULER_INK, None);
    }
    for my in (0..map.height).step_by(RULER_EVERY) {
        draw_text(canvas, &my.to_string(), 1, cell_y(my) + (CELL_PX - TILE_PX) / 2, RULER_INK, None);
    }
}

// ── Labels ───────────────────────────────────────────────────────────────────────────────────────

/// A destination named on the picture: which cells it covers and what it is called.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Label {
    /// Bounding box of the cells this names, in meta-tiles: `(x0, y0, x1, y1)` inclusive.
    cells: (usize, usize, usize, usize),
    text: Vec<String>,
    /// Whether any cell this label names can be routed to from where the player is standing.
    reachable: bool,
}

/// A placed label, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

impl Placed {
    fn overlaps(&self, other: &Placed) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

/// Group the warps and connections into one label each and work out where each box goes.
fn layout_labels(map: &MetaTileMap, canvas: (usize, usize)) -> Vec<(Label, Placed)> {
    let mut placed: Vec<(Label, Placed)> = Vec::new();
    // The player's square is reserved before any label is placed.
    let player = Placed {
        x: cell_x(map.player_position.x as usize) as i64 - 1,
        y: cell_y(map.player_position.y as usize) as i64 - 1,
        w: CELL_PX as i64 + 2,
        h: CELL_PX as i64 + 2,
    };
    let mut blocked = vec![player];
    for label in collect_labels(map) {
        let (w, h) = label_size(&label);
        let (x0, y0, x1, y1) = label.cells;
        let centre = cell_x(x0) as i64 + ((x1 - x0 + 1) * CELL_PX) as i64 / 2 - w / 2;
        let middle = cell_y(y0) as i64 + ((y1 - y0 + 1) * CELL_PX) as i64 / 2 - h / 2;
        let anchors = [
            (centre, cell_y(y0) as i64 - h - 1),                    // above
            (centre, cell_y(y1 + 1) as i64 + 1),                    // below
            (cell_x(x1 + 1) as i64 + 1, middle),                    // right
            (cell_x(x0) as i64 - w - 1, middle),                    // left
        ];
        let mut chosen = None;
        'anchor: for (ax, ay) in anchors {
            for nudge in 0..8 {
                for step in [nudge as i64 * TILE_PX as i64, -(nudge as i64) * TILE_PX as i64] {
                    // Nudge along whichever axis the anchor is free to slide on.
                    let candidate = match ay < cell_y(y0) as i64 || ay > cell_y(y1) as i64 {
                        true => Placed { x: ax + step, y: ay, w, h },
                        false => Placed { x: ax, y: ay + step, w, h },
                    };
                    if candidate.x < 0
                        || candidate.y < RULER_TOP as i64
                        || candidate.x + w > canvas.0 as i64
                        || candidate.y + h > canvas.1 as i64
                    {
                        continue;
                    }
                    if blocked.iter().any(|p| p.overlaps(&candidate)) {
                        continue;
                    }
                    chosen = Some(candidate);
                    break 'anchor;
                }
            }
        }
        if let Some(placed_box) = chosen {
            blocked.push(placed_box);
            placed.push((label, placed_box));
        }
    }
    placed
}

/// One label per destination, grouped so the picture says each name once.
fn collect_labels(map: &MetaTileMap) -> Vec<Label> {
    // Per cell, not per bounding box: one building can have doors on two terraces.
    let routable = map.reachable_tiles();
    let mut groups: Vec<(Map, bool, usize, usize, usize, usize, bool)> = Vec::new();
    for (index, &tile) in map.meta_tiles.iter().enumerate() {
        let (to_map, is_edge) = match tile {
            MetaTile::Warp { to_map, .. } => (to_map, false),
            MetaTile::Connection { to_map, .. } => (to_map, true),
            MetaTile::ConnectionWater(to_map) => (to_map, true),
            _ => continue,
        };
        let (x, y) = (index % map.width, index / map.width);
        // A water crossing needs Surf, as it does in `MetaTileMap::actions()`.
        let here = routable.contains(&Point8 { x: x as u8, y: y as u8 })
            && (map.can_surf || !matches!(tile, MetaTile::ConnectionWater(_)));
        match groups.iter_mut().find(|(m, edge, x0, y0, x1, y1, _)| {
            *m == to_map
                && *edge == is_edge
                // A door is one or two cells, so a warp joins only a box it touches.
                && (is_edge || (x + 1 >= *x0 && x <= *x1 + 1 && y + 1 >= *y0 && y <= *y1 + 1))
        }) {
            Some((_, _, x0, y0, x1, y1, reachable)) => {
                *x0 = (*x0).min(x);
                *y0 = (*y0).min(y);
                *x1 = (*x1).max(x);
                *y1 = (*y1).max(y);
                *reachable |= here;
            }
            None => groups.push((to_map, is_edge, x, y, x, y, here)),
        }
    }
    // A warp's label carries its coordinate, the only key to its menu row when three ladders
    // share a name. A map edge is one row, named by where it leads.
    groups.into_iter()
        .map(|(to_map, is_edge, x0, y0, x1, y1, reachable)| {
            let mut text = wrap(&format!("{to_map}"));
            if !is_edge {
                text.push(format!("({x0},{y0})"));
            }
            if !reachable {
                text.push(NO_ROUTE.to_string());
            }
            Label { cells: (x0, y0, x1, y1), text, reachable }
        })
        .collect()
}

/// `ViridianForestSouthGate` → `["Viridian", "Forest", "South Gate"]`.
fn wrap(name: &str) -> Vec<String> {
    const MAX: usize = 12;
    let mut words: Vec<String> = Vec::new();
    for c in name.chars() {
        match c.is_uppercase() && !words.last().is_none_or(|w: &String| w.is_empty()) {
            true => words.push(c.to_string()),
            false => match words.last_mut() {
                Some(word) => word.push(c),
                None => words.push(c.to_string()),
            },
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for word in words {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= MAX => {
                line.push(' ');
                line.push_str(&word);
            }
            _ => lines.push(word),
        }
    }
    lines
}

fn label_size(label: &Label) -> (i64, i64) {
    let widest = label.text.iter().map(|l| map_gfx::text_width(l)).max().unwrap_or(0);
    ((widest + 4) as i64, (label.text.len() * TILE_PX + 4) as i64)
}

fn draw_labels(canvas: &mut RgbaImage, map: &MetaTileMap) {
    let size = (canvas.width() as usize, canvas.height() as usize);
    for (label, placed) in layout_labels(map, size) {
        let (plate, ink) = match label.reachable {
            true => (LABEL_PLATE, LABEL_INK),
            false => (LABEL_PLATE_OUT_OF_REACH, LABEL_INK_OUT_OF_REACH),
        };
        for y in placed.y..placed.y + placed.h {
            for x in placed.x..placed.x + placed.w {
                set(canvas, x, y, plate);
            }
        }
        for (row, line) in label.text.iter().enumerate() {
            draw_text(canvas, line, placed.x as usize + 2, placed.y as usize + 2 + row * TILE_PX,
                      ink, None);
        }
    }
}

// ── Drawing primitives ───────────────────────────────────────────────────────────────────────────

fn cell_x(mx: usize) -> usize { RULER_LEFT + mx * CELL_PX }
fn cell_y(my: usize) -> usize { RULER_TOP + my * CELL_PX }

fn set(canvas: &mut RgbaImage, x: i64, y: i64, colour: [u8; 4]) {
    if x >= 0 && y >= 0 && (x as u32) < canvas.width() && (y as u32) < canvas.height() {
        canvas.put_pixel(x as u32, y as u32, image::Rgba(colour));
    }
}

fn blend(canvas: &mut RgbaImage, x: u32, y: u32, (rgb, alpha): Tint) {
    if x >= canvas.width() || y >= canvas.height() {
        return;
    }
    let under = canvas.get_pixel(x, y).0;
    let mix = |a: u8, b: u8| ((a as u16 * (255 - alpha) as u16 + b as u16 * alpha as u16) / 255) as u8;
    canvas.put_pixel(x, y, image::Rgba([mix(under[0], rgb[0]), mix(under[1], rgb[1]), mix(under[2], rgb[2]), 0xFF]));
}

fn blit_tile(canvas: &mut RgbaImage, pixels: &[u8; 64], left: usize, top: usize) {
    for y in 0..TILE_PX {
        for x in 0..TILE_PX {
            set(canvas, (left + x) as i64, (top + y) as i64, SHADE[pixels[y * TILE_PX + x] as usize]);
        }
    }
}

/// Shade `0` is transparent for an overworld sprite — painting it draws every person in a box.
fn blit_sprite(canvas: &mut RgbaImage, sprite: &NpcSprite, at: Point8) {
    let (left, top) = (cell_x(at.x as usize), cell_y(at.y as usize));
    for y in 0..SPRITE_PX {
        for x in 0..SPRITE_PX {
            let shade = sprite.shades[y * SPRITE_PX + x];
            if shade != 0 {
                set(canvas, (left + x) as i64, (top + y) as i64, SHADE[shade as usize]);
            }
        }
    }
}

fn fill_cell(canvas: &mut RgbaImage, mx: usize, my: usize, tint: Tint) {
    for y in 0..CELL_PX {
        for x in 0..CELL_PX {
            blend(canvas, (cell_x(mx) + x) as u32, (cell_y(my) + y) as u32, tint);
        }
    }
}

fn outline_cell(canvas: &mut RgbaImage, mx: usize, my: usize, rgb: [u8; 3]) {
    let (left, top) = (cell_x(mx) as i64, cell_y(my) as i64);
    let colour = [rgb[0], rgb[1], rgb[2], 0xFF];
    for i in 0..CELL_PX as i64 {
        set(canvas, left + i, top, colour);
        set(canvas, left + i, top + CELL_PX as i64 - 1, colour);
        set(canvas, left, top + i, colour);
        set(canvas, left + CELL_PX as i64 - 1, top + i, colour);
    }
}

/// A pip on the side of the player's cell they face, which decides what an `A` press talks to.
fn draw_facing_pip(canvas: &mut RgbaImage, at: Point8, facing: SpriteFacing) {
    let (left, top) = (cell_x(at.x as usize) as i64, cell_y(at.y as usize) as i64);
    let mid = CELL_PX as i64 / 2;
    let last = CELL_PX as i64 - 1;
    // Offsets from the cell's top-left corner; `-1` and `last + 1` are the pixel just outside it.
    for step in -1..2i64 {
        let (dx, dy) = match facing {
            SpriteFacing::Up => (mid + step, -1),
            SpriteFacing::Down => (mid + step, last + 1),
            SpriteFacing::Left => (-1, mid + step),
            SpriteFacing::Right => (last + 1, mid + step),
        };
        set(canvas, left + dx, top + dy, PLAYER_INK);
    }
}

fn draw_ledge_arrow(canvas: &mut RgbaImage, mx: usize, my: usize, direction: JumpDirection) {
    let (left, top) = (cell_x(mx) as i64, cell_y(my) as i64);
    let ink = [LEDGE.0[0], LEDGE.0[1], LEDGE.0[2], 0xFF];
    // A four-row chevron pointing the only way this ledge can be jumped.
    for row in 0..4i64 {
        for span in -row..=row {
            let (x, y) = match direction {
                JumpDirection::South => (7 + span, 6 + row),
                JumpDirection::West => (9 - row, 7 + span),
                JumpDirection::East => (6 + row, 7 + span),
            };
            set(canvas, left + x, top + y, ink);
        }
    }
}

/// `text` in the cartridge's own font.
fn draw_text(canvas: &mut RgbaImage, text: &str, left: usize, top: usize, ink: [u8; 4], plate: Option<[u8; 4]>) {
    for (column, glyph) in map_gfx::glyphs(text).into_iter().enumerate() {
        let (gx, gy) = (left + column * TILE_PX, top);
        if let Some(plate) = plate {
            for y in 0..TILE_PX {
                for x in 0..TILE_PX {
                    set(canvas, (gx + x) as i64, (gy + y) as i64, plate);
                }
            }
        }
        let Some(glyph) = glyph else { continue };
        let mask = map_gfx::glyph_mask(glyph);
        for y in 0..TILE_PX {
            for x in 0..TILE_PX {
                if mask[y * TILE_PX + x] {
                    set(canvas, (gx + x) as i64, (gy + y) as i64, ink);
                }
            }
        }
    }
}

// ── Delivery ─────────────────────────────────────────────────────────────────────────────────────

/// PNG bytes.
pub fn encode(canvas: &RgbaImage) -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    canvas.write_to(&mut png, image::ImageFormat::Png).expect("an in-memory image encodes to PNG");
    png.into_inner()
}

/// What the picture is, and the pixel formula that turns a ruler reading into a menu id.
pub fn caption(map: &MetaTileMap, is_dark: bool) -> String {
    let dark = match is_dark {
        true => " This map is unlit — the game's own screen shows almost nothing here, so this is \
                 what is known about the map rather than what is on screen. Flash lights it.",
        false => "",
    };
    format!(
        "A map of {} ({}x{} squares), drawn from the game's own graphics. You are the square ringed \
         in red, with a pip on the side you are facing. Each square is 16x16 pixels: the square at \
         (x, y) is drawn at pixel ({} + 16x, {} + 16y), and the numbers along the top and left edges \
         are those coordinates. Green is tall grass, blue is water, magenta is a warp labelled with \
         where it leads and its own (x,y), orange is a ledge with an arrow for the only way it can \
         be jumped, and anything you cannot walk to from where you are standing is dimmed. A label \
         greyed out and marked `{NO_ROUTE}` names somewhere there is no way to from where you are \
         standing, however walkable the ground between you and it looks: a one-way ledge or a \
         terrace is still a wall.{dark}",
        map.map, map.width, map.height, RULER_LEFT, RULER_TOP,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::integration_tests::fixture::TestFixture;
    use std::time::Duration;

    /// Strips on two edges, a dense city, a cave, a `Plateau` strip tileset unlike its map's, and
    /// blocks read from `wOverworldMap` at runtime.
    fn fixtures() -> Vec<(&'static str, &'static [u8])> {
        vec![
            ("pallet-town", &include_bytes!("../pokemon/data/pallet-town-state.bin")[..]),
            ("celadon", &include_bytes!("../pokemon/data/at-celadon.bin")[..]),
            ("mt-moon", &include_bytes!("../pokemon/data/mt-moon.bin")[..]),
            ("indigo", &include_bytes!("../pokemon/data/at-indigo.bin")[..]),
            ("vermilion", &include_bytes!("../pokemon/data/at-vermilion.bin")[..]),
        ]
    }

    fn render_fixture(snapshot: &[u8]) -> (crate::pokemon::GameState, RgbaImage) {
        let mut fixture = TestFixture::new(snapshot, Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let canvas = render(&state.map).expect("a fixture is a real map");
        (state, canvas)
    }

    #[test]
    fn a_map_renders_at_one_pixel_per_game_pixel() {
        for (name, snapshot) in fixtures() {
            let (state, canvas) = render_fixture(snapshot);
            assert_eq!(
                canvas.dimensions(),
                ((RULER_LEFT + state.map.width * CELL_PX) as u32,
                 (RULER_TOP + state.map.height * CELL_PX) as u32),
                "{name}");
        }
    }

    /// The two silent failures: drawing nothing, and one flat colour from a lookup off the sheet.
    #[test]
    fn the_render_is_neither_blank_nor_uniform() {
        for (name, snapshot) in fixtures() {
            let (_, canvas) = render_fixture(snapshot);
            let mut seen = std::collections::HashMap::new();
            for pixel in canvas.pixels() {
                *seen.entry(pixel.0).or_insert(0usize) += 1;
            }
            let total = canvas.pixels().count();
            assert!(seen.len() >= 16, "{name}: only {} distinct colours", seen.len());
            let modal = *seen.values().max().expect("non-empty");
            assert!(modal * 100 < total * 90, "{name}: {}% of the image is one colour",
                    modal * 100 / total);
        }
    }

    #[test]
    fn the_player_ring_is_where_the_json_says_it_is() {
        for (name, snapshot) in fixtures() {
            let (state, canvas) = render_fixture(snapshot);
            let at = state.map.player_position;
            let (left, top) = (cell_x(at.x as usize) as i64, cell_y(at.y as usize) as i64);
            let ring = image::Rgba(PLAYER_INK);

            let mut inside = 0;
            for (x, y, pixel) in canvas.enumerate_pixels() {
                if *pixel != ring {
                    continue;
                }
                let (dx, dy) = (x as i64 - left, y as i64 - top);
                let near = (-2..CELL_PX as i64 + 2).contains(&dx) && (-2..CELL_PX as i64 + 2).contains(&dy);
                assert!(near, "{name}: player ink at ({x}, {y}), {CELL_PX}px cell starts at ({left}, {top})");
                inside += 1;
            }
            assert!(inside >= CELL_PX * 3, "{name}: only {inside} ring pixels — is the ring drawn?");
        }
    }

    #[test]
    fn two_renders_of_one_state_are_identical() {
        for (name, snapshot) in fixtures() {
            let mut fixture = TestFixture::new(snapshot, Duration::from_secs(10), vec![]);
            let state = fixture.game_state();
            let first = render(&state.map).expect("a real map");
            for attempt in 0..4 {
                assert!(render(&state.map).expect("a real map").as_raw() == first.as_raw(),
                        "{name}: render {attempt} differs");
            }
        }
    }

    #[test]
    fn labels_stay_inside_the_canvas_and_off_each_other() {
        for (name, snapshot) in fixtures() {
            let mut fixture = TestFixture::new(snapshot, Duration::from_secs(10), vec![]);
            let state = fixture.game_state();
            let map = &state.map;
            let size = (RULER_LEFT + map.width * CELL_PX, RULER_TOP + map.height * CELL_PX);
            let placed = layout_labels(map, size);
            for (index, (label, box_)) in placed.iter().enumerate() {
                assert!(box_.x >= 0 && box_.y >= 0
                            && box_.x + box_.w <= size.0 as i64
                            && box_.y + box_.h <= size.1 as i64,
                        "{name}: {:?} at {box_:?} leaves a {size:?} canvas", label.text);
                for (other, other_box) in &placed[index + 1..] {
                    assert!(!box_.overlaps(other_box),
                            "{name}: {:?} overlaps {:?}", label.text, other.text);
                }
            }
        }
    }

    #[test]
    fn a_label_for_somewhere_out_of_reach_is_greyed_and_says_so() {
        let mut fixture = TestFixture::new(
            &include_bytes!("../pokemon/data/split-cerulean.bin")[..], Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let labels = collect_labels(&state.map);
        let at = |x: usize, y: usize| labels.iter().find(|l| l.cells.0 == x && l.cells.1 == y)
            .unwrap_or_else(|| panic!("no label at ({x}, {y}): {labels:?}"));

        // The trashed house, both ways in.
        assert!(!at(28, 10).reachable, "the front door is on the terrace above: {:?}", at(28, 10));
        assert!(at(28, 12).reachable, "the back door is the one that works: {:?}", at(28, 12));
        // In words as well as ink: a dimmed plate is easy to miss.
        assert!(at(28, 10).text.contains(&NO_ROUTE.to_string()), "{:?}", at(28, 10));
        assert!(!at(28, 12).text.contains(&NO_ROUTE.to_string()), "{:?}", at(28, 12));

        // A map edge groups differently: Route 4 is off this terrace, Route 5 two terraces down.
        assert!(labels.iter().any(|l| l.text[0] == "Route4" && l.reachable), "{labels:?}");
        assert!(labels.iter().any(|l| l.text[0] == "Route5" && !l.reachable), "{labels:?}");

        // The ink, not just the flag.
        let size = (RULER_LEFT + state.map.width * CELL_PX, RULER_TOP + state.map.height * CELL_PX);
        let canvas = render(&state.map).expect("a fixture is a real map");
        let mut checked = 0;
        for (label, placed) in layout_labels(&state.map, size) {
            if label.reachable { continue }
            checked += 1;
            for y in placed.y..placed.y + placed.h {
                for x in placed.x..placed.x + placed.w {
                    assert_ne!(canvas.get_pixel(x as u32, y as u32).0, LABEL_INK,
                               "{:?} is drawn in the reachable ink at ({x}, {y})", label.text);
                }
            }
        }
        assert!(checked > 0, "no out-of-reach label was placed, so the ink is untested");
    }

    #[test]
    fn a_warp_label_carries_its_coordinate_and_an_edge_does_not() {
        let mut fixture = TestFixture::new(&include_bytes!("../pokemon/data/mt-moon.bin")[..], Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let labels = collect_labels(&state.map);
        let ladders: Vec<&Label> = labels.iter().filter(|l| l.text.iter().any(|t| t.contains("Mt Moon"))).collect();
        assert_eq!(ladders.len(), 3, "Mt Moon 1F has three ladders down: {labels:?}");
        for label in &ladders {
            let (x0, y0, ..) = label.cells;
            assert_eq!(label.text.last().map(String::as_str), Some(format!("({x0},{y0})").as_str()), "{label:?}");
        }

        let mut fixture = TestFixture::new(&include_bytes!("../pokemon/data/pallet-town-state.bin")[..], Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let edge = collect_labels(&state.map).into_iter().find(|l| l.text.iter().any(|t| t.contains("Route1"))).expect("the way north");
        assert!(!edge.text.iter().any(|t| t.starts_with('(')), "{edge:?}");
    }

    #[test]
    fn the_tiles_drawn_are_the_tiles_the_game_laid_out() {
        use crate::pokemon::symbols::DmgPointerRead;
        const SCREEN: (usize, usize) = (20, 18);

        for (name, snapshot) in fixtures() {
            let mut fixture = TestFixture::new(snapshot, Duration::from_secs(10), vec![]);
            let state = fixture.game_state();
            let metadata = state.map.metadata.as_deref().expect("a fixture is a real map");
            let on_screen = fixture.api().mmu().read_pointer_vec(
                &crate::pokemon::symbols::pokered_symbols::wTileMap, SCREEN.0 * SCREEN.1);

            let (tiles_wide, tiles_high) = (
                metadata.map_header.width as usize * 4,
                metadata.map_header.height as usize * 4,
            );
            // The view can hang off the map's edge, where this renderer draws nothing.
            let mut matches = Vec::new();
            for oy in -(SCREEN.1 as i64)..tiles_high as i64 {
                for ox in -(SCREEN.0 as i64)..tiles_wide as i64 {
                    let mut compared = 0;
                    let agrees = (0..SCREEN.1).all(|y| (0..SCREEN.0).all(|x| {
                        let (tx, ty) = (ox + x as i64, oy + y as i64);
                        if tx < 0 || ty < 0 || tx >= tiles_wide as i64 || ty >= tiles_high as i64 {
                            return true;
                        }
                        compared += 1;
                        metadata.tile_id(tx as usize, ty as usize) == on_screen[y * SCREEN.0 + x]
                    }));
                    if agrees && compared >= 200 {
                        matches.push((ox, oy));
                    }
                }
            }
            assert!(!matches.is_empty(),
                    "{name}: no window of the rendered map reproduces the {}x{} the game has on \
                     screen — the block, blockset or within-block tile order is wrong",
                    SCREEN.0, SCREEN.1);

            // Centred on the player, so a repetitive map cannot pass at an arbitrary offset.
            let dimensions = metadata.dimensions();
            let player = (
                (state.map.player_position.x as i64 - dimensions.west_extra as i64) * 2,
                (state.map.player_position.y as i64 - dimensions.north_extra as i64) * 2,
            );
            assert!(matches.iter().any(|&(ox, oy)| {
                (player.0 - ox - SCREEN.0 as i64 / 2).abs() <= 2
                    && (player.1 - oy - SCREEN.1 as i64 / 2).abs() <= 2
            }), "{name}: the screen matches at {matches:?}, none of them centred on the player at \
                 {player:?}");
        }
    }

    /// [`MetaTileMap::reachable_tiles`] is routable-to, not standable-on.
    #[test]
    fn a_wall_is_dimmed_even_though_the_agent_can_route_to_it() {
        let mut fixture = TestFixture::new(
            &include_bytes!("../pokemon/data/pallet-town-state.bin")[..],
            Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let map = &state.map;
        let routable = map.reachable_tiles();

        let wall_beside_a_street = (0..map.width * map.height)
            .map(|i| (i % map.width, i / map.width))
            .find(|&(x, y)| {
                matches!(map.meta_tiles[x + y * map.width], MetaTile::Obstacle)
                    && routable.contains(&Point8 { x: x as u8, y: y as u8 })
            })
            .expect("a town has a wall you can walk up to");

        let canvas = render(map).expect("a real map");
        let lit = |(x, y): (usize, usize)| {
            let (px, py) = (cell_x(x) as u32 + 8, cell_y(y) as u32 + 8);
            canvas.get_pixel(px, py).0
        };
        // The dim is an alpha wash, so "dimmed" is "much darker than the same art undimmed".
        let wall = lit(wall_beside_a_street);
        let sum = |c: [u8; 4]| c[0] as u32 + c[1] as u32 + c[2] as u32;
        assert!(sum(wall) < 3 * 0x60,
                "a wall at {wall_beside_a_street:?} is routable-to, and must still read as \
                 out of reach — it rendered {wall:?}");

        // The door in the same town does not dim.
        let door = (0..map.width * map.height)
            .map(|i| (i % map.width, i / map.width))
            .find(|&(x, y)| matches!(map.meta_tiles[x + y * map.width], MetaTile::Warp { .. })
                            && routable.contains(&Point8 { x: x as u8, y: y as u8 }))
            .expect("Pallet Town has reachable doors");
        assert!(sum(lit(door)) > sum(wall), "the door at {door:?} dimmed like a wall");
    }

    #[test]
    fn map_names_wrap_into_words() {
        assert_eq!(wrap("PalletTown"), vec!["Pallet Town"]);
        assert_eq!(wrap("Route1"), vec!["Route1"]);
        assert_eq!(wrap("ViridianForestSouthGate"), vec!["Viridian", "Forest South", "Gate"]);
        assert_eq!(wrap("OaksLab"), vec!["Oaks Lab"]);
    }

    /// `MetaTileMap` is `Default`, and a default one has no metadata to draw.
    #[test]
    fn a_map_with_no_metadata_declines_to_render() {
        assert!(render(&MetaTileMap::default()).is_none());
    }

    #[test]
    fn every_committed_fixture_renders() {
        let mut rendered = 0;
        for entry in std::fs::read_dir("src/pokemon/data").expect("the fixture directory") {
            let path = entry.expect("a readable entry").path();
            if path.extension().is_none_or(|e| e != "bin") {
                continue;
            }
            let snapshot = std::fs::read(&path).expect("a readable fixture");
            let mut fixture = TestFixture::new(&snapshot, Duration::from_secs(10), vec![]);
            let Ok(state) = fixture.try_game_state() else { continue };
            let Some(canvas) = render(&state.map) else { continue };
            assert!(canvas.pixels().any(|p| *p != image::Rgba(GUTTER)),
                    "{} rendered as bare gutter", path.display());
            rendered += 1;
        }
        assert!(rendered > 20, "only {rendered} fixtures rendered — did the walk find them?");
    }

    /// Writes the renders to `target/map-renders` and prints what each costs the model.
    #[cfg(feature = "slow-tests")]
    #[test]
    #[ignore = "probe: prints what a map picture costs the model"]
    fn probe_map_images() {
        let out = std::path::Path::new("target/map-renders");
        std::fs::create_dir_all(out).expect("a writable target directory");
        for (name, snapshot) in fixtures() {
            let mut fixture = TestFixture::new(snapshot, Duration::from_secs(10), vec![]);
            let state = fixture.game_state();
            let canvas = render(&state.map).expect("a real map");
            let png = encode(&canvas);
            let (w, h) = canvas.dimensions();
            let path = out.join(format!("{name}-{}.png", state.map.map));
            std::fs::write(&path, &png).expect("writable");
            println!("{path:?}  {w}x{h}px  {} KB png  ~{} high-detail tokens  {} labels",
                     png.len() / 1024,
                     crate::llm::protocol::image_tokens(crate::llm::protocol::ImageDetail::High, w, h),
                     layout_labels(&state.map, (w as usize, h as usize)).len());
            println!("{}", state.map);
        }
    }
}
