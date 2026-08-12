//! The current map as a picture, for a model that can see one.
//!
//! `read_map` used to answer with an ASCII grid and a legend. This draws the same map out of the
//! cartridge's own graphics instead — real tiles, every NPC where it is standing and facing where it
//! is facing, a wash of colour saying what each square *means*, and the warps and map edges labelled
//! with where they lead. [`crate::pokemon::map_gfx`] gets the pixels out of the ROM; everything about
//! what colour they end up is here, exactly as `src/web/` owns the palettes for the badges and the
//! Pokédex.
//!
//! # ⚠️ This runs on the worker thread, and that is the point
//!
//! Encoding Celadon is 460k pixels and Route 17 is 737k — tens to hundreds of milliseconds. The
//! emulator thread's whole invariant is that it never stops while the model thinks, and
//! `AGENT_RESOLUTION` is 20 ms, so rendering there would spend ten agent ticks inside one of them on
//! nearly every overworld turn. What crosses the channel is therefore a [`MetaTileMap`] — which the
//! policy is already cloning once per poll — and the drawing happens here, against
//! [`crate::pokemon::roms::POKERED`], which is a `&'static [u8]` and needs no emulator at all.
//! [`crate::llm::screenshot`] is on this thread for the same reason and says so at a thirtieth of
//! the size.
//!
//! # ⚠️ The coordinate contract
//!
//! **The meta-tile at `(x, y)` is drawn at pixel `(RULER_LEFT + 16x, RULER_TOP + 16y)`**, where
//! `(x, y)` is the coordinate the JSON alongside it uses — `position`, `sprites[].position`,
//! `warps[].at` — and the one baked into the action ids the model quotes back
//! (`PalletTown:5,6:Warp`). A ruler runs along the top and left so the model can *read* a coordinate
//! off the picture rather than counting forty tiles. Without that the picture is decoration;
//! [`crate::llm::map_image::tests::the_player_ring_is_where_the_json_says_it_is`] is what holds it.
//!
//! # ⚠️ Nothing here may iterate a `HashSet`
//!
//! `warp_targets`, `connection_targets` and `MetaTileMap::reachable_tiles` are all sets, and a
//! render whose label order depends on hash iteration order is a render that differs between two
//! reads of an unchanged map. To a model that reads as the world having moved; to the committed
//! checksums it is an intermittent failure. Every pass below walks `meta_tiles` in index order, and
//! the label pass sorts by `(y, x)`.

use image::RgbaImage;

use crate::geometry::Point8;
use crate::pokemon::map::Map;
use crate::pokemon::map_gfx::{self, NpcSprite, SPRITE_PX, TILE_PX};
use crate::pokemon::map_metadata::{MapMetadata, PlayerFacingDirection};
use crate::pokemon::sprite::{PictureId, SpriteFacing};
use crate::pokemon::tile::{JumpDirection, MetaTile};
use crate::pokemon::tile_map::MetaTileMap;

/// One meta-tile, in pixels. Two graphical tiles each way.
pub const CELL_PX: usize = TILE_PX * 2;
/// Room for a three-digit coordinate down the left edge.
pub const RULER_LEFT: usize = 3 * TILE_PX;
/// Room for one row of coordinates along the top.
pub const RULER_TOP: usize = TILE_PX + 2;
/// A coordinate is printed every this many meta-tiles.
const RULER_EVERY: usize = 4;

// ── Palette ──────────────────────────────────────────────────────────────────────────────────────

/// The emulator's own four shades (`DMGColor::to_rgb`, `src/lcd_palette.rs`), so the map and the
/// screenshot beside it are visibly the same game.
///
/// ⚠️ **Not inverted.** `src/web/badges.rs` inverts its ramp and `src/web/sprites.rs` must not; the
/// argument is at `sprites.rs`'s `INK`, and it applies here for the same reason — map tiles are
/// *filled* art, so flipping the ramp is a different picture rather than a palette choice.
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
/// An unlit cave. See [`render`] for why the map is drawn in full anyway.
const DARK: Tint = ([0x04, 0x08, 0x18], 96);

const PLAYER_INK: [u8; 4] = [0xFF, 0x28, 0x28, 0xFF];
const LABEL_PLATE: [u8; 4] = [0x0A, 0x0C, 0x12, 0xE6];
const LABEL_INK: [u8; 4] = [0xF4, 0xF7, 0xFB, 0xFF];
const RULER_INK: [u8; 4] = [0x9A, 0xA4, 0xB4, 0xFF];
const GUTTER: [u8; 4] = [0x12, 0x14, 0x1A, 0xFF];
const GRID_LINE: Tint = ([0x00, 0x00, 0x00], 28);

