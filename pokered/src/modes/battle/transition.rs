//! `engine/battle/battle_transitions.asm`'s `BattleTransition`, and
//! `SlidePlayerAndEnemySilhouettesOnScreen`'s slide, a frame at a time.
//!
//! Exact: which transition plays, each one's tiles and frames. Faithful: the waits before the
//! transition for the tile map and OAM to reach VRAM are not modelled, the tile map reaches the
//! screen whole where `AutoBgMapTransfer` moves a third a frame, and the OAM block
//! `BattleTransition` keeps for the enemy trainer is the caller's to name.

use poke_core::map::Map;
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use crate::gfx::layers::{Object, TileMap};
use crate::gfx::tiles::V_CHARS1;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::gfx::Screen;
use crate::mode::Ctx;

/// `vChars1` tile `$7F`, which `LoadBattleTransitionTile` makes black.
const BLACK: u8 = 0xFF;
const OBJECTS_PER_BLOCK: usize = 4;

/// The eight entries of `BattleTransitions`, by the three bits `GetBattleTransitionID_*` set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    DoubleCircle,
    Spiral,
    Circle,
    HorizontalStripes,
    Shrink,
    VerticalStripes,
    Split,
}

/// What `BattleTransition` reads to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// `wCurOpponent` at or past `OPP_ID_OFFSET`.
    pub trainer: bool,
    /// `wCurEnemyLevel`: a wild mon's, or a trainer's last mon's as `ReadTrainer` leaves it.
    pub enemy_level: u8,
    /// The level of the first party mon with HP.
    pub player_level: u8,
    pub map: Map,
}

impl Choice {
    /// `GetBattleTransitionID_*`: the transition, and for the spiral `wBattleTransitionSpiralDirection`
    /// as whether it winds inward.
    pub fn kind(&self) -> (Kind, bool) {
        let stronger = self.enemy_level >= self.player_level.wrapping_add(3);
        let id = self.trainer as u8 | (stronger as u8) << 1 | (is_dungeon_map(self.map) as u8) << 2;
        let kind = [Kind::DoubleCircle, Kind::Spiral, Kind::Circle, Kind::Spiral, Kind::HorizontalStripes, Kind::Shrink,
                    Kind::VerticalStripes, Kind::Split][id as usize];
        (kind, !stronger)
    }
}

