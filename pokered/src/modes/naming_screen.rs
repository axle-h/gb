//! `DisplayNamingScreen`: a grid of letters, a cursor that wraps, and a name built a tile at a time.
//!
//! The cursor's column and row *are* the state: `.pressedA` reads the letter back out of the tilemap
//! at `wMenuCursorLocation + 1` rather than from any table, so what is on screen is what is typed.
//! A name is handed back in `wStringBuffer`, and an empty one is how the screen says it was cancelled.

use poke_core::species::PokemonSpecies;
use poke_core::text_script::TextBuffer;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::mon_icons::{animate_party_mon, clear_sprites, load_mon_party_sprite_gfx, write_mon_party_sprite_oam};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::{put, tile_at};
use crate::systems::hp_bar::HpBarColour;

/// `$76` and `$77`: the underscore, and the raised one under the next letter.
const UNDERSCORE: u8 = 0x76;
const RAISED: u8 = 0x77;
const CURSOR: u8 = 0xED;
/// A leftover from the Japanese version, blank in English.
const JAPANESE_NO: u8 = 0xC9;
/// `wOnSGB`, which only changes how fast the icon bobs.
const ON_SGB: bool = false;
/// `AnimatePartyMon_ForceSpeed1` takes the yellow speed whatever the mon's HP bar says, and slot 0
/// whatever the grid cursor is on: it saves `wCurrentMenuItem` around the call.
const ICON_COLOUR: HpBarColour = HpBarColour::Yellow;

/// The grid is five rows of nine, every other column, with the case switch on a sixth row.
const GRID_ROWS: u8 = 5;
const FIRST_COLUMN: u8 = 1;
const LAST_COLUMN: u8 = 17;
const CASE_ROW: u8 = 6;

