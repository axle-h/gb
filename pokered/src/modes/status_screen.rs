//! `StatusScreen` and `StatusScreen2`: the two pages a mon's summary is read in, one mode because
//! every caller runs the second straight after the first.
//!
//! Page one is the picture, the nickname, the dex number, the level, the HP bar with its fraction
//! under it, the status or `OK`, the species' types, the ID number and OT, and the four stats. Page
//! two keeps the picture and the dex number and replaces the rest with the moves and their PP, the
//! experience and the experience to the next level, and the *species* name where the nickname was.
//!
//! The one wait before a page takes a press is the cry, which `PlayCry` waits out; the `Delay3`s of
//! `GBPalWhiteOutWithDelay3`, `ClearScreen` and both pages are loading and not modelled.
//! `WaitForTextScrollButtonPress` seeds its blink counter with zero, so no `▼` is ever drawn here.
//!
//! Not modelled: the palette writes, which no mode makes yet, and `UpdateSprites`.

use poke_core::base_stats::BaseStats;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_symbols;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::sgb::{determine_palette_id_out_of_battle, PaletteCommand};
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::party::{BoxMon, Named, PartyMon};
use crate::systems::hp_bar::{draw_hp, HpBarType};
use crate::systems::learn_move::format_moves_string;
use crate::systems::pokedex::front_pic_tiles;
use crate::systems::pp::{max_pp, pp_left};
use crate::systems::print_num::{print_number, NumberFormat};
use crate::systems::stats::calc_stats;
use crate::systems::status_screen::{calc_exp_to_level_up, draw_line_box, encode, place_lines, print_level,
                                    print_mon_type, print_stats_box, print_status_condition, StatsBox, MAX_LEVEL};

/// `<BOLD_P>`, `<to>` and `│`, which the tiles this screen loads draw.
const BOLD_P: u8 = 0x72;
const TO: u8 = 0x70;
const VERTICAL: u8 = 0x78;
const DASH: u8 = 0xE3;
const TILE_BYTES_1BPP: usize = 8;
/// `rAUDVOL` while the screen is up, and after.
const QUIET: u8 = 0x33;
const LOUD: u8 = 0x77;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusScreen {
    /// `wLoadedMon` and the names `NamePointers2` and `OTPointers` find.
    mon: Named<PartyMon>,
    phase: Phase,
    /// `hTileAnimations`, pushed on the way in.
    tile_animations: u8,
    answered: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// `PlayCry`'s `WaitForSoundToFinish`.
    Cry,
    FirstPage,
    SecondPage,
}

impl StatusScreen {
    /// A party mon, which is shown with the stats it carries.
    pub fn new(mon: Named<PartyMon>) -> Self {
        Self { mon, phase: Phase::Cry, tile_animations: 0, answered: 0 }
    }

    /// A mon in a box or the day care: its level is the one it was put away at, and its stats are
    /// worked out again from that with its stat experience.
    pub fn from_box(named: Named<BoxMon>) -> Self {
        let level = named.mon.box_level;
        let stats = calc_stats(named.mon.base_stats().stats, named.mon.dvs, Some(named.mon.stat_exp), level);
        let mon = PartyMon { mon: named.mon, level, stats };
        Self::new(Named { mon, ot: named.ot, nick: named.nick })
    }

    /// Pages turned so far, so a driver can see its press land.
    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// `StatusScreen` from `ClearScreen` to the picture.
    fn draw_first_page(&self, ctx: &mut Ctx) {
        let tiles = &mut ctx.screen.tiles;
        tiles.load_hp_bar_and_status_tiles();
        let one_bpp = |start: poke_core::symbols::DmgPointer, count: usize| &rom_slice(start)[..count * TILE_BYTES_1BPP];
        tiles.load_1bpp(V_CHARS2 + 0x6D, one_bpp(pokered_symbols::BattleHudTiles1, 3));
        tiles.load_1bpp(V_CHARS2 + 0x78, one_bpp(pokered_symbols::BattleHudTiles2, 1));
        tiles.load_1bpp(V_CHARS2 + 0x76, one_bpp(pokered_symbols::BattleHudTiles3, 2));
        tiles.load_1bpp(V_CHARS2 + BOLD_P as usize, one_bpp(pokered_symbols::PTile, 1));

        let PartyMon { mon, level, stats } = &self.mon.mon;
        let ui = &mut ctx.screen.ui;
        let at = |x: usize, y: usize| y * SCREEN_TILES_X + x;
        draw_line_box(ui, at(19, 1), 6, 10);
        ui.set(2, 7, encode("<DOT>")[0]);
        ui.set(1, 7, encode("№")[0]);
        draw_line_box(ui, at(19, 9), 8, 6);
        place_lines(ui, at(10, 9), &encode("TYPE1/<NEXT>TYPE2/<NEXT><ID>№/<NEXT>OT/"), false);
        let hp_bar = draw_hp(ui, at(11, 3), mon.hp, stats[0], false, HpBarType::StatusScreenOrBattle);
        ctx.screen.sgb.run(&PaletteCommand::StatusScreen {
            hp_bar,
            mon: determine_palette_id_out_of_battle(mon.species as u8),
        });
        let ui = &mut ctx.screen.ui;
        if !print_status_condition(ui, at(16, 6), mon.status, mon.hp) {
            place_lines(ui, at(16, 6), &encode("OK"), false);
        }
        place_lines(ui, at(9, 6), &encode("STATUS/"), false);
        print_level(ui, at(14, 2), *level);
        let zeroes = |digits| NumberFormat { digits, leading_zeroes: true, left_align: false };
        print_number(ui, at(3, 7), BaseStats::of(mon.species).dex as u32, zeroes(3));
        print_mon_type(ui, at(11, 10), mon.species);
        place_lines(ui, at(9, 1), &self.mon.nick, false);
        place_lines(ui, at(12, 16), &self.mon.ot, false);
        print_number(ui, at(12, 14), mon.ot_id as u32, zeroes(5));
        print_stats_box(ui, StatsBox::StatusScreen, *stats);
    }