/// `Empty` and `Obstacle` are deliberately untinted: the ROM art already says which is which, and it
/// is the *absence* of a wash on most of the map that makes the washed squares read at a glance.
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
        MetaTile::Pc => PC,
        MetaTile::Empty | MetaTile::Obstacle | MetaTile::Sprite(_) => return None,
    })
}

// ── Rendering ────────────────────────────────────────────────────────────────────────────────────

/// The whole of `map` as an RGBA image at one pixel per game pixel.
///
/// `None` when the map carries no [`MapMetadata`] — a `Default`-constructed grid or one built by
/// hand for a test. There is nothing to draw and the caller falls back to the ASCII grid.
///
/// ⚠️ **An unlit map is drawn in full**, washed with [`DARK`] and flagged in the caption. The ASCII
/// grid never gated tile classification on darkness either (it is ROM-derived, and `is_dark` has
/// always been a separate field), so hiding it here would be a change to what the agent knows
/// smuggled in under a change to how it is drawn.
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

/// Wash the whole canvas for an unlit map. Separate from [`render`] because darkness is a
/// `GameState` fact rather than a `MetaTileMap` one.
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

    // ⚠️ Each strip carries its own tileset — `strip_cells` and `tile_ids_at` are shared with the
    // classification pass precisely so the two cannot place or read a strip differently.
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

/// A faint rule every four meta-tiles, so the ruler's numbers can be carried into the middle of a
/// wide map by eye.
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
    // ⚠️ Index order, not `sprites`' own — two renders of one state must be byte-identical, and
    // `sprites` is built by iteration over slots so it is already stable; sorting keeps it that way
    // if that ever changes.
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