const UPPER: [&str; 6] = ["ABCDEFGHI", "JKLMNOPQR", "STUVWXYZ ", "×():;[]<PK><MN>", "-?!♂♀/<DOT>,<ED>", "lower case"];
const LOWER: [&str; 6] = ["abcdefghi", "jklmnopqr", "stuvwxyz ", "×():;[]<PK><MN>", "-?!♂♀/<DOT>,<ED>", "UPPER CASE"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum NamingScreenType {
    Player,
    Rival,
    Mon,
}

impl NamingScreenType {
    /// `PLAYER_NAME_LENGTH - 1` or `NAME_LENGTH - 1`: how many letters fit.
    pub fn limit(self) -> usize {
        match self {
            Self::Mon => 10,
            _ => 7,
        }
    }
}

/// Which of the three points the cartridge's button handlers return to.
enum Return {
    /// Redraw the alphabet, then fall through the other two.
    Alphabet,
    /// Redraw the name and its underscores, then place the cursor.
    Name,
    /// Place the cursor and take the next press.
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamingScreen {
    kind: NamingScreenType,
    species: Option<PokemonSpecies>,
    /// `wStringBuffer`, charmap bytes, unterminated.
    name: Vec<u8>,
    /// `wCurrentMenuItem`: the row, 1 to 6.
    row: u8,
    /// `wTopMenuItemX`: the column, odd, 1 to 17.
    column: u8,
    /// `wAlphabetCase`.
    lower_case: bool,
    /// `wNamingScreenSubmitName`.
    submit: bool,
    /// Whether the input loop has run since the screen was drawn, which is when it takes a press:
    /// `AnimatePartyMon_ForceSpeed1` waits a frame before every read of the pad.
    polling: bool,
    /// `wAnimCounter`, which the icon bobs on.
    anim_counter: u8,
}

impl NamingScreen {
    pub fn new(kind: NamingScreenType, species: Option<PokemonSpecies>) -> Self {
        Self {
            kind,
            species,
            name: Vec::new(),
            row: 1,
            column: FIRST_COLUMN,
            lower_case: false,
            submit: false,
            polling: false,
            anim_counter: 0,
        }
    }

    pub fn kind(&self) -> NamingScreenType {
        self.kind
    }

    /// The mon a nickname is for.
    pub fn species(&self) -> Option<PokemonSpecies> {
        self.species
    }

    pub fn name(&self) -> &[u8] {
        &self.name
    }

    pub fn limit(&self) -> usize {
        self.kind.limit()
    }

    /// Where a letter sits on the grid, and which case shows it. `ED` is left out: it is the way
    /// out rather than a letter, so a driver must never walk onto it looking for one.
    pub fn position_of(byte: u8) -> Option<(u8, u8, bool)> {
        for (lower, rows) in [(false, UPPER), (true, LOWER)] {
            for (row, line) in rows.iter().take(GRID_ROWS as usize).enumerate() {
                let bytes = poke_core::charmap::encode(line).expect("the alphabet encodes");
                for (column, &found) in bytes.iter().enumerate() {
                    let is_ed = row as u8 + 1 == GRID_ROWS && column == 8;
                    if found == byte && !is_ed {
                        return Some((row as u8 + 1, FIRST_COLUMN + 2 * column as u8, lower));
                    }
                }
            }
        }
        None
    }

    /// The next button that takes a driver towards having `target` typed and handed back.
    pub fn press_toward(&self, target: &[u8]) -> Joypad {
        if !target.starts_with(&self.name) {
            return Joypad::B;
        }
        let Some(&next) = target.get(self.name.len()) else { return Joypad::START };
        let Some((row, column, lower)) = Self::position_of(next) else { return Joypad::START };
        if lower != self.lower_case {
            return Joypad::SELECT;
        }
        if self.row != row {
            return if self.row < row { Joypad::DOWN } else { Joypad::UP };
        }
        if self.column != column {
            return if self.column < column { Joypad::RIGHT } else { Joypad::LEFT };
        }
        Joypad::A
    }

    /// Where the cursor stands, as an index into the grid.
    fn cursor_at(&self) -> usize {
        (3 + 2 * self.row as usize) * SCREEN_TILES_X + self.column as usize
    }

    /// `PrintAlphabet`: five rows of nine every other column, then the case switch beneath them.
    fn print_alphabet(&self, ui: &mut UiSurface) {
        let rows = if self.lower_case { LOWER } else { UPPER };
        for (row, line) in rows.iter().take(GRID_ROWS as usize).enumerate() {
            let bytes = poke_core::charmap::encode(line).expect("the alphabet encodes");
            for (column, &byte) in bytes.iter().enumerate() {
                ui.set(2 + column * 2, 5 + row * 2, byte);
            }
        }
        let label = poke_core::charmap::encode(rows[5]).expect("the case switch encodes");
        ui.place(2, 15, &label);
    }

    /// `PrintNicknameAndUnderscores`. A full name parks the cursor on `ED` and keeps the last
    /// underscore raised, so there is nowhere to type but the way out.
    fn print_name(&mut self, ctx: &mut Ctx) {
        let limit = self.kind.limit();
        ctx.screen.ui.fill(10, 2, 10, 1, UiSurface::BLANK);
        ctx.screen.ui.place(10, 2, &self.name);
        for i in 0..limit {
            ctx.screen.ui.set(10 + i, 3, UNDERSCORE);
        }
        let raised = if self.name.len() == limit {
            ctx.menu.erase_cursor(&mut ctx.screen.ui);
            self.column = LAST_COLUMN;
            self.row = GRID_ROWS;
            limit - 1
        } else {
            self.name.len()
        };
        ctx.screen.ui.set(10 + raised, 3, RAISED);
    }

    /// `PlaceMenuCursor`, which works out where the old cursor was from the *new* column. That is
    /// why a move that changes the column erases the old one itself first.
    fn place_cursor(&self, ctx: &mut Ctx) {
        let at = |row: u8| (3 + 2 * row as usize) * SCREEN_TILES_X + self.column as usize;
        let old = at(ctx.menu.last_item);
        if tile_at(&ctx.screen.ui, old) == CURSOR {
            put(&mut ctx.screen.ui, old, ctx.menu.tile_behind);
        }
        let new = at(self.row);
        ctx.menu.place_at(&mut ctx.screen.ui, new);
        ctx.menu.last_item = self.row;
    }

    /// `PrintNamingText`: the words above the grid.
    fn print_header(&self, ui: &mut UiSurface) {
        let text = |s: &str| poke_core::charmap::encode(s).expect("the header encodes");
        match self.kind {
            NamingScreenType::Player => ui.place(0, 1, &text("YOUR NAME?")),
            NamingScreenType::Rival => ui.place(0, 1, &text("RIVAL's NAME?")),
            NamingScreenType::Mon => {
                let species = self.species.expect("a nickname screen names a species");
                let name = PokemonSpecies::name(species);
                ui.place(4, 1, &name);
                ui.set(4 + name.len() + 1, 1, JAPANESE_NO);
                ui.place(1, 3, &text("NICKNAME?"));
            }
        }
    }

    /// One of the eight buttons, answered as the cartridge's jump table answers it.
    fn pressed(&mut self, keys: Joypad, ctx: &mut Ctx) -> Return {
        if keys.contains(Joypad::DOWN) {
            self.row += 1;
            if self.row == 7 {
                self.row = 1;
                self.column = FIRST_COLUMN;
                ctx.menu.erase_cursor(&mut ctx.screen.ui);
            } else if self.row == CASE_ROW {
                self.column = FIRST_COLUMN;
                ctx.menu.erase_cursor(&mut ctx.screen.ui);
            }
            return Return::Cursor;
        }
        if keys.contains(Joypad::UP) {
            self.row -= 1;
            if self.row == 0 {
                self.row = CASE_ROW;
                self.column = FIRST_COLUMN;
                ctx.menu.erase_cursor(&mut ctx.screen.ui);
            }
            return Return::Cursor;
        }
        if keys.intersects(Joypad::LEFT | Joypad::RIGHT) {
            // The case switch is one wide, so it cannot be stepped along.
            if self.row == CASE_ROW {
                return Return::Cursor;
            }
            self.column = if keys.contains(Joypad::RIGHT) {
                if self.column == LAST_COLUMN { FIRST_COLUMN } else { self.column + 2 }
            } else if self.column == FIRST_COLUMN {
                LAST_COLUMN
            } else {
                self.column - 2
            };
            ctx.menu.erase_cursor(&mut ctx.screen.ui);
            return Return::Cursor;
        }
        if keys.contains(Joypad::START) {
            self.submit = true;
            return Return::Name;
        }
        if keys.contains(Joypad::SELECT) {
            self.lower_case = !self.lower_case;
            return Return::Alphabet;
        }
        if keys.contains(Joypad::B) {
            self.name.pop();
            return Return::Name;
        }
        // A: `ED` submits, the case switch switches, and anything else is a letter.
        if self.row == GRID_ROWS && self.column == LAST_COLUMN {
            self.submit = true;
            return Return::Name;
        }
        if self.row == CASE_ROW && self.column == FIRST_COLUMN {
            self.lower_case = !self.lower_case;
            return Return::Alphabet;
        }
        if self.name.len() < self.kind.limit() {
            // The letter is read off the screen, one tile right of the cursor.
            self.name.push(tile_at(&ctx.screen.ui, self.cursor_at() + 1));
            // `.addLetter` sounds only for a letter actually added.
            ctx.audio.play_sound(crate::audio::data::sounds::SFX_PRESS_AB);
        }
        Return::Name
    }

    /// `.inputLoop`'s `AnimatePartyMon_ForceSpeed1`, which runs whatever the screen is naming; only
    /// a nickname screen has an icon in OAM for it to move.
    fn animate(&mut self, ctx: &mut Ctx) {
        let party: Vec<_> = self.species.into_iter().collect();
        animate_party_mon(&mut ctx.screen.sprites, &mut self.anim_counter, 0, ICON_COLOUR, &party, ON_SGB);
    }

    /// `.ABStartReturnPoint` onwards: the name, the underscores and the cursor.
    fn redraw(&mut self, ctx: &mut Ctx) -> Transition {
        if self.submit {
            ctx.world.text.strings.insert(TextBuffer::StringBuffer, self.name.clone());
            // `.submitNickname`'s `ClearSprites`, and `wAnimCounter` back to zero for the next screen.
            clear_sprites(&mut ctx.screen.sprites);
            self.anim_counter = 0;
            return Transition::Pop(Outcome::Done);
        }
        self.print_name(ctx);
        self.place_cursor(ctx);
        Transition::Stay
    }
}

impl ModeUpdate for NamingScreen {
    fn enter(&mut self, ctx: &mut Ctx) {
        ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        // `SET_PAL_GENERIC`, which nothing puts back: the caller's `LoadGBPal` is the DMG's palette.
        ctx.screen.sgb.run(&PaletteCommand::Generic);
        ctx.screen.tiles.load_hp_bar_and_status_tiles();
        ctx.screen.tiles.load_ed_tile();
        load_mon_party_sprite_gfx(&mut ctx.screen.tiles);
        ctx.screen.ui.text_box_border(0, 4, 18, 9);
        self.print_header(&mut ctx.screen.ui);
        // `PrintNamingText` writes the icon as OAM slot 0, `hPartyMonIndex` forced to zero.
        if let Some(species) = self.species {
            write_mon_party_sprite_oam(&mut ctx.screen.sprites, 0, species);
        }
        self.anim_counter = 0;
        ctx.menu.last_item = self.row;
        self.print_alphabet(&mut ctx.screen.ui);
    }

    /// `PrintAlphabet`'s `Delay3` is loading and not modelled, so the name and the cursor go up in
    /// the frame the alphabet does.
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.redraw(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        self.polling = true;
        self.animate(ctx);
        let keys = ctx.pad.low_sensitivity(ctx.frame_counter);
        if keys.is_empty() {
            return Transition::Stay;
        }
        match self.pressed(keys, ctx) {
            Return::Alphabet => {
                self.print_alphabet(&mut ctx.screen.ui);
                self.redraw(ctx)
            }
            Return::Name => self.redraw(ctx),
            Return::Cursor => {
                self.place_cursor(ctx);
                Transition::Stay
            }
        }
    }

    fn status(&self) -> Status {
        if self.polling { Status::Waiting(Decision::NamingScreen) } else { Status::Busy }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    fn start(kind: NamingScreenType) -> Game {
        let species = (kind == NamingScreenType::Mon).then_some(PokemonSpecies::Pidgey);
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::NamingScreen(NamingScreen::new(kind, species)));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::NamingScreen) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the naming screen never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn tap(game: &mut Game, button: Joypad) {
        until_waiting(game);
        press(game, button);
    }

    fn screen(game: &Game) -> &NamingScreen {
        match game.modes().last() {
            Some(Mode::NamingScreen(screen)) => screen,
            _ => panic!("the naming screen closed"),
        }
    }

    fn cursor(game: &Game) -> Option<(usize, usize)> {
        (0..18).flat_map(|y| (0..20).map(move |x| (x, y))).find(|&(x, y)| game.ui().get(x, y) == CURSOR)
    }

    fn typed(game: &Game) -> Vec<u8> {
        game.world().text.string(TextBuffer::StringBuffer)
    }

    #[test]
    fn the_screen_sends_the_generic_palette() {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.screen_mut().sgb.run(&PaletteCommand::TownMap);
        game.push(Mode::NamingScreen(NamingScreen::new(NamingScreenType::Player, None)));
        until_waiting(&mut game);
        let mut generic = crate::gfx::sgb::SgbState::default();
        generic.run(&PaletteCommand::Generic);
        assert_eq!(game.screen().sgb.palette_ids(), generic.palette_ids());
    }

    #[test]
    fn five_rows_of_nine_every_other_column() {
        let mut game = start(NamingScreenType::Player);
        until_waiting(&mut game);
        assert_eq!(game.ui().get(2, 5), encode("A").unwrap()[0]);
        assert_eq!(game.ui().get(4, 5), encode("B").unwrap()[0], "two columns apart");
        assert_eq!(game.ui().get(2, 7), encode("J").unwrap()[0], "two rows apart");
        assert_eq!(game.ui().get(18, 13), 0xF0, "ED, on the tile the yen sign draws from");
        assert_eq!(game.ui().row(15)[2..12], encode("lower case").unwrap()[..]);
        assert_eq!(cursor(&game), Some((1, 5)), "one tile left of the first letter");
    }

    /// `.pressedA` reads the letter out of the tilemap, so what is typed is whatever is on screen.
    #[test]
    fn a_types_the_letter_beside_the_cursor() {
        let mut game = start(NamingScreenType::Player);
        tap(&mut game, Joypad::A);
        tap(&mut game, Joypad::RIGHT);
        tap(&mut game, Joypad::A);
        until_waiting(&mut game);
        assert_eq!(screen(&game).name(), &encode("AB").unwrap()[..]);
        assert_eq!(game.ui().row(2)[10..12], encode("AB").unwrap()[..], "and it is shown as it goes");
    }

    #[test]
    fn select_swaps_the_alphabet_under_the_cursor() {
        let mut game = start(NamingScreenType::Player);
        tap(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(game.ui().get(2, 5), encode("a").unwrap()[0]);
        assert_eq!(game.ui().row(15)[2..12], encode("UPPER CASE").unwrap()[..]);
        tap(&mut game, Joypad::A);
        until_waiting(&mut game);
        assert_eq!(screen(&game).name(), &encode("a").unwrap()[..]);
    }

    #[test]
    fn the_columns_wrap_and_the_case_row_does_not_move() {
        let mut game = start(NamingScreenType::Player);
        tap(&mut game, Joypad::LEFT);
        until_waiting(&mut game);
        assert_eq!(cursor(&game), Some((17, 5)), "left from the first column wraps to the last");
        tap(&mut game, Joypad::RIGHT);
        until_waiting(&mut game);
        assert_eq!(cursor(&game), Some((1, 5)));
        tap(&mut game, Joypad::UP);
        until_waiting(&mut game);
        assert_eq!(cursor(&game), Some((1, 15)), "up from the top row is the case switch");
        tap(&mut game, Joypad::RIGHT);
        until_waiting(&mut game);
        assert_eq!(cursor(&game), Some((1, 15)), "which is one wide");
        tap(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(cursor(&game), Some((1, 5)), "and down from it comes back to the top");
    }

    #[test]
    fn b_takes_one_letter_off_and_does_nothing_to_an_empty_name() {
        let mut game = start(NamingScreenType::Player);
        tap(&mut game, Joypad::A);
        tap(&mut game, Joypad::B);
        until_waiting(&mut game);
        assert!(screen(&game).name().is_empty());
        tap(&mut game, Joypad::B);
        until_waiting(&mut game);
        assert!(screen(&game).name().is_empty(), "and it stays empty");
    }

    /// Seven letters for a trainer, ten for a mon, and a full name parks the cursor on the way out.
    /// Once the name is full the cursor is moved onto `ED` for you, so the next A is not a refused
    /// letter but the way out.
    #[test]
    fn a_full_name_parks_the_cursor_on_ed() {
        let mut game = start(NamingScreenType::Player);
        for _ in 0..7 {
            tap(&mut game, Joypad::A);
        }
        until_waiting(&mut game);
        assert_eq!(screen(&game).name(), &encode("AAAAAAA").unwrap()[..]);
        assert_eq!(cursor(&game), Some((17, 13)), "parked on ED");
        assert_eq!(game.ui().get(16, 3), RAISED, "with the last underscore still raised");
        tap(&mut game, Joypad::A);
        for _ in 0..5 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty(), "and A there hands the name back");
        assert_eq!(typed(&game), encode("AAAAAAA").unwrap());
    }

    #[test]
    fn a_mon_gets_ten_letters_and_its_own_header() {
        let mut game = start(NamingScreenType::Mon);
        until_waiting(&mut game);
        assert_eq!(game.ui().row(1)[4..10], PokemonSpecies::name(PokemonSpecies::Pidgey)[..]);
        assert_eq!(game.ui().row(3)[1..10], encode("NICKNAME?").unwrap()[..]);
        for _ in 0..10 {
            tap(&mut game, Joypad::A);
        }
        until_waiting(&mut game);
        assert_eq!(screen(&game).name().len(), 10, "three more than a trainer gets");
    }

    /// `PrintNamingText` writes the icon at OAM slot 0 whatever the grid cursor is doing.
    #[test]
    fn a_nickname_screen_shows_the_mon_s_icon_and_a_trainer_s_does_not() {
        let mut game = start(NamingScreenType::Mon);
        until_waiting(&mut game);
        let icon = crate::gfx::mon_icons::icon_tile(PokemonSpecies::Pidgey);
        let objects = &game.screen().sprites[..4];
        assert_eq!(objects.iter().map(|o| (o.y, o.x)).collect::<Vec<_>>(),
                   [(16, 16), (16, 24), (24, 16), (24, 24)], "four objects in the top-left corner");
        assert_eq!(objects.iter().map(|o| o.tile).collect::<Vec<_>>(), [icon, icon, icon + 2, icon + 2]);
        assert!(game.screen().sprites[4..].iter().all(|o| o.tile == 0), "and nothing else in OAM");

        let mut game = start(NamingScreenType::Player);
        until_waiting(&mut game);
        assert!(game.screen().sprites.iter().all(|o| o.tile == 0), "a trainer's name has no icon");
    }

    /// `AnimatePartyMon_ForceSpeed1` takes the yellow speed rather than the mon's own, so the icon
    /// bobs every 17 frames however healthy the mon is.
    #[test]
    fn the_icon_bobs_at_the_forced_yellow_speed() {
        let mut game = start(NamingScreenType::Mon);
        until_waiting(&mut game);
        let icon = crate::gfx::mon_icons::icon_tile(PokemonSpecies::Pidgey);
        let mut tiles = vec![];
        for _ in 0..34 {
            game.frame(Input::None);
            tiles.push(game.screen().sprites[0].tile);
        }
        assert_eq!(tiles.iter().filter(|&&tile| tile == icon).count(), 17, "{tiles:?}");
        assert_eq!(tiles.iter().filter(|&&tile| tile == icon + 0x40).count(), 17, "ICONOFFSET, frame two");
    }

    #[test]
    fn handing_the_name_back_takes_the_icon_down() {
        let mut game = start(NamingScreenType::Mon);
        tap(&mut game, Joypad::START);
        for _ in 0..5 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
        assert!(game.screen().sprites.iter().all(|o| *o == crate::gfx::layers::Object::default()),
                "`.submitNickname`'s ClearSprites");
    }

    #[test]
    fn ed_and_start_both_hand_the_name_back() {
        let mut game = start(NamingScreenType::Player);
        tap(&mut game, Joypad::A);
        tap(&mut game, Joypad::START);
        for _ in 0..5 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
        assert_eq!(typed(&game), encode("A").unwrap(), "left in wStringBuffer");
    }

    /// An empty name is how the screen says the player backed out, which is what every caller tests.
    #[test]
    fn a_name_never_typed_comes_back_empty() {
        let mut game = start(NamingScreenType::Player);
        tap(&mut game, Joypad::START);
        for _ in 0..5 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty());
        assert!(typed(&game).is_empty());
    }

    #[test]
    fn a_save_mid_name_resumes_identically() {
        let mut whole = start(NamingScreenType::Player);
        tap(&mut whole, Joypad::A);
        tap(&mut whole, Joypad::DOWN);
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..40 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
    }
}

#[cfg(test)]
mod driving {
    use poke_core::charmap::encode;
    use crate::command::{Command, Refusal, Reply};
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    fn start(kind: NamingScreenType) -> Game {
        let species = (kind == NamingScreenType::Mon).then_some(PokemonSpecies::Pidgey);
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::NamingScreen(NamingScreen::new(kind, species)));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::NamingScreen) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the naming screen never waited");
    }

    /// Runs the command to completion and answers with what was left in `wStringBuffer`.
    fn enter(name: &str) -> Vec<u8> {
        let mut game = start(NamingScreenType::Player);
        until_waiting(&mut game);
        let command = Command::EnterName(encode(name).unwrap());
        assert_eq!(game.frame(Input::Command(command.clone())).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..600 {
            events.extend(game.frame(Input::None).events);
            if !events.is_empty() {
                break;
            }
        }
        assert_eq!(events, [Event::CommandDone(command)], "{name}");
        assert!(game.modes().is_empty(), "the screen handed it back");
        game.world().text.string(TextBuffer::StringBuffer)
    }

    #[test]
    fn a_command_walks_the_grid_and_types_the_name() {
        assert_eq!(enter("RED"), encode("RED").unwrap());
    }

    /// A letter in the other case costs a SELECT on the way, which the driver works out for itself.
    #[test]
    fn a_mixed_case_name_switches_the_alphabet_as_it_goes() {
        assert_eq!(enter("Red"), encode("Red").unwrap());
    }

    #[test]
    fn a_name_of_the_full_length_is_typed_and_still_handed_back() {
        assert_eq!(enter("ABCDEFG"), encode("ABCDEFG").unwrap());
    }

    #[test]
    fn a_name_longer_than_the_screen_takes_is_refused() {
        let mut game = start(NamingScreenType::Player);
        until_waiting(&mut game);
        let reply = game.frame(Input::Command(Command::EnterName(encode("ABCDEFGH").unwrap()))).reply;
        assert!(matches!(reply, Some(Reply::Refused(Refusal::Invalid(_)))), "{reply:?}");
    }

    #[test]
    fn a_letter_the_grid_does_not_have_is_refused() {
        let mut game = start(NamingScreenType::Player);
        until_waiting(&mut game);
        let reply = game.frame(Input::Command(Command::EnterName(vec![0x00]))).reply;
        assert!(matches!(reply, Some(Reply::Refused(Refusal::Invalid(_)))), "{reply:?}");
    }
}