    /// `LoadFlippedFrontSpriteByMonIndex` at (1, 0), then the cry.
    fn show_picture(&self, ctx: &mut Ctx) {
        let species = self.mon.mon.mon.species;
        ctx.screen.tiles.load(V_CHARS2, &front_pic_tiles(species, true).concat());
        for column in 0..7u8 {
            for row in 0..7u8 {
                ctx.screen.ui.set(7 - column as usize, row as usize, column * 7 + row);
            }
        }
        ctx.audio.play_cry(species as u8);
    }

    /// `StatusScreen2` up to its wait for a press.
    fn draw_second_page(&self, ctx: &mut Ctx) {
        let PartyMon { mon, level, .. } = &self.mon.mon;
        let moves = format_moves_string(&mon.moves);
        let ui = &mut ctx.screen.ui;
        let at = |x: usize, y: usize| y * SCREEN_TILES_X + x;
        ui.fill(9, 2, 10, 5, UiSurface::BLANK);
        ui.set(19, 3, VERTICAL);
        ui.text_box_border(0, 8, 18, 8);
        place_lines(ui, at(2, 9), &moves.string, false);

        let known = mon.moves.iter().take_while(|known| known.is_some()).count();
        for slot in 0..4 {
            let label = if slot < known { BOLD_P } else { DASH };
            ui.set(11, 10 + 2 * slot, label);
            ui.set(12, 10 + 2 * slot, label);
        }
        let two = NumberFormat { digits: 2, leading_zeroes: false, left_align: false };
        for (slot, known) in mon.moves.iter().map_while(|known| *known).enumerate() {
            let end = print_number(ui, at(14, 10 + 2 * slot), pp_left(mon.pp[slot]) as u32, two);
            ui.set(end % SCREEN_TILES_X, end / SCREEN_TILES_X, encode("/")[0]);
            print_number(ui, end + 1, max_pp(known, mon.pp[slot]) as u32, two);
        }

        place_lines(ui, at(9, 3), &encode("EXP POINTS<NEXT>LEVEL UP"), false);
        ui.set(14, 6, TO);
        // The level printed is the next one, and 100 is shown as itself.
        print_level(ui, at(16, 6), if *level == MAX_LEVEL { *level } else { level.wrapping_add(1) });
        let seven = NumberFormat { digits: 7, leading_zeroes: false, left_align: false };
        print_number(ui, at(12, 4), mon.exp, seven);
        let growth_rate = mon.base_stats().growth_rate;
        print_number(ui, at(7, 6), calc_exp_to_level_up(growth_rate, *level, mon.exp), seven);
        // `StatusScreen_ClearName` twice, the first clearing a row only the Japanese version uses.
        ui.fill(9, 0, 10, 2, UiSurface::BLANK);
        place_lines(ui, at(9, 1), &mon.species.name(), false);
    }

    /// `WaitForTextScrollButtonPress`, polled once a frame.
    fn pressed(ctx: &mut Ctx) -> bool {
        ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B)
    }

    fn first_page(&mut self, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::FirstPage;
        if Self::pressed(ctx) {
            self.answered += 1;
            ctx.screen.tiles.animation.kind = 0;
            self.draw_second_page(ctx);
            return self.second_page(ctx);
        }
        Transition::Stay
    }

    fn second_page(&mut self, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::SecondPage;
        if Self::pressed(ctx) {
            self.answered += 1;
            ctx.screen.tiles.animation.kind = self.tile_animations;
            ctx.audio.no_audio_fade_out = false;
            ctx.audio.set_master_volume(LOUD);
            ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
            return Transition::Pop(Outcome::Done);
        }
        Transition::Stay
    }
}

impl ModeUpdate for StatusScreen {
    fn enter(&mut self, ctx: &mut Ctx) {
        ctx.audio.no_audio_fade_out = true;
        ctx.audio.set_master_volume(QUIET);
    }

