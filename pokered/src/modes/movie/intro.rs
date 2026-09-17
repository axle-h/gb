//! `PlayIntro`: `PlayShootingStar`'s copyright and Game Freak's shooting star (`splash.asm`), then
//! `PlayIntroScene`'s Gengar and Nidorino (`intro.asm`), then `GBFadeOutToWhite`.
//!
//! A new A or START cuts the star short and ends the scene, as `CheckForUserInterruption` does. Only
//! the star's own `ret c` and the scene's first move stop what follows them: a press during a later
//! move ends that move and the scene carries on.
//!
//! Loading, and not modelled: `ClearScreen`'s `Delay3`, the LCD-off graphics load and `ClearVram`,
//! the `CopyVideoData` frames of the star's tiles, and the `Delay3` before the scene.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::{pokered_symbols as sym, DmgPointer};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, AudioBank, SoundId};
use crate::gfx::layers::{Object, TileMap};
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::tiles::{TileData, V_CHARS0, V_CHARS1, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::mode::Ctx;
use super::screen::{between, clear_screen, copy_tile_ids, Dest, MovieScreen, BGP_NORMAL, OBP0_NORMAL};
use super::wait::{Tick, Wait};

/// `TILEMAP_GENGAR_INTRO_1`; the next two follow it.
const TILEMAP_GENGAR_INTRO_1: usize = 3;
/// `ANIMATION_END`.
const ANIMATION_END: u8 = 80;
/// Tiles each Nidorino pose takes: `(FightIntroFrontMon2 - FightIntroFrontMon) / TILE_SIZE`.
const POSE_TILES: u8 = 36;
/// `SCREEN_HEIGHT_PX + OAM_Y_OFS`, which hides an object.
const OFF_SCREEN_Y: u8 = 160;
/// Where `.bigStarLoop` stops: the star's top-left object at this `Y`.
const BIG_STAR_OFF_BOTTOM: u8 = 0xA0;
/// `PlayShootingStar`'s holds.
const COPYRIGHT_FRAMES: u16 = 180;
const BARS_FRAMES: u16 = 64;
const AFTER_STAR_FRAMES: u16 = 40;
/// `GBFadeOutToWhite`'s and `GBFadeInFromWhite`'s frames a palette, and how many palettes.
pub const FADE_STEP: u16 = 8;
pub const FADE_STEPS: u8 = 3;

/// `IntroMoveMon`'s `e`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Move {
    NidorinoRight,
    GengarRight,
    GengarLeft,
}

/// One statement of `PlayIntroScene`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Op {
    CopyTiles(usize),
    Sound(SoundId),
    BaseTile(u8),
    /// `AnimateIntroNidorino` over `IntroNidorinoAnimationN`.
    Animate(u8),
    /// `IntroMoveMon`, two pixels a step; `true` where the scene's `ret c` follows it.
    Move(u8, Move, bool),
    Check(u16),
}

