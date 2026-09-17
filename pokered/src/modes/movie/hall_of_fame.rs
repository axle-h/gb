//! `HallOfFamePC`: `AnimateHallOfFame` (`hall_of_fame.asm`), each party mon and then the player
//! scrolled on and described, the team recorded; then the credits (`credits.asm`).
//!
//! The team goes into `World::hall_of_fame` where `SaveHallOfFameTeams` would write it to SRAM, and
//! `wNumHoFTeams` into `World::hall_of_fame_teams`, before the player is shown.
//!
//! The Hall of Fame's texts print a letter a frame: `AnimateHallOfFame` clears the letter delay
//! flags, and the League's script puts them back after. Here the movie does both.
//!
//! Loading, and not modelled: `ClearScreen`'s `Delay3`, the LCD-off loads and the pictures'
//! decompression, and each `CreditsCopyTileMapToVRAM`'s `Delay3`.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_events::EVENT_HALL_OF_FAME_DEX_RATING, pokered_symbols as sym, DmgPointer};
use poke_core::text_script::{decode, TextNumber};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::gfx::layers::TileMap;
use crate::gfx::sgb::{determine_palette_id_out_of_battle, PaletteCommand};
use crate::gfx::tiles::{V_CHARS1, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::mode::{Ctx, Mode, Transition};
use crate::modes::battle::hud::{load_back_pic, load_player_back_pic};
use crate::modes::text_box::TextBox;
use crate::systems::hall_of_fame::{HallOfFameMon, HOF_TEAM_CAPACITY};
use crate::systems::pokedex::count_set_bits;
use crate::systems::print_num::{print_bcd, print_number, BcdFormat, NumberFormat};
use crate::systems::status_screen::print_mon_type;
use super::intro::{fade_out_to_white, load_copyright_tiles, place_string_lines, FADE_STEP, FADE_STEPS};
use super::screen::{between, clear_screen, copy_pic_to_tile_map, copy_tile_ids, load_mon_pic, load_pic, Dest, MovieScreen};
use super::wait::{Tick, Wait};

/// `TILEMAP_MON_PIC`.
const TILEMAP_MON_PIC: usize = 0;
/// Where `vBackPic` starts in `vChars2`.
const BACK_PIC_TILE: u8 = 0x31;
/// `CreditsOrder`'s commands, counting down from `$FF` past the strings.
const CRED_TEXT_FADE_MON: u8 = 0xFF;
const CRED_TEXT_MON: u8 = 0xFE;
const CRED_TEXT_FADE: u8 = 0xFD;
const CRED_TEXT: u8 = 0xFC;
const CRED_COPYRIGHT: u8 = 0xFB;
const CRED_THE_END: u8 = 0xFA;
/// `FillFourRowsWithBlack`'s tile, made solid.
const SOLID_BLACK: u8 = 0x7E;
/// `%11000000`: the credits' palette, in which only solid black shows.
const CREDITS_BGP: u8 = 0b1100_0000;
/// `%11111100`: a credits mon as a silhouette.
const SILHOUETTE_BGP: u8 = 0b1111_1100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    FadeOut(u8),
    AfterClear,
    /// A mon, or the player at the party's length: the back pic scrolling, then the front.
    ScrollBack(u8),
    ScrollFront(u8),
    Cry(u8),
    Described(u8),
    Named(u8),
    /// The fade after a mon; the mon and the palette.
    Faded { mon: u8, step: u8 },
    /// `HoFPrintTextAndDelay`'s texts: which one is up or held.
    PlayerText { text: u8, held: bool },
    LastFade(u8),
    /// `HallOfFamePC` after the Hall of Fame: the 100-frame hold, the credits' own screen, and
    /// its 128.
    CreditsHold,
    CreditsStart,
    CreditsMusic,
    /// A credits screen: the order's byte read next, and a hold or fade under way.
    Credits { order: usize },
    Fading { order: usize, step: u8, then: After },
    Holding { order: usize, then: After },
    /// `DisplayCreditsMon`'s scroll, a tile a frame.
    MonScroll { order: usize, step: u8 },
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    /// Held this long, then the mon or the next screen.
    Hold(u16, bool),
    Mon,
    Next,
    TheEnd,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HallOfFame {
    phase: Phase,
    wait: Wait,
    screen: MovieScreen,
    /// A text is up.
    child: bool,
    team: Vec<HallOfFameMon>,
    /// `wNumCreditsMonsDisplayed`.
    credits_mons: u8,
    /// The screen is the movie's: until its first mon, what was there fades out as it stands.
    #[serde(default)]
    drawn: bool,
}

impl Default for HallOfFame {
    fn default() -> Self {
        Self { phase: Phase::FadeOut(0), wait: Wait::default(), screen: MovieScreen::default(), child: false, team: Vec::new(), credits_mons: 0, drawn: false }
    }
}

impl HallOfFame {
    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    pub fn update(&mut self, ctx: &mut Ctx) -> Transition {
        if self.child {
            return Transition::Stay;
        }
        if self.drawn {
            self.screen.vblank(ctx);
        }
        let transition = self.run(ctx);
        if self.drawn && !self.child {
            self.screen.present(ctx);
        }
        transition
    }

    pub fn resume(&mut self, ctx: &mut Ctx) -> Transition {
        self.child = false;
        if let Phase::PlayerText { text, .. } = self.phase {
            self.phase = Phase::PlayerText { text, held: true };
            self.wait = Wait::frames(120);
        }
        self.screen.present(ctx);
        Transition::Stay
    }

    fn run(&mut self, ctx: &mut Ctx) -> Transition {
        if self.wait.tick(ctx) == Tick::Waiting {
            return Transition::Stay;
        }
        loop {
            if let Some(transition) = self.step(ctx) {
                return transition;
            }
            if self.wait.is_running() || self.phase == Phase::Done {
                return Transition::Stay;
            }
        }
    }

    fn step(&mut self, ctx: &mut Ctx) -> Option<Transition> {
        match self.phase {
            Phase::FadeOut(0) => {
                // `HoFFadeOutScreenAndMusic`.
                ctx.audio.fade_out_to_silence(10);
                self.fade(ctx, 0);
            }
            Phase::FadeOut(step) if step < FADE_STEPS => self.fade(ctx, step),
            Phase::FadeOut(_) => {
                clear_screen(&mut ctx.screen.ui);
                self.phase = Phase::AfterClear;
                self.wait = Wait::frames(100);
            }
            Phase::AfterClear => {
                self.set_up(ctx);
                self.show(ctx, 0);
            }
            Phase::ScrollBack(mon) => {
                self.screen.scx = self.screen.scx.wrapping_add(4);
                if self.screen.scx != 0xA0 {
                    self.wait = Wait::frames(1);
                } else {
                    self.screen.scy = 0;
                    copy_tile_ids(&mut ctx.screen.ui, 12, 5, TILEMAP_MON_PIC, 0);
                    self.phase = Phase::ScrollFront(mon);
                    self.wait = Wait::frames(1);
                }
            }
            Phase::ScrollFront(mon) => {
                self.screen.scx = self.screen.scx.wrapping_sub(4);
                if self.screen.scx != 0 {
                    self.wait = Wait::frames(1);
                } else if (mon as usize) < ctx.world.party.len() {
                    self.describe(ctx, mon);
                } else {
                    return self.player_stats(ctx);
                }
            }
            Phase::Cry(mon) if ctx.audio.sound_finished() => {
                self.phase = Phase::Described(mon);
                self.wait = Wait::frames(80);
            }
            Phase::Cry(_) => self.wait = Wait::frames(1),
            Phase::Described(mon) => {
                let ui = &mut ctx.screen.ui;
                ui.text_box_border(2, 13, 14, 3);
                ui.place(4, 15, &poke_core::charmap::encode("HALL OF FAME").expect("encodes"));
                self.phase = Phase::Named(mon);
                self.wait = Wait::frames(180);
            }
            Phase::Named(mon) => {
                self.phase = Phase::Faded { mon, step: 0 };
            }
            Phase::Faded { mon, step } if step < FADE_STEPS => {
                fade_out_to_white(ctx, step);
                self.phase = Phase::Faded { mon, step: step + 1 };
                self.wait = Wait::frames(FADE_STEP);
            }
            Phase::Faded { mon, .. } => {
                if mon as usize + 1 < ctx.world.party.len() {
                    self.show(ctx, mon + 1);
                } else {
                    self.save_team(ctx);
                    self.show(ctx, ctx.world.party.len() as u8);
                }
            }
            Phase::PlayerText { held: false, .. } => {}
            Phase::PlayerText { text, held: true } => return self.player_text(ctx, text + 1),
            Phase::LastFade(0) => {
                ctx.audio.fade_out_to_silence(10);
                self.last_fade(ctx, 0);
            }
            Phase::LastFade(step) if step < FADE_STEPS => self.last_fade(ctx, step),
            Phase::LastFade(_) => {
                self.screen.wy = 0;
                self.screen.bg_map = 0;
                // `HallOfFamePC`.
                clear_screen(&mut ctx.screen.ui);
                self.phase = Phase::CreditsHold;
                self.wait = Wait::frames(100);
            }
            Phase::CreditsHold => {
                self.credits_screen(ctx);
                self.phase = Phase::CreditsStart;
            }
            Phase::CreditsStart if ctx.audio.sound_finished() => {
                ctx.audio.play_sound(SoundId::STOP_ALL_MUSIC);
                ctx.audio.play_music(sounds::MUSIC_CREDITS);
                self.phase = Phase::CreditsMusic;
                self.wait = Wait::frames(128);
            }
            Phase::CreditsStart => self.wait = Wait::frames(1),
            Phase::CreditsMusic => {
                self.credits_mons = 0;
                self.phase = Phase::Credits { order: 0 };
            }
            Phase::Credits { order } => self.credits(ctx, order),
            Phase::Fading { order, step, then } => {
                if step < 4 {
                    ctx.screen.effects.bgp = rom_slice(sym::HoFGBPalettes)[step as usize];
                    self.phase = Phase::Fading { order, step: step + 1, then };
                    self.wait = Wait::frames(5);
                } else {
                    self.after(ctx, order, then);
                }
            }
            Phase::Holding { order, then } => self.after(ctx, order, then),
            Phase::MonScroll { order, step } => self.mon_scroll(ctx, order, step),
            Phase::Done => {}
        }
        None
    }

    fn fade(&mut self, ctx: &mut Ctx, step: u8) {
        fade_out_to_white(ctx, step);
        self.phase = Phase::FadeOut(step + 1);
        self.wait = Wait::frames(FADE_STEP);
    }

    fn last_fade(&mut self, ctx: &mut Ctx, step: u8) {
        fade_out_to_white(ctx, step);
        self.phase = Phase::LastFade(step + 1);
        self.wait = Wait::frames(FADE_STEP);
    }

    /// `AnimateHallOfFame` from its fonts to the first mon.
    fn set_up(&mut self, ctx: &mut Ctx) {
        self.drawn = true;
        ctx.screen.tiles.load_font();
        ctx.screen.tiles.load_text_box_tiles();
        ctx.screen.map = Default::default();
        self.screen.maps = [TileMap::filled(UiSurface::BLANK), TileMap::filled(UiSurface::BLANK)];
        self.screen.bg_map = 1;
        self.screen.transfer = Some(Dest::BG_MAP1);
        self.screen.clear_sprites();
        ctx.screen.tiles.animation.kind = 0;
        ctx.world.one_frame_letter_delay = true;
        ctx.world.hall_of_fame_teams = ctx.world.hall_of_fame_teams.saturating_add(1);
        self.screen.wy = 0x90;
        ctx.audio.play_music(sounds::MUSIC_HALL_OF_FAME);
        self.team.clear();
    }

    /// `HoFShowMonOrPlayer` to its first scroll: the party mon in `mon`, or past the party the player.
    fn show(&mut self, ctx: &mut Ctx, mon: u8) {
        clear_screen(&mut ctx.screen.ui);
        self.screen.scy = 0xD0;
        self.screen.scx = 0xC0;
        let species = ctx.world.party.get(mon as usize).map(|named| named.mon.mon.species);
        match species {
            Some(species) => {
                load_mon_pic(ctx, species, false);
                copy_pic_to_tile_map(&mut ctx.screen.ui, 12, 5, 0, false);
                ctx.screen.ui.fill(1, 5, 8, 7, UiSurface::BLANK);
                load_back_pic(&mut ctx.screen.tiles, species);
            }
            None => {
                load_pic(ctx, sym::RedPicFront);
                load_player_back_pic(&mut ctx.screen.tiles, false);
            }
        }
        let palette = species.map_or(0, |species| determine_palette_id_out_of_battle(species as u8));
        ctx.screen.sgb.run(&PaletteCommand::PokemonWholeScreen { mon: palette, black: false });
        ctx.screen.effects.bgp = super::screen::BGP_NORMAL;
        copy_tile_ids(&mut ctx.screen.ui, 12, 5, TILEMAP_MON_PIC, BACK_PIC_TILE);
        self.phase = Phase::ScrollBack(mon);
        self.wait = Wait::frames(1);
    }

    /// `HoFDisplayAndRecordMonInfo`, to its cry.
    fn describe(&mut self, ctx: &mut Ctx, mon: u8) {
        let named = ctx.world.party[mon as usize].clone();
        let species = named.mon.mon.species;
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(0, 2, 10, 9);
        place_string_lines(ui, 2, 6, rom_slice(sym::HoFMonInfoText));
        ui.place(1, 4, &named.nick);
        print_number(ui, 7 * SCREEN_TILES_X + 8, named.mon.level as u32, NumberFormat { digits: 3, leading_zeroes: false, left_align: true });
        print_mon_type(ui, 9 * SCREEN_TILES_X + 3, species);
        ctx.audio.play_cry(species as u8);
        self.team.push(HallOfFameMon { species, level: named.mon.level, nick: named.nick });
        self.phase = Phase::Cry(mon);
    }

    /// `SaveHallOfFameTeams`, oldest dropped past the record's capacity.
    fn save_team(&mut self, ctx: &mut Ctx) {
        let teams = &mut ctx.world.hall_of_fame;
        if teams.len() >= HOF_TEAM_CAPACITY {
            teams.remove(0);
        }
        teams.push(std::mem::take(&mut self.team));
    }

    /// `HoFDisplayPlayerStats` to its first text.
    fn player_stats(&mut self, ctx: &mut Ctx) -> Option<Transition> {
        ctx.world.events.set(EVENT_HALL_OF_FAME_DEX_RATING);
        let seen = count_set_bits(&ctx.world.pokedex.seen) as u32;
        let owned = count_set_bits(&ctx.world.pokedex.owned) as u32;
        ctx.world.text.numbers.insert(TextNumber::DexRatingNumMonsSeen, seen);
        ctx.world.text.numbers.insert(TextNumber::DexRatingNumMonsOwned, owned);
        ctx.world.events.clear(EVENT_HALL_OF_FAME_DEX_RATING);
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(0, 4, 10, 6);
        ui.text_box_border(5, 0, 9, 2);
        let name = ctx.world.player_name.clone();
        ui.place(7, 2, &name);
        ui.place(1, 6, &poke_core::charmap::encode("PLAY TIME").expect("encodes"));
        let time = ctx.world.play_time;
        let plain = NumberFormat { digits: 3, leading_zeroes: false, left_align: false };
        let end = print_number(ui, 7 * SCREEN_TILES_X + 5, time.hours as u32, plain);
        ui.set(end % SCREEN_TILES_X, end / SCREEN_TILES_X, 0x6D);
        print_number(ui, end + 1, time.minutes as u32, NumberFormat { digits: 2, leading_zeroes: true, left_align: false });
        ui.place(1, 9, &poke_core::charmap::encode("MONEY").expect("encodes"));
        print_bcd(ui, 10 * SCREEN_TILES_X + 4, &ctx.world.money, BcdFormat { skip_leading_zeroes: true, left_align: false, money_sign: true });
        self.player_text(ctx, 0)
    }

    /// `HoFPrintTextAndDelay` for the seen and owned counts, the rating's label and the rating.
    fn player_text(&mut self, ctx: &mut Ctx, text: u8) -> Option<Transition> {
        let script = match text {
            0 => sym::DexSeenOwnedText,
            1 => sym::DexRatingText,
            2 => rating_text(count_set_bits(&ctx.world.pokedex.owned)),
            _ => {
                self.phase = Phase::LastFade(0);
                return None;
            }
        };
        self.phase = Phase::PlayerText { text, held: false };
        self.child = true;
        MovieScreen::release(ctx);
        Some(Transition::Push(Mode::TextBox(TextBox::script(decode(script).expect("the rating texts decode")))))
    }

    /// `HallOfFamePC`'s screen for the credits: the font shifted a shade so the text can fade in,
    /// a solid tile for the bars, and the bars.
    fn credits_screen(&mut self, ctx: &mut Ctx) {
        for tile in (V_CHARS1..V_CHARS1 + 0x80).chain(V_CHARS2 + 0x60..V_CHARS2 + 0x80) {
            let mut bytes = *ctx.screen.tiles.bg(tile_id(tile));
            for byte in bytes.iter_mut().step_by(2) {
                *byte = 0;
            }
            ctx.screen.tiles.load(tile, &bytes);
        }
        ctx.screen.tiles.load(V_CHARS2 + SOLID_BLACK as usize, &[0xFF; TILE_BYTES]);
        let ui = &mut ctx.screen.ui;
        ui.fill(0, 0, SCREEN_TILES_X, 4, SOLID_BLACK);
        ui.fill(0, 14, SCREEN_TILES_X, 4, SOLID_BLACK);
        ctx.screen.effects.bgp = CREDITS_BGP;
        self.screen.transfer = Some(Dest::BG_MAP1);
    }

    /// `Credits`' `.nextCreditsScreen` and the commands after it, to the next hold.
    fn credits(&mut self, ctx: &mut Ctx, mut order: usize) {
        let orders = rom_slice(sym::CreditsOrder);
        fill_middle_with_white(&mut ctx.screen.ui);
        let mut at = 6 * SCREEN_TILES_X + 9;
        loop {
            let command = orders[order];
            order += 1;
            match command {
                CRED_TEXT_FADE_MON => return self.phase = Phase::Fading { order, step: 0, then: After::Hold(90, true) },
                CRED_TEXT_MON => return self.hold(order, 110, After::Mon),
                CRED_TEXT_FADE => return self.phase = Phase::Fading { order, step: 0, then: After::Hold(120, false) },
                CRED_TEXT => return self.hold(order, 140, After::Next),
                CRED_COPYRIGHT => load_copyright_tiles(ctx),
                CRED_THE_END => return self.hold(order, 16, After::TheEnd),
                string => {
                    let pointers = rom_slice(sym::CreditsTextPointers);
                    let address = u16::from_le_bytes([pointers[string as usize * 2], pointers[string as usize * 2 + 1]]);
                    let text = rom_slice(DmgPointer { address, ..sym::CreditsTextPointers });
                    let from = (at as isize + text[0] as i8 as isize) as usize;
                    place_string_lines(&mut ctx.screen.ui, from % SCREEN_TILES_X, from / SCREEN_TILES_X, &text[1..]);
                    at += 2 * SCREEN_TILES_X;
                }
            }
        }
    }

    fn hold(&mut self, order: usize, frames: u16, then: After) {
        self.phase = Phase::Holding { order, then };
        self.wait = Wait::frames(frames);
    }

    fn after(&mut self, ctx: &mut Ctx, order: usize, then: After) {
        match then {
            After::Hold(frames, mon) => self.hold(order, frames, if mon { After::Mon } else { After::Next }),
            After::Mon => self.display_credits_mon(ctx, order),
            After::Next => self.phase = Phase::Credits { order },
            After::TheEnd => self.the_end(ctx),
            After::Finished => {
                ctx.world.one_frame_letter_delay = false;
                self.phase = Phase::Done;
            }
        }
    }

    /// `DisplayCreditsMon` to its scroll: the mon drawn into the columns past the screen's edge of
    /// `vBGMap0`, the text over the rest, and the window holding the text still.
    fn display_credits_mon(&mut self, ctx: &mut Ctx, order: usize) {
        self.screen.transfer = None;
        let text = ctx.screen.ui.clone();
        fill_middle_with_white(&mut ctx.screen.ui);
        let species = PokemonSpecies::from_repr(rom_slice(sym::CreditsMons)[self.credits_mons as usize]).expect("a credits mon");
        self.credits_mons += 1;
        load_mon_pic(ctx, species, false);
        copy_pic_to_tile_map(&mut ctx.screen.ui, 8, 6, 0, false);
        self.screen.copy_to(&ctx.screen.ui, Dest { map: 0, offset: 0x0C });
        self.screen.transfer = None;
        ctx.screen.ui = text;
        self.screen.copy_to(&ctx.screen.ui, Dest::BG_MAP0);
        self.screen.wx = 0xA7;
        self.screen.copy_to(&ctx.screen.ui, Dest::BG_MAP1);
        fill_middle_with_white(&mut ctx.screen.ui);
        ctx.screen.effects.bgp = SILHOUETTE_BGP;
        self.mon_scroll(ctx, order, 0);
    }

    /// A frame of `ScrollCreditsMonLeft`: the middle rows a tile further left, and from the eighth
    /// the window a tile left too, which the cartridge moves after the rows it covers are drawn.
    fn mon_scroll(&mut self, ctx: &mut Ctx, order: usize, step: u8) {
        const SCROLLS: u8 = 7 + 20;
        let d = step.wrapping_mul(8);
        let lines = (0..144).map(|line| if (0x20..0x70).contains(&line) { d } else { 0 }).collect();
        ctx.screen.effects.line_scx = Some(lines);
        if step >= 7 {
            self.screen.wx_next = Some(self.screen.wx.wrapping_sub(8));
        }
        if step + 1 < SCROLLS {
            self.phase = Phase::MonScroll { order, step: step + 1 };
            self.wait = Wait::frames(1);
        } else {
            self.screen.wy = 0;
            ctx.screen.effects.bgp = CREDITS_BGP;
            self.phase = Phase::Credits { order };
        }
    }

    /// `.showTheEnd`, then `FadeInCredits`, and the credits are over.
    fn the_end(&mut self, ctx: &mut Ctx) {
        fill_middle_with_white(&mut ctx.screen.ui);
        ctx.screen.tiles.load(V_CHARS2 + 0x60, between(sym::TheEndGfx, sym::TheEndGfxEnd));
        let text = rom_slice(sym::TheEndTextString);
        let split = text.iter().position(|&b| b == 0x50).expect("two strings");
        place_string_lines(&mut ctx.screen.ui, 4, 8, &text[..split]);
        place_string_lines(&mut ctx.screen.ui, 4, 9, &text[split + 1..]);
        self.phase = Phase::Fading { order: 0, step: 0, then: After::Finished };
    }
}

fn tile_id(tile: usize) -> u8 {
    if tile >= V_CHARS2 { (tile - V_CHARS2) as u8 } else { (tile - V_CHARS1) as u8 + 0x80 }
}

/// `FillMiddleOfScreenWithWhite`.
fn fill_middle_with_white(ui: &mut UiSurface) {
    ui.fill(0, 4, SCREEN_TILES_X, 10, UiSurface::BLANK);
}

/// `DexRatingsTable`: the text for how many are owned.
fn rating_text(owned: u8) -> DmgPointer {
    let table = rom_slice(sym::DexRatingsTable);
    let row = table.chunks_exact(3).find(|row| owned < row[0]).expect("the table ends past 151");
    DmgPointer { address: u16::from_le_bytes([row[1], row[2]]), ..sym::DexRatingsTable }
}
