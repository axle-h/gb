//! `DisplayTitleScreen` (`title.asm`, `title2.asm`): the logo bouncing in, the version sliding in
//! under a scroll split, and a new mon scrolled in every 200 frames until A or START. The version
//! is "Rust Version", in the cartridge's own lettering.
//!
//! The screen is two pictures. `vBGMap0` holds the title with the mon, `vBGMap1` the title without,
//! and the window shows `vBGMap1` over `vBGMap0` while the mon on `vBGMap0` is changed; the logo's
//! bounce scrolls the top of `vBGMap0` while the window holds the bottom still. Both are drawn here
//! into [`MovieScreen`] at the moments the cartridge copies `wTileMap` to them.
//!
//! Loading, and not modelled: `ClearScreen`'s and every `TitleScreenCopyTileMapToVRAM`'s `Delay3`,
//! the LCD-off load, the picture's decompression, the `Delay3` before the title music and the one
//! after the white-out.

use poke_core::gfx;
use poke_core::tables::TITLE_MONS;
use poke_core::species::PokemonSpecies;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, AudioBank};
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::tiles::{V_CHARS0, V_CHARS1, V_CHARS2};
use crate::gfx::ui::UiSurface;
use crate::gfx::layers::TileMap;
use crate::mode::Ctx;
use crate::rng::Rng;
use super::intro::{pal_normal, white_out};
use super::screen::{clear_screen, copy_pic_to_tile_map, load_mon_pic, Dest, MovieScreen, WINDOW_HIDDEN};
use super::wait::{Tick, Wait};