/// `GetBattleTransitionID_IsDungeonMap`: `DungeonMaps1`, then `DungeonMaps2`'s ranges.
pub fn is_dungeon_map(map: Map) -> bool {
    let map = map as u8;
    let singles = rom_slice(sym::DungeonMaps1);
    if singles.iter().take_while(|&&m| m != 0xFF).any(|&m| m == map) {
        return true;
    }
    let ranges = rom_slice(sym::DungeonMaps2);
    let mut at = 0;
    while ranges[at] != 0xFF {
        let (low, high) = (ranges[at], ranges[at + 1]);
        at += 2;
        if map <= high {
            return map >= low;
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Op {
    Wait(u8),
    Black(usize, usize),
    /// `BattleTransition_Shrink`'s or `BattleTransition_Split`'s four copies.
    Shrink,
    Split,
    Bgp(u8),
    /// `BattleTransition_BlackScreen`.
    BlackScreen,
    ClearSprites,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BattleTransition {
    ops: VecDeque<Op>,
    wait: u8,
}

impl BattleTransition {
    /// The transition `choice` names over the screen as it stands, keeping OAM block
    /// `trainer_block` if the enemy trainer's sprite is in it.
    pub fn new(choice: Choice, trainer_block: Option<u8>, screen: &mut Screen) -> Self {
        materialize_map(screen);
        let blocks = (1..10u8).filter(|&block| Some(block) != trainer_block);
        if screen.sprites.len() < 40 {
            screen.sprites.resize(40, Object::default());
        }
        for block in blocks {
            let at = block as usize * OBJECTS_PER_BLOCK;
            screen.sprites[at..at + OBJECTS_PER_BLOCK].fill(Object::default());
        }
        screen.tiles.load(V_CHARS1 + 0x7F, &rom_slice(sym::BattleTransitionTile)[..TILE_BYTES]);

        let mut ops = VecDeque::new();
        let (kind, inward) = choice.kind();
        match kind {
            Kind::DoubleCircle => {
                flash_screen(&mut ops);
                let (first, second) = (half_circle(sym::BattleTransition_HalfCircle1), half_circle(sym::BattleTransition_HalfCircle2));
                for (a, b) in first.iter().zip(&second) {
                    ops.extend(a.iter().chain(b).map(|&(x, y)| Op::Black(x, y)));
                    ops.push_back(Op::Wait(3));
                }
            }
            Kind::Circle => {
                flash_screen(&mut ops);
                for half in [sym::BattleTransition_HalfCircle1, sym::BattleTransition_HalfCircle2] {
                    for cells in half_circle(half) {
                        ops.extend(cells.into_iter().map(|(x, y)| Op::Black(x, y)));
                        ops.push_back(Op::Wait(3));
                    }
                }
            }
            Kind::Spiral if inward => inward_spiral(&mut ops),
            Kind::Spiral => outward_spiral(&mut ops, &screen.ui),
            Kind::HorizontalStripes => {
                for i in 0..SCREEN_TILES_X {
                    ops.extend((0..SCREEN_TILES_Y).step_by(2).map(|y| Op::Black(i, y)));
                    ops.extend((1..SCREEN_TILES_Y).step_by(2).map(|y| Op::Black(SCREEN_TILES_X - 1 - i, y)));
                    ops.push_back(Op::Wait(3));
                }
            }
            Kind::VerticalStripes => {
                for i in 0..SCREEN_TILES_Y {
                    ops.extend((0..SCREEN_TILES_X).step_by(2).map(|x| Op::Black(x, i)));
                    ops.extend((1..SCREEN_TILES_X).step_by(2).map(|x| Op::Black(x, SCREEN_TILES_Y - 1 - i)));
                    ops.push_back(Op::Wait(3));
                }
            }
            Kind::Shrink => {
                for _ in 0..SCREEN_TILES_Y / 2 {
                    ops.extend([Op::Shrink, Op::Wait(6)]);
                }
            }
            Kind::Split => {
                for _ in 0..SCREEN_TILES_Y / 2 {
                    ops.extend([Op::Split, Op::Wait(6)]);
                }
            }
        }
        ops.push_back(Op::BlackScreen);
        if matches!(kind, Kind::Shrink | Kind::Split) {
            ops.push_back(Op::Wait(10));
        }
        ops.push_back(Op::ClearSprites);
        Self { ops, wait: 0 }
    }

    /// One frame. True once the transition is over, in this frame.
    pub fn update(&mut self, ctx: &mut Ctx) -> bool {
        let instant = ctx.pacing == crate::Pacing::Instant;
        loop {
            if self.wait > 0 {
                self.wait -= 1;
                if self.wait > 0 {
                    return false;
                }
            }
            let Some(op) = self.ops.pop_front() else { return true };
            let screen = &mut *ctx.screen;
            match op {
                Op::Wait(frames) => self.wait = if instant { 0 } else { frames + 1 },
                Op::Black(x, y) => screen.ui.set(x, y, BLACK),
                Op::Shrink => shrink(&mut screen.ui),
                Op::Split => split(&mut screen.ui),
                Op::Bgp(bgp) => screen.effects.bgp = bgp,
                Op::BlackScreen => {
                    screen.effects.bgp = 0xFF;
                    screen.effects.obp0 = 0xFF;
                    screen.effects.obp1 = 0xFF;
                }
                Op::ClearSprites => crate::gfx::mon_icons::clear_sprites(&mut screen.sprites),
            }
        }
    }
}

/// The map drawn into every cell nothing covers, as `wTileMap` always holds it on the cartridge.
fn materialize_map(screen: &mut Screen) {
    let (cx, cy) = screen.map.camera;
    for y in 0..SCREEN_TILES_Y {
        for x in 0..SCREEN_TILES_X {
            if screen.ui.cover(x, y).is_none() {
                let tile = screen.map.tile_at(cx + 8 * x as i32, cy + 8 * y as i32).unwrap_or(UiSurface::BLANK);
                screen.ui.set(x, y, tile);
            }
        }
    }
}

/// `BattleTransition_FlashScreen`: `BattleTransition_FlashScreenPalettes` three times, two frames each.
fn flash_screen(ops: &mut VecDeque<Op>) {
    let palettes: Vec<u8> = rom_slice(sym::BattleTransition_FlashScreenPalettes).iter().copied().take_while(|&p| p != 1).collect();
    for _ in 0..3 {
        for &bgp in &palettes {
            ops.extend([Op::Bgp(bgp), Op::Wait(2)]);
        }
    }
}

/// `BattleTransition_Circle_Sub1`'s ten steps of one half: the cells `BattleTransition_Circle_Sub3`
/// blackens for each `half_circle` entry.
fn half_circle(table: poke_core::symbols::DmgPointer) -> Vec<Vec<(usize, usize)>> {
    let entries = rom_slice(table);
    let bank = table.bank;
    (0..10).map(|step| {
        let entry = &entries[step * 5..step * 5 + 5];
        let right = entry[0] != 0;
        let data = rom_slice(poke_core::symbols::DmgPointer { bank, address: u16::from_le_bytes([entry[1], entry[2]]) });
        let target = u16::from_le_bytes([entry[3], entry[4]]) - sym::wTileMap.address;
        // The first half runs down the screen and the second up it, `wBattleTransitionCircleScreenQuadrantY`.
        let upward = table == sym::BattleTransition_HalfCircle2;
        let mut at = target as i32;
        let mut cells = vec![];
        let mut data = data.iter();
        loop {
            let run = *data.next().expect("circle data");
            let row_start = at;
            for _ in 0..run {
                cells.push(((at.rem_euclid(20)) as usize, (at.div_euclid(20)) as usize));
                at += if right { 1 } else { -1 };
            }
            at = row_start + if upward { -20 } else { 20 };
            let back = *data.next().expect("circle data");
            if back == 0xFF {
                break;
            }
            at += if right { -(back as i32) } else { back as i32 };
        }
        cells
    }).collect()
}

/// `BattleTransition_InwardSpiral`: the edges inward, a transfer every seven tiles.
fn inward_spiral(ops: &mut VecDeque<Op>) {
    let mut counter = 7u8;
    let mut at = 0i32;
    let mut run = |ops: &mut VecDeque<Op>, at: &mut i32, count: u8, step: i32| {
        for _ in 0..count {
            ops.push_back(Op::Black(at.rem_euclid(20) as usize, at.div_euclid(20) as usize));
            *at += step;
            counter -= 1;
            if counter == 0 {
                ops.push_back(Op::Wait(3));
                counter = 7;
            }
        }
    };
    let mut c = (SCREEN_TILES_Y - 1) as u8;
    run(ops, &mut at, c, 20);
    c += 1;
    let mut first = true;
    loop {
        if !first {
            run(ops, &mut at, c, 20);
        }
        first = false;
        c += 1;
        run(ops, &mut at, c, 1);
        c -= 2;
        run(ops, &mut at, c, -20);
        c += 1;
        run(ops, &mut at, c, -1);
        c -= 2;
        if c == 0 {
            break;
        }
    }
}

/// `BattleTransition_OutwardSpiral_` 360 times from (10, 10), a frame every three tiles. It turns
/// wherever the tile on its inside is not yet black, reading past the tile map's edges, where
/// nothing it has not written itself is black.
fn outward_spiral(ops: &mut VecDeque<Op>, ui: &UiSurface) {
    let mut black: HashSet<i32> = (0..SCREEN_TILES_X * SCREEN_TILES_Y)
        .filter(|&i| ui.get(i % SCREEN_TILES_X, i / SCREEN_TILES_X) == BLACK)
        .map(|i| i as i32)
        .collect();
    let mut at = 10 * 20 + 10i32;
    let mut direction = 3;
    for _ in 0..120 {
        for _ in 0..3 {
            let (inside, forward) = match direction {
                0 => (-1, -20),
                1 => (20, -1),
                2 => (1, 20),
                _ => (-20, 1),
            };
            if black.contains(&(at + inside)) {
                at += forward;
            } else {
                at += inside;
                direction = (direction + 1) % 4;
            }
            black.insert(at);
            if (0..(SCREEN_TILES_X * SCREEN_TILES_Y) as i32).contains(&at) {
                ops.push_back(Op::Black(at as usize % SCREEN_TILES_X, at as usize / SCREEN_TILES_X));
            }
        }
        ops.push_back(Op::Wait(1));
    }
}

fn copy_row(ui: &mut UiSurface, from: usize, to: usize) {
    for x in 0..SCREEN_TILES_X {
        let tile = ui.get(x, from);
        ui.set(x, to, tile);
    }
}

fn copy_column(ui: &mut UiSurface, from: usize, to: usize) {
    for y in 0..SCREEN_TILES_Y {
        let tile = ui.get(from, y);
        ui.set(to, y, tile);
    }
}

/// `BattleTransition_CopyTiles1` from row `first` towards `last`, each row into the one beside it,
/// and `BattleTransition_CopyTiles2` likewise for columns, the row or column left behind black.
fn rows_towards(ui: &mut UiSurface, from: [usize; 8], step: isize) {
    for &row in &from {
        copy_row(ui, row, (row as isize + step) as usize);
    }
    let last = from[7];
    for x in 0..SCREEN_TILES_X {
        ui.set(x, last, BLACK);
    }
}

fn columns_towards(ui: &mut UiSurface, from: [usize; 9], step: isize) {
    for &column in &from {
        copy_column(ui, column, (column as isize + step) as usize);
    }
    let last = from[8];
    for y in 0..SCREEN_TILES_Y {
        ui.set(last, y, BLACK);
    }
}

/// `BattleTransition_Shrink`'s step: the top half down a row, the bottom half up, the left half
/// right a column and the right half left.
fn shrink(ui: &mut UiSurface) {
    rows_towards(ui, [7, 6, 5, 4, 3, 2, 1, 0], 1);
    rows_towards(ui, [10, 11, 12, 13, 14, 15, 16, 17], -1);
    columns_towards(ui, [8, 7, 6, 5, 4, 3, 2, 1, 0], 1);
    columns_towards(ui, [11, 12, 13, 14, 15, 16, 17, 18, 19], -1);
}

/// `BattleTransition_Split`'s step: the halves apart, from the middle outwards.
fn split(ui: &mut UiSurface) {
    rows_towards(ui, [16, 15, 14, 13, 12, 11, 10, 9], 1);
    rows_towards(ui, [1, 2, 3, 4, 5, 6, 7, 8], -1);
    columns_towards(ui, [18, 17, 16, 15, 14, 13, 12, 11, 10], 1);
    columns_towards(ui, [1, 2, 3, 4, 5, 6, 7, 8, 9], -1);
}

/// `SlidePlayerAndEnemySilhouettesOnScreen`'s loop: the enemy's picture scrolled in from the left
/// above line `$40`, the player's body from the right on lines `$40` to `$5F`, the head in OAM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Silhouettes {
    frame: u8,
}

/// How many frames the slide takes: `$90` pixels, two a frame.
pub const SILHOUETTE_FRAMES: u8 = 0x90 / 2;
/// The `hSCX` the slide's last pass leaves, which nothing sets back.
pub const SILHOUETTES_SCX: u8 = 2;

impl Silhouettes {
    pub fn new(screen: &mut Screen) -> Self {
        let mut background = TileMap::filled(UiSurface::BLANK);
        for y in 0..SCREEN_TILES_Y {
            for x in 0..SCREEN_TILES_X {
                background.set(x, y, screen.ui.get(x, y));
            }
        }
        screen.background = Some(background);
        screen.tiles.load_font();
        screen.tiles.load_hp_bar_and_status_tiles();
        crate::modes::battle::hud::load_hud_tiles(&mut screen.tiles);
        // `LoadPlayerBackPic` copies the merged picture to `vSprites` too, for the head.
        for tile in 0..49u8 {
            let bytes = *screen.tiles.bg(crate::modes::battle::hud::BACK_PIC_TILE + tile);
            screen.tiles.load(crate::gfx::tiles::V_CHARS0 + tile as usize, &bytes);
        }
        let mut sprites = vec![Object::default(); 40];
        for column in 0..7u8 {
            for row in 0..3u8 {
                sprites[(column * 3 + row) as usize] = Object { y: 0x38 + 8 * row, x: 0xA0 + 8 * column, tile: column * 7 + row, attributes: 0 };
            }
        }
        screen.sprites = sprites;
        Self { frame: 0 }
    }

    /// One frame of the slide. True once it is over, the screen back to the tile map.
    pub fn update(&mut self, screen: &mut Screen) -> bool {
        if self.frame == SILHOUETTE_FRAMES {
            screen.background = None;
            screen.effects.line_scx = None;
            screen.effects.scx = 0;
            for sprite in screen.sprites.iter_mut() {
                sprite.y = 160;
            }
            return true;
        }
        let k = self.frame;
        // The palettes are written a few lines into the slide's first frame, so it shows the old ones.
        if k == 1 {
            screen.effects.bgp = 0b11100100;
            screen.effects.obp0 = 0b11100100;
            screen.effects.obp1 = 0b11100100;
        }
        let top = if k == 0 { 0x90 } else { 0x90 - 2 * (k - 1) };
        let body = 0x70u8.wrapping_add(2 * k);
        screen.effects.line_scx = Some((0..144).map(|line| match line { 0..0x40 => top, 0x40..0x60 => body, _ => 0 }).collect());
        for sprite in screen.sprites.iter_mut().take(21) {
            sprite.x = sprite.x.wrapping_sub(if k == 0 { 0 } else { 2 });
        }
        self.frame += 1;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dungeon_maps_are_the_lists_and_their_ranges() {
        assert!(is_dungeon_map(Map::ViridianForest));
        assert!(is_dungeon_map(Map::MtMoonB2F));
        assert!(is_dungeon_map(Map::PokemonTower3F));
        assert!(!is_dungeon_map(Map::PalletTown));
        assert!(!is_dungeon_map(Map::SilphCo1F));
        assert!(!is_dungeon_map(Map::DiglettsCave));
    }

    #[test]
    fn a_trainer_three_levels_up_in_a_dungeon_splits_the_screen() {
        let choice = Choice { trainer: true, enemy_level: 13, player_level: 10, map: Map::MtMoon1F };
        assert_eq!(choice.kind(), (Kind::Split, false));
        let weaker = Choice { enemy_level: 12, ..choice };
        assert_eq!(weaker.kind(), (Kind::Shrink, true));
    }

    #[derive(serde::Deserialize)]
    struct TransitionInput { choice: Choice }

    #[test]
    fn every_harvested_case_of_get_battle_transition_id() {
        for (input, (id, direction), _) in crate::fixtures::cases::<TransitionInput, (u8, u8)>(include_str!("../../../fixtures/battle_anims/get_battle_transition_id.jsonl")) {
            let (kind, inward) = input.choice.kind();
            let ours = match kind {
                Kind::DoubleCircle => 0,
                Kind::Spiral if inward => 1,
                Kind::Circle => 2,
                Kind::Spiral => 3,
                Kind::HorizontalStripes => 4,
                Kind::Shrink => 5,
                Kind::VerticalStripes => 6,
                Kind::Split => 7,
            };
            assert_eq!((ours, inward as u8), (id, direction), "{:?}", input.choice);
        }
    }

    #[test]
    fn the_spirals_blacken_the_whole_screen() {
        let mut ops = VecDeque::new();
        inward_spiral(&mut ops);
        let cells: HashSet<(usize, usize)> = ops.iter().filter_map(|op| match op { Op::Black(x, y) => Some((*x, *y)), _ => None }).collect();
        let missed: Vec<(usize, usize)> = (0..SCREEN_TILES_Y).flat_map(|y| (0..SCREEN_TILES_X).map(move |x| (x, y))).filter(|cell| !cells.contains(cell)).collect();
        assert_eq!(missed, vec![(9, 8)], "the inward spiral stops a tile short, and the black screen covers it");
        let mut ops = VecDeque::new();
        outward_spiral(&mut ops, &UiSurface::default());
        assert_eq!(ops.iter().filter(|op| matches!(op, Op::Wait(_))).count(), 120);
    }

    #[test]
    fn each_half_circle_step_blackens_cells_on_the_screen() {
        for half in [sym::BattleTransition_HalfCircle1, sym::BattleTransition_HalfCircle2] {
            let steps = half_circle(half);
            assert_eq!(steps.len(), 10);
            assert!(steps.iter().all(|cells| !cells.is_empty() && cells.iter().all(|&(x, y)| x < 20 && y < 18)));
        }
    }
}