    /// Everything up to the cry happens in the frame the screen is pushed.
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        self.tile_animations = ctx.screen.tiles.animation.kind;
        ctx.screen.tiles.animation.kind = 0;
        self.draw_first_page(ctx);
        self.show_picture(ctx);
        self.phase = Phase::Cry;
        self.update(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Cry if ctx.audio.sound_finished() => self.first_page(ctx),
            Phase::Cry => Transition::Stay,
            Phase::FirstPage => self.first_page(ctx),
            Phase::SecondPage => self.second_page(ctx),
        }
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::FirstPage | Phase::SecondPage => Status::Waiting(Decision::StatusScreen),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::species::PokemonSpecies;
    use crate::command::{Command, Reply};
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    fn mon() -> Named<PartyMon> {
        let mut mon = new_party_mon(PokemonSpecies::Bulbasaur, 12, 1234, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.hp = 20;
        Named { mon, ot: encode("RED"), nick: encode("BULBY") }
    }

    fn game(mon: Named<PartyMon>) -> Game {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::StatusScreen(StatusScreen::new(mon)));
        game
    }

    fn until_waiting(game: &mut Game) -> u32 {
        for frames in 1..600 {
            game.frame(Input::None);
            if game.status() == Status::Waiting(Decision::StatusScreen) {
                return frames;
            }
        }
        panic!("the status screen never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn text(game: &Game, x: usize, y: usize, word: &str) -> bool {
        let bytes = encode(word);
        game.ui().row(y)[x..x + bytes.len()] == bytes[..]
    }

    #[test]
    fn page_one_has_the_names_the_numbers_and_the_stats() {
        let mut game = game(mon());
        until_waiting(&mut game);
        assert!(text(&game, 9, 1, "BULBY"), "the nickname");
        assert!(text(&game, 1, 7, "№<DOT>001"), "the dex number");
        assert!(text(&game, 9, 6, "STATUS/OK"));
        assert!(text(&game, 11, 10, "GRASS") && text(&game, 11, 12, "POISON"), "both types");
        assert!(text(&game, 12, 14, "01234") && text(&game, 12, 16, "RED"));
        assert!(text(&game, 1, 9, "ATTACK"));
        assert_eq!(game.ui().get(14, 2), 0x6E, "<LV>");
        assert!(text(&game, 12, 4, " 20/"), "the fraction under the bar");
        assert_eq!(game.ui().get(7, 0), 0, "the picture's first tile is its top right");
    }

    /// The press is not taken until the cry has finished.
    #[test]
    fn the_cry_is_waited_out_before_the_page_takes_a_press() {
        let mut game = game(mon());
        let frames = until_waiting(&mut game);
        assert_eq!(frames, 48, "Bulbasaur's cry, and nothing before it");
        assert!(game.audio().sound_finished());
    }

    #[test]
    fn page_two_shows_the_moves_the_experience_and_the_species_name() {
        let mut game = game(mon());
        until_waiting(&mut game);
        press(&mut game, Joypad::A);
        until_waiting(&mut game);
        assert!(text(&game, 9, 1, "BULBASAUR"), "the species where the nickname was");
        assert!(text(&game, 2, 9, "TACKLE") && text(&game, 2, 11, "GROWL"));
        assert_eq!(game.ui().row(10)[11..13], [BOLD_P, BOLD_P], "PP for a move");
        assert_eq!(game.ui().row(16)[11..13], [DASH, DASH], "and dashes where the fourth would be");
        assert!(text(&game, 14, 10, "35/35"));
        assert!(text(&game, 9, 3, "EXP POINTS") && text(&game, 9, 5, "LEVEL UP"));
        assert_eq!(game.ui().row(6)[14..18], [TO, 0x7F, 0x6E, 0xF6 + 1], "<to> and the next level");
        assert_eq!(game.ui().row(6)[18], 0xF6 + 3);
    }

    #[test]
    fn a_second_press_clears_the_screen_and_leaves() {
        let mut game = game(mon());
        until_waiting(&mut game);
        press(&mut game, Joypad::A);
        until_waiting(&mut game);
        press(&mut game, Joypad::B);
        for _ in 0..3 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
        assert!((0..18).all(|y| game.ui().row(y).iter().all(|&tile| tile == UiSurface::BLANK)));
    }

    #[test]
    fn a_box_mon_is_shown_at_the_level_it_was_put_away_at() {
        let named = mon();
        let mut boxed = crate::systems::add_mon::deposit(named.mon.clone());
        boxed.box_level = 30;
        let screen = StatusScreen::from_box(Named { mon: boxed, ot: named.ot, nick: named.nick });
        assert_eq!(screen.mon.mon.level, 30);
        assert!(screen.mon.mon.stats[1] > named.mon.stats[1]);
    }

    #[test]
    fn advance_turns_each_page() {
        let mut game = game(mon());
        until_waiting(&mut game);
        assert_eq!(game.frame(Input::Command(Command::Advance)).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..30 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::Advance)]);
        assert!(text(&game, 9, 3, "EXP POINTS"));
    }

    #[test]
    fn a_save_mid_screen_resumes_identically() {
        let mut whole = game(mon());
        for _ in 0..8 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..120 {
            let input = || if frame == 80 { Input::Buttons(Joypad::A) } else { Input::None };
            let (a, b) = (whole.frame(input()), restored.frame(input()));
            assert_eq!((whole.ui(), a.events, a.audio), (restored.ui(), b.events, b.audio), "frame {frame}");
        }
    }
}
