//! `StartMenu_TrainerInfo`, `DrawTrainerInfo` and `DrawBadges`: the player's picture, name, money
//! and play time over the eight gym leaders, each face replaced by its badge once won.
//!
//! The picture goes where `DisplayPicCenteredOrUpperRight` puts an upper-right picture, `(15, 1)`,
//! which runs two columns off the right of the screen and wraps onto the left of the next row. The
//! card then blanks exactly those two columns, shifts the picture's tiles down a column, and draws
//! its border over the last column and row, so four columns by six rows of it show.
//!
//! Every frame between the press that opens it and its wait is loading (`ClearScreen`, the picture,
//! the LCD-off tile copies), and so is the way back; the wait for A or B is the one thing a player
//! sees take time. `WaitForTextScrollButtonPress` seeds its blink with zero, so no `▼` is drawn.

use poke_core::mon_gfx::pic_shades;
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::tiles::{V_CHARS1, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::systems::pokedex::pic_tiles;
use crate::systems::print_num::{print_bcd, print_number, BcdFormat, NumberFormat};
use crate::systems::status_screen::{encode, place_lines};

/// `hlcoord 15, 1`, the upper-right picture's top-left.
const PICTURE_AT: usize = SCREEN_TILES_X + 15;
const PIC_TILES: usize = 7;
/// The tiles `DrawTrainerInfo` loads over the text box's and the font's.
const BORDER_TILES: u8 = 0x77;
const LEADER_NAMES: u8 = 0x60;
const FACES: u8 = 0x20;
const COLON: u8 = 0xD6;
const BACKGROUND: u8 = 0xD7;
const BADGE_NUMBERS: u8 = 0xD8;
const CIRCLE: u8 = 0x76;
/// `TextBoxGraphics tile 13`, the colon.
const TEXT_BOX_COLON: usize = 13;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainerCard {
    /// `hTileAnimations`, pushed on the way in.
    tile_animations: u8,
    polled: bool,
    answered: u32,
}

impl TrainerCard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Presses taken, so a driver can see its press land.
    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// `DrawTrainerInfo`.
    fn draw_trainer_info(ctx: &mut Ctx) {
        let tiles = &mut ctx.screen.tiles;
        let picture = pic_tiles(&pic_shades(rom_slice(pokered_symbols::RedPicFront)), false).concat();
        tiles.load(V_CHARS2, &picture);
        let ui = &mut ctx.screen.ui;
        for column in 0..PIC_TILES {
            for row in 0..PIC_TILES {
                // `CopyUncompressedPicToHL` steps a tile at a time and never checks the edge.
                put(ui, PICTURE_AT + column + row * SCREEN_TILES_X, (column * PIC_TILES + row) as u8);
            }
        }
        vertical_line(ui, 0, 2, UiSurface::BLANK);
        vertical_line(ui, 1, 2, UiSurface::BLANK);
        // `vChars2 tile $07` over `tile $00`, `$1c` tiles: the picture shifted a column.
        let shifted = picture[PIC_TILES * TILE_BYTES..(PIC_TILES + 0x1C) * TILE_BYTES].to_vec();
        tiles.load(V_CHARS2, &shifted);

        let tiles_at = |start: DmgPointer, count: usize| &rom_slice(start)[..count * TILE_BYTES];
        tiles.load(V_CHARS2 + BORDER_TILES as usize, tiles_at(pokered_symbols::TrainerInfoTextBoxTileGraphics, 8));
        tiles.load(V_CHARS2 + LEADER_NAMES as usize, tiles_at(pokered_symbols::BlankLeaderNames, 0x17));
        tiles.load(V_CHARS1 + (BADGE_NUMBERS - 0x80) as usize, tiles_at(pokered_symbols::BadgeNumbersTileGraphics, 8));
        tiles.load(V_CHARS2 + FACES as usize, tiles_at(pokered_symbols::GymLeaderFaceAndBadgeTileGraphics, 8 * 8));
        let colon = &tiles_at(pokered_symbols::TextBoxGraphics, TEXT_BOX_COLON + 1)[TEXT_BOX_COLON * TILE_BYTES..];
        tiles.load(V_CHARS1 + (COLON - 0x80) as usize, colon);
        let background = &tiles_at(pokered_symbols::TrainerInfoTextBoxTileGraphics, 9)[8 * TILE_BYTES..];
        tiles.load(V_CHARS1 + (BACKGROUND - 0x80) as usize, background);

        let ui = &mut ctx.screen.ui;
        trainer_info_text_box(ui, 0, 18, 1);
        trainer_info_text_box(ui, SCREEN_TILES_X * 10 + 1, 16, 3);
        vertical_line(ui, 0, 10, BACKGROUND);
        vertical_line(ui, 19, 10, BACKGROUND);
        let at = |x: usize, y: usize| y * SCREEN_TILES_X + x;
        let mut badges_text = vec![CIRCLE];
        badges_text.extend(encode("BADGES"));
        badges_text.push(CIRCLE);
        place_lines(ui, at(6, 9), &badges_text, false);
        place_lines(ui, at(2, 2), &encode("NAME/<NEXT>MONEY/<NEXT>TIME/"), false);
        place_lines(ui, at(7, 2), &ctx.world.player_name, false);
        let money = BcdFormat { skip_leading_zeroes: true, left_align: true, money_sign: true };
        print_bcd(ui, at(8, 4), &ctx.world.money, money);
        let time = ctx.world.play_time;
        let hours = NumberFormat { digits: 3, leading_zeroes: false, left_align: true };
        let end = print_number(ui, at(9, 6), time.hours as u32, hours);
        put(ui, end, COLON);
        let minutes = NumberFormat { digits: 2, leading_zeroes: true, left_align: false };
        print_number(ui, end + 1, time.minutes as u32, minutes);
    }

    /// `WaitForTextScrollButtonPress`, and then the way back to the start menu, which puts the
    /// screen back itself: the font, the default palettes and the map's tiles.
    fn wait(&mut self, ctx: &mut Ctx) -> Transition {
        self.polled = true;
        if !ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
            return Transition::Stay;
        }
        self.answered += 1;
        ctx.screen.tiles.load_font();
        ctx.screen.sgb.run(&PaletteCommand::Default);
        ctx.screen.tiles.load_text_box_tiles();
        if let Some(tileset) = ctx.screen.map.tileset {
            ctx.screen.tiles.load_tileset(tileset);
        }
        ctx.screen.tiles.animation.kind = self.tile_animations;
        Transition::Pop(Outcome::Done)
    }
}