const SCENE: &[Op] = &[
    Op::Move(80 / 2, Move::NidorinoRight, true),
    // hip, hop, hip, hop
    Op::Sound(sounds::SFX_INTRO_HIP), Op::BaseTile(0), Op::Animate(1),
    Op::Sound(sounds::SFX_INTRO_HOP), Op::Animate(2), Op::Check(10),
    Op::Sound(sounds::SFX_INTRO_HIP), Op::Animate(1),
    Op::Sound(sounds::SFX_INTRO_HOP), Op::Animate(2), Op::Check(30),
    // raise
    Op::CopyTiles(TILEMAP_GENGAR_INTRO_1 + 1), Op::Sound(sounds::SFX_INTRO_RAISE),
    Op::Move(8 / 2, Move::GengarLeft, false), Op::Check(30),
    // slash
    Op::CopyTiles(TILEMAP_GENGAR_INTRO_1 + 2), Op::Sound(sounds::SFX_INTRO_CRASH),
    Op::Move(16 / 2, Move::GengarRight, false),
    Op::Sound(sounds::SFX_INTRO_HIP), Op::BaseTile(POSE_TILES), Op::Animate(3), Op::Check(30),
    Op::Move(8 / 2, Move::GengarLeft, false), Op::CopyTiles(TILEMAP_GENGAR_INTRO_1), Op::Check(60),
    // hip, hop
    Op::Sound(sounds::SFX_INTRO_HIP), Op::BaseTile(0), Op::Animate(4),
    Op::Sound(sounds::SFX_INTRO_HOP), Op::Animate(5), Op::Check(20),
    Op::BaseTile(POSE_TILES), Op::Animate(6), Op::Check(30),
    // lunge
    Op::Sound(sounds::SFX_INTRO_LUNGE), Op::BaseTile(2 * POSE_TILES), Op::Animate(7),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Copyright,
    Bars,
    StarStart,
    BigStar,
    /// The logo's flashes done so far.
    Flash(u8),
    /// A wave of small stars and how many of `MoveDownSmallStars`' eight steps are done.
    SmallStars { wave: u8, step: u8 },
    Music,
    /// `PlayIntroScene`'s statement, how far into it (an animation's entry or a move's step), and
    /// whether the step under way has its wait running.
    Scene { op: usize, progress: u8, waiting: bool },
    /// `GBFadeOutToWhite`'s palettes shown so far, then `PlayIntro`'s last `DelayFrame`.
    Fade(u8),
    LastFrame,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intro {
    phase: Phase,
    wait: Wait,
    /// `wIntroNidorinoBaseTile`.
    base_tile: u8,
    /// `wMoveDownSmallStarsOAMCount`.
    small_stars: u8,
}

impl Default for Intro {
    fn default() -> Self {
        Self { phase: Phase::Copyright, wait: Wait::default(), base_tile: 0, small_stars: 0 }
    }
}

fn fade_palette(n: u16) -> [u8; 3] {
    let table = rom_slice(sym::FadePal1 + (n - 1) * 3);
    [table[0], table[1], table[2]]
}

fn set_palettes(ctx: &mut Ctx, [bgp, obp0, obp1]: [u8; 3]) {
    (ctx.screen.effects.bgp, ctx.screen.effects.obp0, ctx.screen.effects.obp1) = (bgp, obp0, obp1);
}

/// `GBFadeOutToWhite`'s palette `n`, from `FadePal6` on.
pub fn fade_out_to_white(ctx: &mut Ctx, n: u8) {
    set_palettes(ctx, fade_palette(6 + n as u16));
}

/// `GBFadeInFromWhite`'s palette `n`, from `FadePal7` down.
pub fn fade_in_from_white(ctx: &mut Ctx, n: u8) {
    set_palettes(ctx, fade_palette(7 - n as u16));
}

/// `GBPalWhiteOut`.
pub fn white_out(ctx: &mut Ctx) {
    set_palettes(ctx, [0, 0, 0]);
}

/// `GBPalNormal`, which leaves `rOBP1` alone.
pub fn pal_normal(ctx: &mut Ctx) {
    ctx.screen.effects.bgp = BGP_NORMAL;
    ctx.screen.effects.obp0 = OBP0_NORMAL;
}

pub fn objects(bytes: &[u8]) -> impl Iterator<Item = Object> + '_ {
    bytes.chunks_exact(4).map(|o| Object { y: o[0], x: o[1], tile: o[2], attributes: o[3] })
}

fn audio_bank(label: DmgPointer) -> AudioBank {
    AudioBank::from_rom_bank(label.bank.id()).expect("a sound lives in an audio bank")
}

impl Intro {
    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    /// One frame. The caller runs `screen`'s `vblank` before and its `present` after.
    pub fn update(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        let mut interrupted = match self.wait.tick(ctx) {
            Tick::Waiting => return,
            Tick::Done => false,
            Tick::Interrupted => true,
        };
        while !self.wait.is_running() && self.phase != Phase::Done {
            self.step(ctx, screen, interrupted);
            interrupted = false;
        }
    }

