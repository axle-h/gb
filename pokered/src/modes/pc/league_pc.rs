//! `PKMNLeaguePC`: the Hall of Fame's teams in `World::hall_of_fame`, oldest first, a mon a screen.
//!
//! Each screen is `LeaguePCShowMon`: the mon's picture, the team's number under it and
//! `HoFDisplayMonInfo`'s name, level and types, then its cry and a wait for A or B with no `▼`. B
//! held as that wait ends leaves at once. Past `HOF_TEAM_CAPACITY` teams the oldest are gone, and the
//! numbers start where the record does.
//!
//! Loading, and not modelled: `GBPalWhiteOutWithDelay3` before each screen and on the way out, and
//! the picture's decompression.

use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::text_script::TextCommand;
use serde::{Deserialize, Serialize};
use crate::gfx::sgb::{determine_palette_id_out_of_battle, PaletteCommand};
use crate::gfx::tiles::V_CHARS2;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, Outcome, Transition};
use crate::modes::text_box::TextBox;
use crate::party::PARTY_LENGTH;
use crate::systems::hall_of_fame::HOF_TEAM_CAPACITY;
use crate::systems::pokedex::front_pic_tiles;
use crate::systems::print_num::{print_number, NumberFormat};
use crate::systems::status_screen::{place_lines, print_mon_type};
use super::print_at;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaguePc {
    /// `wHoFTeamIndex2`.
    team: u8,
    /// Which of the team is on screen.
    mon: u8,
    /// `wHoFTeamNo` before the first team: the teams no longer recorded.
    dropped: u8,
    /// `hTileAnimations` as the PC found it.
    tile_animations: u8,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Accessed,
    /// `PlayCry`'s wait.
    Cry,
    Shown,
}

impl LeaguePc {
    /// `PKMNLeaguePC` up to its first text.
    pub fn start(ctx: &mut Ctx) -> (Self, Transition) {
        let pc = Self { team: 0, mon: 0, dropped: 0, tile_animations: 0, phase: Phase::Accessed };
        (pc, print_at(sym::AccessedHoFPCText, ctx))
    }

    pub fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Cry if ctx.audio.sound_finished() => {
                self.phase = Phase::Shown;
                Transition::Push(Mode::TextBox(TextBox::without_box(vec![TextCommand::WaitButton])))
            }
            _ => Transition::Stay,
        }
    }

    /// A child has closed. `Pop` means the PC has finished.
    pub fn resume(&mut self, _outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Accessed => {
                ctx.world.no_text_delay = true;
                self.tile_animations = ctx.screen.tiles.animation.kind;
                ctx.screen.tiles.animation.kind = 0;
                self.dropped = ctx.world.hall_of_fame_teams.saturating_sub(HOF_TEAM_CAPACITY as u8);
                self.show(ctx)
            }
            Phase::Cry => Transition::Stay,
            Phase::Shown if ctx.pad.held.contains(Joypad::B) => self.finish(ctx),
            Phase::Shown => {
                self.mon += 1;
                if self.mon as usize >= PARTY_LENGTH || self.entry(ctx).is_none() {
                    self.team += 1;
                    self.mon = 0;
                }
                if self.team >= self.teams(ctx) {
                    return self.finish(ctx);
                }
                self.show(ctx)
            }
        }
    }

    /// `wNumHoFTeams`, as far as the record holds.
    fn teams(&self, ctx: &Ctx) -> u8 {
        ctx.world.hall_of_fame_teams.min(HOF_TEAM_CAPACITY as u8)
    }

    fn entry<'a>(&self, ctx: &'a Ctx) -> Option<&'a crate::systems::hall_of_fame::HallOfFameMon> {
        ctx.world.hall_of_fame.get(self.team as usize)?.get(self.mon as usize)
    }

    /// `LeaguePCShowMon` to its cry.
    fn show(&mut self, ctx: &mut Ctx) -> Transition {
        let Some(entry) = self.entry(ctx).cloned() else { return self.finish(ctx) };
        let species = entry.species;
        let ui = &mut ctx.screen.ui;
        ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        ctx.screen.sgb.run(&PaletteCommand::PokemonWholeScreen { mon: determine_palette_id_out_of_battle(species as u8), black: false });
        ctx.screen.tiles.load(V_CHARS2, &front_pic_tiles(species, false).concat());
        let ui = &mut ctx.screen.ui;
        for column in 0..7 {
            for row in 0..7 {
                ui.set(12 + column, 5 + row, (column * 7 + row) as u8);
            }
        }
        ui.text_box_border(0, 13, 18, 2);
        place_lines(ui, 15 * SCREEN_TILES_X + 1, rom_slice(sym::HallOfFameNoText), false);
        let number = NumberFormat { digits: 3, leading_zeroes: false, left_align: false };
        print_number(ui, 15 * SCREEN_TILES_X + 16, self.dropped as u32 + self.team as u32 + 1, number);
        // `HoFDisplayMonInfo`.
        ui.text_box_border(0, 2, 10, 9);
        place_lines(ui, 6 * SCREEN_TILES_X + 2, rom_slice(sym::HoFMonInfoText), false);
        ui.place(1, 4, &entry.nick);
        let level = NumberFormat { digits: 3, leading_zeroes: false, left_align: true };
        print_number(ui, 7 * SCREEN_TILES_X + 8, entry.level as u32, level);
        print_mon_type(ui, 9 * SCREEN_TILES_X + 3, species);
        ctx.audio.play_cry(species as u8);
        self.phase = Phase::Cry;
        Transition::Stay
    }

    /// `.doneShowingTeams`: the flags put back and the screen cleared for the PC's menu.
    fn finish(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.screen.tiles.animation.kind = self.tile_animations;
        ctx.world.no_text_delay = false;
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        ctx.screen.sgb.run(&PaletteCommand::Default);
        Transition::Pop(Outcome::Done)
    }
}