fn put(ui: &mut UiSurface, at: usize, tile: u8) {
    ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
}

/// `TrainerInfo_DrawVerticalLine`: eight tiles down.
fn vertical_line(ui: &mut UiSurface, x: usize, y: usize, tile: u8) {
    for row in y..y + 8 {
        ui.set(x, row, tile);
    }
}

/// `TrainerInfo_DrawTextBox`: six rows high, edges only. The right edge is `width + 1` on from the
/// left and each row starts `next_row` past the right edge, which is how one routine draws a box
/// of any width from any column.
fn trainer_info_text_box(ui: &mut UiSurface, mut at: usize, width: usize, next_row: usize) {
    let edge = |ui: &mut UiSurface, at: &mut usize, left: u8, middle: u8, right: u8| {
        put(ui, *at, left);
        for _ in 0..width {
            *at += 1;
            put(ui, *at, middle);
        }
        *at += 1;
        put(ui, *at, right);
    };
    edge(ui, &mut at, 0x79, 0x7A, 0x7B);
    at += next_row;
    for _ in 0..6 {
        put(ui, at, 0x7C);
        at += width + 1;
        put(ui, at, 0x78);
        at += next_row;
    }
    edge(ui, &mut at, 0x7D, BORDER_TILES, 0x7E);
}

/// `DrawBadges`: four to a row at `(2, 11)` and `(2, 14)`, a badge's number, then its leader's blank
/// name over a two-by-two face. A badge won shows the badge where the face was and skips the name,
/// though the name tiles still count past it.
pub fn draw_badges(ui: &mut UiSurface, badges: u8) {
    const FACE_BADGE_TILES: [u8; 8] = [0x20, 0x28, 0x30, 0x38, 0x40, 0x48, 0x50, 0x58];
    let (mut number, mut name) = (BADGE_NUMBERS, LEADER_NAMES);
    for (badge, &face) in FACE_BADGE_TILES.iter().enumerate() {
        let (x, y) = (2 + 4 * (badge % 4), 11 + 3 * (badge / 4));
        let won = badges & 1 << badge != 0;
        ui.set(x, y, number);
        number += 1;
        if !won {
            ui.set(x + 1, y, name);
            ui.set(x + 2, y, name + 1);
        }
        name += 2;
        let face = if won { face + 4 } else { face };
        ui.set(x + 1, y + 1, face);
        ui.set(x + 2, y + 1, face + 1);
        ui.set(x + 1, y + 2, face + 2);
        ui.set(x + 2, y + 2, face + 3);
    }
}