    fn step(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen, interrupted: bool) {
        match self.phase {
            Phase::Copyright => self.copyright(ctx, screen),
            Phase::Bars => self.bars(ctx, screen),
            Phase::StarStart => {
                load_shooting_star_graphics(ctx, screen);
                ctx.audio.play_sound(sounds::SFX_SHOOTING_STAR);
                self.move_big_star(screen);
                self.phase = Phase::BigStar;
            }
            Phase::BigStar if interrupted => self.phase = Phase::Music,
            Phase::BigStar if screen.oam[0].y != BIG_STAR_OFF_BOTTOM => self.move_big_star(screen),
            Phase::BigStar => {
                for object in &mut screen.oam[..4] {
                    object.y = OFF_SCREEN_Y;
                }
                self.flash_logo(ctx, 0);
            }
            Phase::Flash(_) if interrupted => self.phase = Phase::Music,
            Phase::Flash(n) if n < 3 => self.flash_logo(ctx, n),
            Phase::Flash(_) => {
                let small = rom_slice(sym::SmallStarsOAM);
                let object = objects(&small[..4]).next().expect("one object");
                screen.oam[..24].fill(object);
                self.small_stars = 0;
                self.start_wave(ctx, screen, 0);
            }
            Phase::SmallStars { wave, step } => {
                if interrupted || step == 8 {
                    // The waves shift down to make room for the next, even when cut short.
                    screen.oam.copy_within(4..24, 0);
                }
                if interrupted {
                    self.phase = Phase::Music;
                } else if step < 8 {
                    self.move_down_small_stars(ctx, screen, wave, step);
                } else if wave + 1 < 6 {
                    self.start_wave(ctx, screen, wave + 1);
                } else {
                    self.phase = Phase::Music;
                    self.wait = Wait::frames(AFTER_STAR_FRAMES);
                }
            }
            Phase::Music => {
                ctx.audio.set_bank(audio_bank(sym::Music_IntroBattle));
                ctx.audio.play_new_sound(sounds::MUSIC_INTRO_BATTLE.id);
                // `IntroClearMiddleOfScreen`.
                ctx.screen.ui.fill(0, 4, SCREEN_TILES_X, 10, 0);
                screen.clear_sprites();
                self.scene_prologue(ctx, screen);
            }
            Phase::Scene { op, progress, waiting } => self.scene(ctx, screen, op, progress, waiting, interrupted),
            Phase::Fade(n) if n < FADE_STEPS => {
                fade_out_to_white(ctx, n);
                self.phase = Phase::Fade(n + 1);
                self.wait = Wait::frames(FADE_STEP);
            }
            Phase::Fade(_) => {
                screen.scx = 0;
                screen.transfer = None;
                screen.clear_sprites();
                self.phase = Phase::LastFrame;
                self.wait = Wait::frames(1);
            }
            Phase::LastFrame => {
                // `Init` after `PlayIntro`: `ClearVram`, `GBPalNormal`, `ClearSprites`.
                screen.maps = [TileMap::filled(0), TileMap::filled(0)];
                ctx.screen.tiles = TileData::default();
                pal_normal(ctx);
                screen.clear_sprites();
                self.phase = Phase::Done;
            }
            Phase::Done => {}
        }
    }

    /// `Init`'s tail and `PlayShootingStar` to its first hold.
    fn copyright(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        ctx.audio.stop_all_sounds();
        ctx.audio.set_bank(audio_bank(sym::SFX_Shooting_Star));
        screen.transfer = Some(Dest::BG_MAP1);
        ctx.screen.sgb.run(&PaletteCommand::GameFreakIntro);
        // `LoadCopyrightAndTextBoxTiles`. The window it shows `vBGMap1` through covers the whole
        // screen, which is the background showing `vBGMap1` from this frame on.
        screen.wy = 0;
        screen.bg_map = 1;
        clear_screen(&mut ctx.screen.ui);
        ctx.screen.tiles.load_text_box_tiles();
        load_copyright_tiles(ctx);
        ctx.screen.effects.bgp = BGP_NORMAL;
        self.phase = Phase::Bars;
        self.wait = Wait::frames(COPYRIGHT_FRAMES);
    }

    /// `IntroDrawBlackBars` and `LoadIntroGraphics` with the LCD off, then the background switched
    /// to `vBGMap1`, whose bars run the whole 32 tiles.
    fn bars(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        const BLACK: u8 = 1;
        clear_screen(&mut ctx.screen.ui);
        screen.maps[1] = TileMap::filled(0);
        for row in (0..4).chain(14..18) {
            ctx.screen.ui.fill(0, row, SCREEN_TILES_X, 1, BLACK);
            for column in 0..TileMap::SIZE {
                screen.maps[1].set(column, row, BLACK);
            }
        }
        let back = between(sym::FightIntroBackMon, sym::FightIntroBackMonEnd);
        let game_freak = between(sym::GameFreakIntro, sym::GameFreakIntroEnd);
        ctx.screen.tiles.load(V_CHARS2, back);
        ctx.screen.tiles.load(V_CHARS2 + back.len() / TILE_BYTES, game_freak);
        ctx.screen.tiles.load(V_CHARS1, game_freak);
        ctx.screen.tiles.load(V_CHARS0, between(sym::FightIntroFrontMon, sym::FightIntroFrontMonEnd));
        screen.bg_map = 1;
        screen.wy = 144;
        self.phase = Phase::StarStart;
        self.wait = Wait::frames(BARS_FRAMES);
    }