/// `vTitleLogo2`: the tiles past `vFrontPic` in `vChars2`.
const TITLE_LOGO2: usize = V_CHARS2 + 49;
/// `SCREEN_HEIGHT_PX`, where the window goes while the version slides in.
const WINDOW_OFF: u8 = 144;
/// `ScrollTitleScreenGameVersion`'s split lines and `TitleScroll`'s.
const VERSION_TOP: usize = 64;
const VERSION_BOTTOM: usize = 80;
const MON_TOP: usize = 0x48;
const MON_BOTTOM: usize = 0x88;
/// `.awaitUserInterruptionLoop`'s wait between mons.
const MON_STANDS: u16 = 200;
/// The hold after the bounce, before the version's whoosh.
const AFTER_BOUNCE: u16 = 36;
/// The Pokéball in Red's hand: `wShadowOAMSprite10`.
const BALL_OBJECT: usize = 10;
const STARTERS: [PokemonSpecies; 3] = [PokemonSpecies::Charmander, PokemonSpecies::Squirtle, PokemonSpecies::Bulbasaur];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Scroll {
    /// `TitleScroll_Out` from `d = 0`, which `TitleScreenScrollInMon` runs despite its name.
    Out,
    /// `TitleScroll_In` from `d = $88`.
    In,
    /// `TitleScroll_WaitBall`, with the ball animating.
    Ball,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Start,
    /// `.bouncePokemonLogoLoop`'s entry and the scrolls left of it.
    Bounce { entry: u8, left: u8 },
    AfterBounce,
    /// `.scrollTitleScreenGameVersionLoop`, at this scroll.
    Version(u8),
    WaitForWhoosh,
    /// `.awaitUserInterruptionLoop`'s 200 frames.
    MonStands,
    /// A `TitleScroll`: the table entry, its frames left and the scroll it is at.
    Scrolling { scroll: Scroll, entry: u8, left: u8, d: u8 },
    /// The `CheckForUserInterruption` of one frame after the scroll out.
    Hidden,
    /// `.finishedWaiting`'s cry.
    Cry,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Title {
    phase: Phase,
    wait: Wait,
    /// `wTitleMonSpecies`.
    species: PokemonSpecies,
    /// `wTileMapBackup2`: the title without a mon.
    without_mon: UiSurface,
    /// `wTileMapBackup`: the title with one.
    with_mon: UiSurface,
    /// `GetTitleBallY`'s `e`.
    ball: u8,
    /// Presses that have ended the wait, for a driver to see its own land.
    answered: u32,
}

impl Default for Title {
    fn default() -> Self {
        Self {
            phase: Phase::Start,
            wait: Wait::default(),
            species: PokemonSpecies::Charmander,
            without_mon: UiSurface::default(),
            with_mon: UiSurface::default(),
            ball: 0,
            answered: 0,
        }
    }
}

impl Title {
    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// Waiting on the player, from the title music on.
    pub fn is_waiting(&self) -> bool {
        matches!(self.phase, Phase::MonStands | Phase::Hidden) && self.wait.is_polling()
    }

    pub fn update(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        match self.wait.tick(ctx) {
            Tick::Waiting => return,
            Tick::Interrupted => self.finish(ctx),
            Tick::Done => {}
        }
        while self.step(ctx, screen) && !self.wait.is_running() {}
    }

    /// One statement; `false` when nothing more runs this frame.
    fn step(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) -> bool {
        match self.phase {
            Phase::Start => self.draw(ctx, screen),
            Phase::Bounce { entry, left } => self.bounce(ctx, screen, entry, left),
            Phase::AfterBounce => {
                ctx.audio.play_sound(sounds::SFX_INTRO_WHOOSH);
                print_version(&mut ctx.screen.ui);
                screen.wy = WINDOW_OFF;
                self.phase = Phase::Version(144);
            }
            Phase::Version(d) => return self.version(ctx, screen, d),
            Phase::WaitForWhoosh if ctx.audio.sound_finished() => {
                ctx.audio.play_new_sound(sounds::MUSIC_TITLE_SCREEN.id);
                self.stand();
            }
            Phase::MonStands => {
                // `TitleScreenScrollInMon`.
                self.phase = Phase::Scrolling { scroll: Scroll::Out, entry: 0, left: 0, d: 0 };
            }
            Phase::Scrolling { .. } => return self.scroll(ctx, screen),
            Phase::Hidden if STARTERS.contains(&self.species) => {
                self.ball = 1;
                self.phase = Phase::Scrolling { scroll: Scroll::Ball, entry: 0, left: 0, d: 0 };
            }
            Phase::Hidden => self.pick_new_mon(ctx, screen),
            Phase::Cry if ctx.audio.sound_finished() => self.leave(ctx, screen),
            Phase::WaitForWhoosh | Phase::Cry | Phase::Done => return false,
        }
        true
    }

    /// `DisplayTitleScreen` to its bounce.
    fn draw(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        ctx.audio.set_bank(AudioBank::Three);
        white_out(ctx);
        screen.transfer.get_or_insert(Dest::BG_MAP1);
        ctx.screen.tiles.animation.kind = 0;
        screen.scx = 0;
        screen.scy = 0x40;
        screen.wy = WINDOW_HIDDEN;
        screen.bg_map = 0;
        clear_screen(&mut ctx.screen.ui);

        let tiles = &mut ctx.screen.tiles;
        tiles.load_font();
        tiles.load(TITLE_LOGO2 + 16, &gfx::splash::COPYRIGHT[..5 * 16]);
        tiles.load(TITLE_LOGO2 + 16 + 5, gfx::title::GAMEFREAK_INC);
        let logo = gfx::title::POKEMON_LOGO;
        tiles.load(V_CHARS1, &logo[..0x60 * 16]);
        tiles.load(TITLE_LOGO2, &logo[0x60 * 16..0x70 * 16]);
        tiles.load_1bpp(V_CHARS2 + 0x60, &rust_version());
        screen.maps = [TileMap::filled(UiSurface::BLANK), TileMap::filled(UiSurface::BLANK)];

        let ui = &mut ctx.screen.ui;
        for row in 0..6 {
            for column in 0..16 {
                ui.set(2 + column, 1 + row, 0x80 + (row * 16 + column) as u8);
            }
        }
        for column in 0..16 {
            ui.set(2 + column, 7, 0x31 + column as u8);
        }
        // `DrawPlayerCharacter`, with a Pokéball put in Red's hand.
        tiles.load(V_CHARS0, gfx::title::PLAYER);
        screen.clear_sprites();
        for row in 0..7u8 {
            for column in 0..5u8 {
                let object = &mut screen.oam[(row * 5 + column) as usize];
                (object.y, object.x, object.tile) = (0x60 + 8 * row, 0x5A + 8 * column, row * 5 + column);
            }
        }
        screen.oam[BALL_OBJECT].y = 0x74;
        ui.place(2, 17, poke_core::tables::TITLE_SCREEN_COPYRIGHT_TILES);
        self.without_mon = ui.clone();

        self.species = PokemonSpecies::Charmander;
        load_title_mon(ctx, self.species);
        screen.copy_to(&ctx.screen.ui, Dest { map: 0, offset: 0x300 });
        self.with_mon = ctx.screen.ui.clone();
        screen.wy = 0x40;
        ctx.screen.ui = self.without_mon.clone();
        screen.copy_to(&ctx.screen.ui, Dest::BG_MAP0);
        ctx.screen.sgb.run(&PaletteCommand::TitleScreen);
        pal_normal(ctx);
        ctx.screen.effects.obp0 = 0b1110_0100;
        self.start_bounce(ctx, 0);
    }

    fn scrolls(entry: u8) -> (i8, u8) {
        poke_core::tables::TITLE_SCREEN_POKEMON_LOGO_Y_SCROLLS.get(entry as usize).copied().unwrap_or((0, 0))
    }

    /// An entry of `.TitleScreenPokemonLogoYScrolls`: the crash at the bounce's first rebound, then
    /// the first of its scrolls a frame on.
    fn start_bounce(&mut self, ctx: &mut Ctx, entry: u8) {
        let (d, times) = Self::scrolls(entry);
        if d == 0 {
            self.phase = Phase::AfterBounce;
            ctx.screen.ui = self.with_mon.clone();
            self.wait = Wait::frames(AFTER_BOUNCE);
            return;
        }
        if d == -3 {
            ctx.audio.play_sound(sounds::SFX_INTRO_CRASH);
        }
        self.phase = Phase::Bounce { entry, left: times };
        self.wait = Wait::frames(1);
    }

    fn bounce(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen, entry: u8, left: u8) {
        let (d, _) = Self::scrolls(entry);
        screen.scy = screen.scy.wrapping_add(d as u8);
        if left > 1 {
            self.phase = Phase::Bounce { entry, left: left - 1 };
            self.wait = Wait::frames(1);
        } else {
            self.start_bounce(ctx, entry + 1);
        }
    }

    /// A frame of `.scrollTitleScreenGameVersionLoop`: the version's two rows scrolled by `d`.
    fn version(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen, d: u8) -> bool {
        split(ctx, screen.scx, VERSION_TOP, VERSION_BOTTOM, d);
        let d = d.wrapping_add(4);
        if d != 0 {
            self.phase = Phase::Version(d);
            return false;
        }
        screen.copy_to(&ctx.screen.ui, Dest::BG_MAP1);
        ctx.screen.ui = self.without_mon.clone();
        print_version(&mut ctx.screen.ui);
        self.phase = Phase::WaitForWhoosh;
        true
    }

    fn stand(&mut self) {
        self.phase = Phase::MonStands;
        self.wait = Wait::check(MON_STANDS);
    }

    /// A frame of `_TitleScroll`: the mon's rows split off at `d`, and the ball moved if it is
    /// animating. Going from one entry of the table to the next costs no frame.
    fn scroll(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) -> bool {
        let Phase::Scrolling { scroll, mut entry, mut left, mut d } = self.phase else { unreachable!() };
        // A step's high nybble is the speed and its low one the frames it lasts.
        let table = match scroll {
            Scroll::Out => poke_core::tables::TITLE_SCROLL_OUT,
            Scroll::In => poke_core::tables::TITLE_SCROLL_IN,
            Scroll::Ball => poke_core::tables::TITLE_SCROLL_WAIT_BALL,
        };
        if left == 0 {
            let Some(&step) = table.get(entry as usize) else {
                self.scrolled(ctx, screen, scroll);
                return true;
            };
            left = step & 0xF;
            entry += 1;
        }
        split(ctx, screen.scx, MON_TOP, MON_BOTTOM, d);
        d = d.wrapping_add(table[entry as usize - 1] >> 4);
        // `GetTitleBallY`, which stops at the table's zero without moving on.
        let y = poke_core::tables::TITLE_BALL_Y_TABLE[self.ball as usize];
        if y != 0 {
            screen.oam[BALL_OBJECT].y = y;
            self.ball += 1;
        }
        left -= 1;
        self.phase = Phase::Scrolling { scroll, entry, left, d };
        if left == 0 && entry as usize == table.len() {
            self.scrolled(ctx, screen, scroll);
        }
        false
    }

    /// What follows each `TitleScroll`, in the frame its last split was set.
    fn scrolled(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen, scroll: Scroll) {
        match scroll {
            Scroll::Out => {
                screen.wy = 0;
                self.phase = Phase::Hidden;
                self.wait = Wait::check(1);
            }
            Scroll::Ball => {
                self.ball = 0;
                self.pick_new_mon(ctx, screen);
            }
            Scroll::In => self.stand(),
        }
    }

    /// `TitleScreenPickNewMon`, up to the scroll in, whose first split falls in the next frame the
    /// cartridge reaches its line in.
    fn pick_new_mon(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        screen.copy_to(&ctx.screen.ui, Dest::BG_MAP0);
        let species = loop {
            let pick = PokemonSpecies::from_repr(TITLE_MONS[(ctx.rng.random() & 0xF) as usize]).expect("a title mon");
            if pick != self.species {
                break pick;
            }
        };
        self.species = species;
        load_title_mon(ctx, species);
        screen.wy = WINDOW_HIDDEN;
        self.phase = Phase::Scrolling { scroll: Scroll::In, entry: 0, left: 0, d: 0x88 };
    }

    /// `.finishedWaiting`'s cry, from the press that ended the wait.
    fn finish(&mut self, ctx: &mut Ctx) {
        self.answered += 1;
        ctx.audio.play_cry(self.species as u8);
        self.phase = Phase::Cry;
    }

    /// `.finishedWaiting` after the cry: both maps blank and the window over everything.
    fn leave(&mut self, ctx: &mut Ctx, screen: &mut MovieScreen) {
        white_out(ctx);
        screen.clear_sprites();
        screen.wy = 0;
        clear_screen(&mut ctx.screen.ui);
        screen.copy_to(&ctx.screen.ui, Dest::BG_MAP0);
        screen.copy_to(&ctx.screen.ui, Dest::BG_MAP1);
        // `LoadGBPal`, with no dark map to offset it.
        (ctx.screen.effects.bgp, ctx.screen.effects.obp0, ctx.screen.effects.obp1) = poke_core::tables::FADE_PALETTES[3];
        self.phase = Phase::Done;
    }
}

/// `rSCX` set at line `top` and back to 0 at `bottom`; the lines above keep the latched scroll.
fn split(ctx: &mut Ctx, scx: u8, top: usize, bottom: usize, d: u8) {
    let lines = (0..144).map(|line| if line < top { scx } else if line < bottom { d } else { 0 }).collect();
    ctx.screen.effects.line_scx = Some(lines);
}

/// `LoadTitleMonSprite`.
fn load_title_mon(ctx: &mut Ctx, species: PokemonSpecies) {
    load_mon_pic(ctx, species, false);
    copy_pic_to_tile_map(&mut ctx.screen.ui, 5, 10, 0, false);
}

/// The tile row `PrintGameVersionOnTitleScreen` writes the version on.
pub const TITLE_VERSION_ROW: usize = 8;

/// `PrintGameVersionOnTitleScreen`, a column left of Red's for the third tile "Rust" takes.
fn print_version(ui: &mut UiSurface) {
    ui.place(6, TITLE_VERSION_ROW, &[0x60, 0x61, 0x62, 0x7F, 0x65, 0x66, 0x67, 0x68, 0x69]);
}

/// `Version_GFX` reading "Rust Version": Red's strip with "Red" redrawn as "Rust". The
/// letters are the cartridge's, a column apart as Red's are: R and s from Red's strip, u from
/// Blue's, t from "Nintendo" in the copyright line, whose letters sit two rows higher. "Rust" ends
/// on the column "Red" does, so "Version" keeps its place.
fn rust_version() -> [u8; 80] {
    use gfx::{splash::COPYRIGHT, title::{BLUE_VERSION, RED_VERSION}};
    let copyright_dark_plane: Vec<u8> = COPYRIGHT.iter().skip(1).step_by(2).copied().collect();
    // Each letter's source strip, its columns there, and how far it moves down.
    let letters: [(&[u8], std::ops::Range<usize>, usize); 4] = [
        (RED_VERSION, 0..5, 0),
        (BLUE_VERSION, 9..13, 0),
        (RED_VERSION, 57..62, 0),
        (&copyright_dark_plane, 55..59, 2),
    ];
    let mut strip = *RED_VERSION;
    strip[..5 * 8].fill(0);
    let mut to = 3;
    for (from, columns, down) in letters {
        for x in columns {
            for y in down..8 {
                let lit = from[x / 8 * 8 + y - down] >> (7 - x % 8) & 1;
                strip[to / 8 * 8 + y] |= lit << (7 - to % 8);
            }
            to += 1;
        }
        to += 1;
    }
    strip
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_reads_rust_version() {
        let strip = rust_version();
        let row = |y: usize, columns: std::ops::Range<usize>| -> String {
            columns.map(|x| if strip[x / 8 * 8 + y] >> (7 - x % 8) & 1 != 0 { '#' } else { '.' }).collect()
        };
        let rust: Vec<String> = (0..8).map(|y| row(y, 0..40)).collect();
        assert_eq!(rust, [
            "........................................",
            "........................................",
            "........................................",
            "...####..............##.................",
            "...##.##.##.#..####.####................",
            "...####..##.#.###....##.................",
            "...##.##.##.#...###..##.................",
            "...##.##..###.####...##.................",
        ]);
        assert_eq!(strip[5 * 8..], gfx::title::RED_VERSION[5 * 8..], "Red's \"Version\"");
    }
}