/// Dim every square the player can neither stand on nor act on — the single most useful thing the
/// picture can say that the raw art cannot.
///
/// ⚠️ **`MetaTileMap::reachable_tiles` is not the answer on its own, and using it as one is a bug
/// that renders.** It is the key set of the routing BFS, and that BFS records *every* neighbour of
/// an open square — walls included — because a route has to be able to end at a door, a counter or a
/// cut tree. So "reachable" there means **routable to**, not standable, and dimming its complement
/// dims only walls that are walled in on all four sides: the fence around a house stays lit and the
/// one dead cell in the middle of it goes dark. That looks arbitrary, because it is.
///
/// What is left after subtracting the walls is the honest question — *can I get to this square* —
/// which is what makes a map split by a ledge or a locked door read at a glance.
///
/// Drawn *after* the people so an NPC out of reach dims with the ground they are standing on.
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
///
/// Deterministic and search-free. vjeux's renderer runs A* with per-tile costs to route label
/// leaders across the whole of Kanto; one map with a handful of doors is a different problem, and a
/// fixed anchor order with a bounded nudge is both simpler and stable between renders — which the
/// committed checksums require.
///
/// ⚠️ **A label that cannot be placed is dropped, not forced.** That is lossless: the JSON beside
/// the picture lists every warp with its coordinates and destination, so the picture is a second
/// way of reading it rather than the only one.
fn layout_labels(map: &MetaTileMap, canvas: (usize, usize)) -> Vec<(Label, Placed)> {
    let mut placed: Vec<(Label, Placed)> = Vec::new();
    // ⚠️ **The player's square is reserved before any label is placed.** Labels are drawn last so
    // that terrain cannot swallow them, which means they are also drawn over the red ring — and in a
    // town with a lot of doors (Vermilion) one of them lands right on it. The ring is where every
    // coordinate the model reads is measured from, so it wins.
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
///
/// ⚠️ **A connection groups across the whole edge; a warp only groups with its neighbours.** They
/// look like the same problem and are not. Every cell of a map edge leads to the same place *by
/// definition*, but the strip is broken up by whatever is drawn along it — Pallet Town's northern
/// edge is fence posts and trees, which split `Route1` into four runs and had the picture saying
/// "Route1" four times across the top. A *warp* is the opposite: two doors into the same building
/// can be genuinely different doors landing in different rooms (Mt Moon B1F), so merging those on
/// destination alone would collapse two real choices into one label pointing between them.
fn collect_labels(map: &MetaTileMap) -> Vec<Label> {
    let mut groups: Vec<(Map, bool, usize, usize, usize, usize)> = Vec::new();
    for (index, &tile) in map.meta_tiles.iter().enumerate() {
        let (to_map, is_edge) = match tile {
            MetaTile::Warp { to_map, .. } => (to_map, false),
            MetaTile::Connection { to_map, .. } => (to_map, true),
            MetaTile::ConnectionWater(to_map) => (to_map, true),
            _ => continue,
        };
        let (x, y) = (index % map.width, index / map.width);
        match groups.iter_mut().find(|(m, edge, x0, y0, x1, y1)| {
            *m == to_map
                && *edge == is_edge
                // A door is one or two cells, so a warp joins only a box it touches.
                && (is_edge || (x + 1 >= *x0 && x <= *x1 + 1 && y + 1 >= *y0 && y <= *y1 + 1))
        }) {
            Some((_, _, x0, y0, x1, y1)) => {
                *x0 = (*x0).min(x);
                *y0 = (*y0).min(y);
                *x1 = (*x1).max(x);
                *y1 = (*y1).max(y);
            }
            None => groups.push((to_map, is_edge, x, y, x, y)),
        }
    }
    groups.into_iter()
        .map(|(to_map, _, x0, y0, x1, y1)| Label { cells: (x0, y0, x1, y1), text: wrap(&format!("{to_map}")) })
        .collect()
}

/// `ViridianForestSouthGate` → `["Viridian", "Forest", "South Gate"]`. Map names are CamelCase, and
/// eight pixels a character means a long one is wider than a small map.
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
        for y in placed.y..placed.y + placed.h {
            for x in placed.x..placed.x + placed.w {
                set(canvas, x, y, LABEL_PLATE);
            }
        }
        for (row, line) in label.text.iter().enumerate() {
            draw_text(canvas, line, placed.x as usize + 2, placed.y as usize + 2 + row * TILE_PX,
                      LABEL_INK, None);
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

/// ⚠️ Shade `0` is transparent for an overworld sprite — painting it draws every person in a box.
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

/// A three-pixel pip on the edge of the player's cell they are facing — the picture's answer to "and
/// which way am I pointing", which decides whether an `A` press talks to anyone.
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

/// `text` in the cartridge's own font. `plate` fills behind each glyph when the text has to sit over
/// busy art.
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

/// PNG bytes. Same three lines as every other encoder in the repo (`src/web/sprites.rs`).
pub fn encode(canvas: &RgbaImage) -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    canvas.write_to(&mut png, image::ImageFormat::Png).expect("an in-memory image encodes to PNG");
    png.into_inner()
}

/// The picture as a `data:` URL, ready for [`crate::llm::protocol::Message::user_with_image`].
pub fn data_url(canvas: &RgbaImage) -> String {
    let png = encode(canvas);
    let mut url = String::with_capacity(png.len() * 4 / 3 + 32);
    url.push_str("data:image/png;base64,");
    base64::Engine::encode_string(&base64::engine::general_purpose::STANDARD, &png, &mut url);
    url
}

/// What the picture is and how to read a coordinate off it. ⚠️ The formula is stated because it is
/// the whole reason the ruler is there: without it a model cannot turn "the door two squares left of
/// me" into the id the action menu wants.
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
         are those coordinates. Green is tall grass, blue is water, magenta is a warp, orange is a \
         ledge with an arrow for the only way it can be jumped, and anything you cannot walk to from \
         where you are standing is dimmed.{dark}",
        map.map, map.width, map.height, RULER_LEFT, RULER_TOP,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::integration_tests::fixture::TestFixture;
    use std::time::Duration;

    /// Fixtures chosen for what they make *drawable*, the way `soak`'s states are chosen for what
    /// they make reachable: an outdoor town with connection strips on two edges, a dense city, a
    /// cave with none, a `Plateau` map whose strip tileset differs from its own, and a map whose
    /// blocks are read from `wOverworldMap` at runtime rather than from ROM.
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

    /// Catches the two ways a renderer fails silently: drawing nothing, and drawing one flat colour
    /// because the tile lookup landed outside the sheet.
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

    /// **The load-bearing one.** The caption promises the model that the square at `(x, y)` is at
    /// pixel `(RULER_LEFT + 16x, RULER_TOP + 16y)`; this is what makes that true. If the ring drifts
    /// by a cell the picture still looks perfectly good and every coordinate the model reads off it
    /// is wrong.
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

    /// Two reads of one unchanged map must be the same picture. The renderer walks several
    /// `HashSet`s' worth of data (`reachable_tiles`, `warp_targets`), and a render that depended on
    /// their iteration order would read to the model as the world having moved, and would make the
    /// committed checksums flake rather than fail.
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

    /// Labels are the one part with a placement search in it. Whatever it decides, no two boxes may
    /// overlap and none may leave the canvas — a label half off the edge is worse than a dropped one,
    /// because the JSON already carries every warp.
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

    /// **The ground truth.** Everything else here checks that the picture is *well formed*; this
    /// checks that it is *right*, against the only authority there is — the tile ids the game itself
    /// has laid out for the PPU to draw.
    ///
    /// `wTileMap` is the 20×18 grid of background tile ids currently on screen, built by the
    /// cartridge's own `LoadCurrentMapView` from the same blocks and blockset this renderer walks.
    /// If a window of `MapMetadata::tile_id` matches it exactly, then the block map, the blockset
    /// indirection and the 4×4 tile order within a block are all correct — the three places an
    /// off-by-one still produces a picture that looks like a Pokémon map.
    ///
    /// ⚠️ The offset is *searched* rather than derived. The view's origin comes from
    /// `wCurrentTileBlockMapViewPointer` plus a half-block scroll, and rederiving that arithmetic
    /// here would be testing this test. A 360-tile exact agreement cannot happen at a wrong offset
    /// by accident, so finding one anywhere is the proof; where it is found is then checked against
    /// the player, which is what rules out a map uniform enough to match everywhere.
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
            // The view can hang off the edge of the map, where the game draws the border block and
            // this renderer draws nothing. Those cells are wildcards; the rest must agree.
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

            // …and the window the game is showing is the one around the player, which is what stops
            // a repetitive map from passing this at an arbitrary offset.
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


    /// What "dimmed" means, pinned — because the obvious reading of
    /// [`MetaTileMap::reachable_tiles`] is wrong and produces a picture that still looks like a map.
    ///
    /// That set is the routing BFS's keys, and the BFS deliberately records walls so a route can end
    /// at a door. Dimming its complement therefore lit every wall touching open floor and darkened
    /// only the cells walled in on all four sides — which on Pallet Town meant 18% of the map dimmed
    /// in a pattern with no relation to anything. The rule has to subtract the walls itself.
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
        // The dim is an alpha wash, so "dimmed" is "much darker than the same art undimmed". The
        // player's own square is the brightest thing that is definitely not dimmed.
        let wall = lit(wall_beside_a_street);
        let sum = |c: [u8; 4]| c[0] as u32 + c[1] as u32 + c[2] as u32;
        assert!(sum(wall) < 3 * 0x60,
                "a wall at {wall_beside_a_street:?} is routable-to, and must still read as \
                 out of reach — it rendered {wall:?}");

        // …and the door in the same town does not dim, because walking to it is the whole point.
        let door = (0..map.width * map.height)
            .map(|i| (i % map.width, i / map.width))
            .find(|&(x, y)| matches!(map.meta_tiles[x + y * map.width], MetaTile::Warp { .. })
                            && routable.contains(&Point8 { x: x as u8, y: y as u8 }))
            .expect("Pallet Town has reachable doors");
        assert!(sum(lit(door)) > sum(wall), "the door at {door:?} dimmed like a wall");
    }

    /// A CamelCase map name has to come apart into words a 12-character line can hold, and stay
    /// readable when it does.
    #[test]
    fn map_names_wrap_into_words() {
        assert_eq!(wrap("PalletTown"), vec!["Pallet Town"]);
        assert_eq!(wrap("Route1"), vec!["Route1"]);
        assert_eq!(wrap("ViridianForestSouthGate"), vec!["Viridian", "Forest South", "Gate"]);
        assert_eq!(wrap("OaksLab"), vec!["Oaks Lab"]);
    }

    /// `MetaTileMap` is `Default`, and a default one has no metadata to draw. The caller falls back
    /// to the ASCII grid rather than the turn failing.
    #[test]
    fn a_map_with_no_metadata_declines_to_render() {
        assert!(render(&MetaTileMap::default()).is_none());
    }

    /// Renders every committed save state. The five checksum fixtures cannot visit every
    /// tileset/blockset pairing in the game; this walks the lot looking only for a panic, which is
    /// what an off-by-one in a blockset index would be.
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

    /// Writes the renders out as real PNGs so a human can look at them, and prints what each one
    /// would cost the model. ⚠️ Prints a report rather than asserting, so it stays `#[ignore]`d on
    /// top of its feature gate, as `CLAUDE.md`'s table requires of every `probe_`.
    #[cfg(feature = "diagnostics")]
    #[test]
    #[ignore]
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