    /// One pass of `.bigStarLoop`: the star four pixels down and left.
    fn move_big_star(&mut self, screen: &mut MovieScreen) {
        for object in &mut screen.oam[..4] {
            object.y = object.y.wrapping_add(4);
            object.x = object.x.wrapping_sub(4);
        }
        self.wait = Wait::check(1);
    }

    /// `.flashLogoLoop`: `rOBP0` rotated two bits.
    fn flash_logo(&mut self, ctx: &mut Ctx, n: u8) {
        ctx.screen.effects.obp0 = ctx.screen.effects.obp0.rotate_right(2);
        self.phase = Phase::Flash(n + 1);
        self.wait = Wait::check(10);
    }

    /// `.smallStarsInnerLoop` for one wave, then its first step down.
    fn start_wave(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen, wave: u8) {
        let table = rom_slice(sym::SmallStarsWaveCoordsPointerTable);
        let pointer = u16::from_le_bytes([table[wave as usize * 2], table[wave as usize * 2 + 1]]);
        let coords = rom_slice(DmgPointer { address: pointer, ..sym::SmallStarsWaveCoordsPointerTable });
        if coords[0] != 0xFF {
            for (i, pair) in coords.chunks_exact(2).take(4).enumerate() {
                screen.oam[20 + i].y = pair[0];
                screen.oam[20 + i].x = pair[1];
            }
            // Six rather than four, but the two extra are never on the screen.
            if self.small_stars != 24 {
                self.small_stars += 6;
            }
        }
        self.move_down_small_stars(ctx, screen, wave, 0);
    }

    /// One of `MoveDownSmallStars`' steps: every star a pixel down, the lower star's shade blinked.
    fn move_down_small_stars(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen, wave: u8, step: u8) {
        for i in 0..self.small_stars as usize {
            let slot = &mut screen.oam[23 - i];
            slot.y = slot.y.wrapping_add(1);
        }
        ctx.screen.effects.obp1 ^= 0b1010_0000;
        self.phase = Phase::SmallStars { wave, step: step + 1 };
        self.wait = Wait::check(3);
    }

    /// `PlayIntroScene` before its first move.
    fn scene_prologue(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        ctx.screen.sgb.run(&PaletteCommand::NidorinoIntro);
        set_palettes(ctx, [BGP_NORMAL; 3]);
        screen.scx = 0;
        copy_tile_ids(&mut ctx.screen.ui, 13, 7, TILEMAP_GENGAR_INTRO_1, 0);
        // `InitIntroNidorinoOAM`: six columns of six, from (0, 88).
        for column in 0..6 {
            for row in 0..6 {
                let tile = column * 6 + row;
                screen.oam[tile as usize] = Object { y: 80 + 8 * (row + 1), x: 8 * column, tile, attributes: Object::BEHIND_BG };
            }
        }
        self.phase = Phase::Scene { op: 0, progress: 0, waiting: false };
    }

    /// `UpdateIntroNidorinoOAM`: every object moved by `(dy, dx)` and given the pose's tiles.
    fn update_nidorino_oam(&self, screen: &mut MovieScreen, dy: u8, dx: u8) {
        for (i, object) in screen.oam[..POSE_TILES as usize].iter_mut().enumerate() {
            object.y = object.y.wrapping_add(dy);
            object.x = object.x.wrapping_add(dx);
            object.tile = self.base_tile.wrapping_add(i as u8);
        }
    }