impl ModeUpdate for TrainerCard {
    /// Everything up to the wait happens in the frame the card is pushed, and the wait reads the
    /// pad in that same frame.
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        self.tile_animations = ctx.screen.tiles.animation.kind;
        ctx.screen.tiles.animation.kind = 0;
        Self::draw_trainer_info(ctx);
        draw_badges(&mut ctx.screen.ui, ctx.world.badges);
        ctx.screen.sgb.run(&PaletteCommand::TrainerCard { badges: ctx.world.badges });
        self.wait(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        self.wait(ctx)
    }

    fn status(&self) -> Status {
        if self.polled { Status::Waiting(Decision::TrainerCard) } else { Status::Busy }
    }
}

#[cfg(test)]
mod tests {
    use crate::command::{Command, Reply};
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::systems::play_time::PlayTime;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    fn game(badges: u8) -> Game {
        let world = World {
            player_name: encode("RED"),
            money: [0x01, 0x23, 0x45],
            badges,
            play_time: PlayTime { hours: 12, minutes: 5, ..PlayTime::default() },
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::TrainerCard(TrainerCard::new()));
        game
    }

    fn row(game: &Game, y: usize, from: usize, text: &str) {
        let bytes = encode(text);
        assert_eq!(game.ui().row(y)[from..from + bytes.len()], bytes[..], "row {y} from {from}");
    }

    #[test]
    fn the_name_the_money_and_the_time() {
        let game = game(0);
        assert_eq!(game.status(), Status::Waiting(Decision::TrainerCard), "it waits in the frame it opens");
        row(&game, 2, 2, "NAME/");
        row(&game, 2, 7, "RED");
        row(&game, 4, 2, "MONEY/");
        row(&game, 4, 8, "¥12345");
        row(&game, 6, 2, "TIME/");
        row(&game, 6, 9, "12");
        assert_eq!(game.ui().get(11, 6), COLON, "left-aligned hours leave the colon right after them");
        row(&game, 6, 12, "05");
    }

    /// The picture wraps past the right edge onto the next row's first two columns, and those are
    /// the columns the card blanks.
    #[test]
    fn the_picture_s_overflow_is_blanked_and_its_last_column_bordered() {
        let game = game(0);
        assert_eq!(game.ui().row(1)[15..19], [0, 7, 14, 21]);
        assert_eq!(game.ui().get(19, 1), 0x78, "the border's right edge covers the last column");
        assert_eq!(game.ui().get(1, 2), UiSurface::BLANK, "what wrapped is rubbed out");
        assert_eq!(game.ui().row(8)[..2], [UiSurface::BLANK; 2], "down to the picture's last row");
    }

    #[test]
    fn a_badge_won_replaces_the_face_and_its_name() {
        let game = game(0b0000_0010);
        assert_eq!(game.ui().row(11)[2..5], [0xD8, 0x60, 0x61], "Brock: number, name");
        assert_eq!(game.ui().row(12)[3..5], [0x20, 0x21], "and face");
        assert_eq!(game.ui().row(11)[6..9], [0xD9, UiSurface::BLANK, UiSurface::BLANK], "Misty's name is gone");
        assert_eq!(game.ui().row(12)[7..9], [0x2C, 0x2D], "and her badge is up");
        assert_eq!(game.ui().row(14)[2..5], [0xDC, 0x68, 0x69], "the names keep counting past a won badge");
    }

    #[test]
    fn a_or_b_goes_back_and_a_command_can_press_it() {
        let mut game = game(0);
        game.frame(Input::Buttons(Joypad::B));
        assert!(game.modes().is_empty());

        let mut game = self::game(0);
        assert_eq!(game.frame(Input::Command(Command::Advance)).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..10 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::Advance)]);
        assert!(game.modes().is_empty());
    }
}