    fn scene(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen, op: usize, progress: u8, waiting: bool, interrupted: bool) {
        let Some(&statement) = SCENE.get(op) else {
            self.phase = Phase::Fade(0);
            return;
        };
        let next = Phase::Scene { op: op + 1, progress: 0, waiting: false };
        match statement {
            Op::CopyTiles(entry) => {
                copy_tile_ids(&mut ctx.screen.ui, 13, 7, entry, 0);
                self.phase = next;
            }
            Op::Sound(id) => {
                ctx.audio.play_sound(id);
                self.phase = next;
            }
            Op::BaseTile(tile) => {
                self.base_tile = tile;
                self.phase = next;
            }
            Op::Animate(n) => {
                let table = rom_slice(animation_table(n));
                let entry = &table[progress as usize * 2..];
                if entry[0] == ANIMATION_END {
                    self.phase = next;
                    return;
                }
                self.update_nidorino_oam(screen, entry[0], entry[1]);
                self.phase = Phase::Scene { op, progress: progress + 1, waiting: true };
                self.wait = Wait::frames(5);
            }
            Op::Move(steps, kind, ends_scene) => {
                if waiting && interrupted {
                    self.phase = if ends_scene { Phase::Fade(0) } else { next };
                    return;
                }
                if waiting && progress == steps {
                    self.phase = next;
                    return;
                }
                match kind {
                    Move::NidorinoRight => {
                        self.update_nidorino_oam(screen, 0, 2);
                        screen.scx = screen.scx.wrapping_add(2);
                    }
                    Move::GengarLeft => screen.scx = screen.scx.wrapping_add(2),
                    Move::GengarRight => screen.scx = screen.scx.wrapping_sub(2),
                }
                self.phase = Phase::Scene { op, progress: progress + 1, waiting: true };
                self.wait = Wait::check(2);
            }
            Op::Check(frames) => {
                if !waiting {
                    self.phase = Phase::Scene { op, progress, waiting: true };
                    self.wait = Wait::check(frames);
                } else {
                    self.phase = if interrupted { Phase::Fade(0) } else { next };
                }
            }
        }
    }
}

/// `IntroNidorinoAnimationN`: pairs of `(dy, dx)` to `ANIMATION_END`.
fn animation_table(n: u8) -> DmgPointer {
    [sym::IntroNidorinoAnimation1, sym::IntroNidorinoAnimation2, sym::IntroNidorinoAnimation3,
     sym::IntroNidorinoAnimation4, sym::IntroNidorinoAnimation5, sym::IntroNidorinoAnimation6,
     sym::IntroNidorinoAnimation7][n as usize - 1]
}

/// `LoadShootingStarGraphics`.
fn load_shooting_star_graphics(ctx: &mut Ctx, screen: &mut MovieScreen) {
    ctx.screen.effects.obp0 = 0xF9;
    ctx.screen.effects.obp1 = 0xA4;
    let tile = |n: u16| &rom_slice(sym::MoveAnimationTiles1 + n * TILE_BYTES as u16)[..TILE_BYTES];
    ctx.screen.tiles.load(V_CHARS1 + 0x20, tile(3));
    ctx.screen.tiles.load(V_CHARS1 + 0x21, tile(19));
    ctx.screen.tiles.load(V_CHARS1 + 0x22, between(sym::FallingStar, sym::FallingStarEnd));
    for (i, object) in objects(between(sym::GameFreakLogoOAMData, sym::GameFreakLogoOAMDataEnd)).enumerate() {
        screen.oam[24 + i] = object;
    }
    for (i, object) in objects(between(sym::GameFreakShootingStarOAMData, sym::GameFreakShootingStarOAMDataEnd)).enumerate() {
        screen.oam[i] = object;
    }
}

/// `LoadCopyrightTiles`: the copyright's tiles at `vChars2 $60`, then its three lines.
pub fn load_copyright_tiles(ctx: &mut Ctx) {
    ctx.screen.tiles.load(V_CHARS2 + 0x60, between(sym::NintendoCopyrightLogoGraphics, sym::GameFreakLogoGraphicsEnd));
    place_string_lines(&mut ctx.screen.ui, 2, 7, rom_slice(sym::CopyrightTextString));
}

/// `PlaceString` for a string of plain tiles and `<NEXT>`, each line two rows below the last.
pub fn place_string_lines(ui: &mut UiSurface, x: usize, y: usize, bytes: &[u8]) {
    const NEXT: u8 = 0x4E;
    const TERMINATOR: u8 = 0x50;
    let (mut column, mut row) = (x, y);
    for &byte in bytes.iter().take_while(|&&b| b != TERMINATOR) {
        if byte == NEXT {
            (column, row) = (x, row + 2);
        } else {
            for &tile in crate::modes::place_string::ligature(byte).unwrap_or(&[byte]) {
                ui.set(column, row, tile);
                column += 1;
            }
        }
    }
}
