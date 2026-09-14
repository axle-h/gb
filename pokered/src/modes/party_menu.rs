//! `DisplayPartyMenu` and `RedrawPartyMenu_`: six two-row entries and a cursor down the left.
//!
//! The mon icons are not drawn: they are OAM sprites out of a bank this chunk does not own. Their
//! absence costs nothing in timing, because `AnimatePartyMon` ends in one `DelayFrame` and the
//! cursor loop polls once a frame either way; what is missing is the picture, not the cadence.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::text_boxes::TextBoxId;
use crate::modes::place_string::ligature;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::{MenuInput, UNFILLED_CURSOR};
use crate::systems::hp_bar::{draw_hp, HpBarType};
use crate::systems::print_num::{print_number, NumberFormat};

/// `<LV>`, the two-tile ":L" the level is printed after.
const LEVEL: u8 = 0x6E;
/// The first entry's name, and two rows to the next.
const NAME_X: usize = 3;
const STATUS_X: usize = 17;
const LEVEL_X: usize = 13;
const BAR_X: usize = 4;

/// `wPartyMenuTypeOrMessageID` below `FIRST_PARTY_MENU_TEXT_ID`: which screen this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum PartyMenuType {
    Normal,
    UseItem,
    Battle,
    TmHm,
    SwapMons,
    EvoStone,
}

impl PartyMenuType {
    /// `PartyMenuMessagePointers`, where the stone screen and the item screen share a line.
    fn message(self) -> &'static str {
        match self {
            Self::Normal => "_PartyMenuNormalText",
            Self::UseItem | Self::EvoStone => "_PartyMenuItemUseText",
            Self::Battle => "_PartyMenuBattleText",
            Self::TmHm => "_PartyMenuUseTMText",
            Self::SwapMons => "_PartyMenuSwapMonText",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyMenu {
    kind: PartyMenuType,
    input: MenuInput,
    /// The `Delay3` after the prompt is printed.
    opening: u8,
    /// `wMenuItemToSwap`, counting from 1. The same WRAM byte the item list marks its swaps with,
    /// and never live at the same time as that one.
    to_swap: u8,
    /// `DisplayPartyMenu` clears the screen and loads the icons; `GoBackToPartyMenu` does neither.
    cold: bool,
}

impl PartyMenu {
    pub fn new(kind: PartyMenuType) -> Self {
        let input = MenuInput::new(0, 0, (0, 1), Joypad::A | Joypad::B);
        Self { kind, input, opening: 3, to_swap: 0, cold: true }
    }

    /// `.choseSwitch`: the mon in `slot` is marked, and the next one chosen changes places with it.
    /// The caller is the one that refuses this for a party of one.
    pub fn swapping(slot: u8) -> Self {
        Self { to_swap: slot + 1, cold: false, ..Self::new(PartyMenuType::SwapMons) }
    }

    /// `GoBackToPartyMenu`: the list is already on screen, so it is not cleared and redrawn.
    pub fn again(kind: PartyMenuType) -> Self {
        Self { cold: false, ..Self::new(kind) }
    }

    pub fn armed(&self) -> Option<u8> {
        (self.to_swap != 0).then(|| self.to_swap - 1)
    }

    /// `SwitchPartyMon_ClearGfx`: both rows of one mon's entry. Not housekeeping: `PrintNumber`
    /// steps over a leading blank rather than writing one, so without this the `1` of a departing
    /// `119` stays put and the `35` that replaces it reads as `135`.
    fn clear_entry(ui: &mut UiSurface, slot: usize) {
        ui.fill(0, 2 * slot, SCREEN_TILES_X, 2, UiSurface::BLANK);
    }

    /// `ErasePartyMenuCursors`: the six cursor slots, two rows apart. Note these are the rows the
    /// cursor uses, not the rows the names are on, so a swap marker is not among them.
    fn erase_cursors(ui: &mut UiSurface) {
        for slot in 0..6 {
            ui.set(0, 1 + 2 * slot, UiSurface::BLANK);
        }
    }

    pub fn selected(&self) -> u8 {
        self.input.current
    }

    /// `PrintStatusCondition`: a fainted mon reads `FNT` whatever its status byte says.
    fn status_text(status: u8, hp: u16) -> Option<&'static str> {
        if hp == 0 {
            return Some("FNT");
        }
        match status {
            s if s & 1 << 3 != 0 => Some("PSN"),
            s if s & 1 << 4 != 0 => Some("BRN"),
            s if s & 1 << 5 != 0 => Some("FRZ"),
            s if s & 1 << 6 != 0 => Some("PAR"),
            s if s & 0b111 != 0 => Some("SLP"),
            _ => None,
        }
    }

    /// `PrintLevel`. At level 100 the number is three digits and writes over the `:L` itself.
    fn print_level(ui: &mut UiSurface, x: usize, y: usize, level: u8) {
        ui.set(x, y, LEVEL);
        let (at, digits) = if level >= 100 { (x, 3) } else { (x + 1, 2) };
        let format = NumberFormat { digits, left_align: true, leading_zeroes: false };
        print_number(ui, y * SCREEN_TILES_X + at, level as u32, format);
    }

    /// The prompt under the list. It prints whole because the cartridge sets `BIT_NO_TEXT_DELAY`
    /// around it, so there is nothing here for a `PlaceString` to pace.
    fn print_message(&self, ctx: &mut Ctx) {
        TextBoxId::MessageBox.draw(&mut ctx.screen.ui);
        let script = poke_core::text_script::far_text(self.kind.message())
            .expect("the party menu's prompts are in the cartridge");
        let mut at = 14 * SCREEN_TILES_X + 1;
        for command in script {
            let poke_core::text_script::TextCommand::Text(bytes) = command else { continue };
            for byte in bytes {
                match byte {
                    0x50 | 0x57 | 0x58 => return,
                    // `<LINE>` drops to the box's second line, `<NEXT>` two rows on.
                    0x4F => at = 16 * SCREEN_TILES_X + 1,
                    0x4E => at += 2 * SCREEN_TILES_X,
                    letter => {
                        // `#` is one byte in the cartridge's text and four tiles on the screen.
                        for &tile in ligature(letter).unwrap_or(std::slice::from_ref(&letter)) {
                            ctx.screen.ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
                            at += 1;
                        }
                    }
                }
            }
        }
    }

    /// `RedrawPartyMenu_`'s loop: a name, a status, a bar and a level, two rows apart.
    fn draw_rows(&self, ctx: &mut Ctx) {
        let rows: Vec<(Vec<u8>, u8, u16, u16, u8)> = ctx.world.party.iter()
            .map(|held| (held.nick.clone(), held.mon.mon.status, held.mon.mon.hp, held.mon.stats[0], held.mon.level))
            .collect();
        for (row, (nick, status, hp, max_hp, level)) in rows.into_iter().enumerate() {
            let y = row * 2;
            ctx.screen.ui.place(NAME_X, y, &nick);
            // `RedrawPartyMenu_` draws this, but only on a pass that also redraws the list, and the
            // only state that enables it is the one that skips the list. No path reaches it.
            if self.to_swap as usize == row + 1 {
                ctx.screen.ui.set(0, y, UNFILLED_CURSOR);
            }
            if let Some(text) = Self::status_text(status, hp) {
                let bytes = poke_core::charmap::encode(text).expect("a status encodes");
                ctx.screen.ui.place(STATUS_X, y, &bytes);
            }
            draw_hp(&mut ctx.screen.ui, (y + 1) * SCREEN_TILES_X + BAR_X, hp, max_hp, true, HpBarType::PartyMenu);
            Self::print_level(&mut ctx.screen.ui, LEVEL_X, y, level);
        }
    }
}

impl PartyMenu {
    /// `HandlePartyMenuInput`'s swapping branch. Either way the menu stays up, because the swap is
    /// a thing done *to* the list rather than an answer to whoever opened it.
    fn swap_or_cancel(&mut self, keys: Joypad, ctx: &mut Ctx) -> Transition {
        let first = self.to_swap as usize - 1;
        let second = self.input.current as usize;
        self.to_swap = 0;
        self.kind = PartyMenuType::Normal;
        if keys.contains(Joypad::B) {
            Self::erase_cursors(&mut ctx.screen.ui);
        } else {
            // A mon cannot change places with itself; the cartridge checks and returns, but still
            // clears and redraws both rows either way.
            if first != second {
                ctx.world.party.swap(first, second);
            }
            Self::clear_entry(&mut ctx.screen.ui, first);
            Self::clear_entry(&mut ctx.screen.ui, second);
        }
        self.draw_rows(ctx);
        self.print_message(ctx);
        self.opening = 3;
        self.input.call(ctx);
        Transition::Stay
    }
}

impl ModeUpdate for PartyMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        if self.cold {
            ctx.screen.ui.fill(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y, UiSurface::BLANK);
        }
        ctx.screen.tiles.load_hp_bar_and_status_tiles();
        let party = ctx.world.party.len();
        self.input = MenuInput::new(
            ctx.menu.party_and_bills,
            party.saturating_sub(1) as u8,
            (0, 1),
            Joypad::A | Joypad::B,
        );
        // `wMenuWrappingEnabled`: the party menu is the one that runs off either end.
        self.input.wrapping = true;
        ctx.menu.last_item = ctx.menu.party_and_bills;
        // The swap prompt goes up over the list that is already there, which is why arming one
        // never draws the marker beside it.
        if self.kind != PartyMenuType::SwapMons {
            self.draw_rows(ctx);
        }
        self.print_message(ctx);
        self.opening = 3;
        self.input.call(ctx);
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        if self.opening > 0 {
            self.opening -= 1;
            if self.opening > 0 {
                return Transition::Stay;
            }
        }
        let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
        ctx.menu.party_and_bills = self.input.current;
        if self.to_swap != 0 {
            return self.swap_or_cancel(keys, ctx);
        }
        ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
        if keys.contains(Joypad::B) || ctx.world.party.is_empty() {
            return Transition::Pop(Outcome::Cancelled);
        }
        ctx.menu.chosen_item = self.input.current;
        Transition::Pop(Outcome::Chosen(self.input.current))
    }

    fn status(&self) -> Status {
        if self.opening == 0 && self.input.is_polling() {
            Status::Waiting(Decision::PartyMenu)
        } else {
            Status::Busy
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::species::PokemonSpecies;
    use crate::command::{Command, Reply};
    use crate::mode::Mode;
    use crate::party::{Named, PartyMon};
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const CURSOR: u8 = 0xED;

    fn mon(species: PokemonSpecies, level: u8, nick: &str) -> Named<PartyMon> {
        let mon = new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        Named { mon, ot: encode("RED").unwrap(), nick: encode(nick).unwrap() }
    }

    fn game(party: Vec<Named<PartyMon>>) -> Game {
        let world = World { party, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal)));
        game
    }

    fn two() -> Vec<Named<PartyMon>> {
        vec![mon(PokemonSpecies::Pidgey, 7, "BIRD"), mon(PokemonSpecies::Rattata, 100, "RAT")]
    }

    #[test]
    fn the_prompt_spells_the_poke_byte_out() {
        let mut game = game(two());
        until_waiting(&mut game);
        assert_eq!(game.ui().row(14)[1..18], encode("Choose a POKéMON.").unwrap()[..],
            "`#` is one byte in the cartridge's text and four tiles here");
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::PartyMenu) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the party menu never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn cursor_row(game: &Game) -> Option<usize> {
        (0..18).find(|&y| game.ui().get(0, y) == CURSOR)
    }

    #[test]
    fn a_name_a_bar_and_a_level_two_rows_apart() {
        let mut game = game(two());
        until_waiting(&mut game);
        assert_eq!(game.ui().row(0)[3..7], encode("BIRD").unwrap()[..]);
        assert_eq!(game.ui().row(2)[3..6], encode("RAT").unwrap()[..]);
        assert_eq!(game.ui().row(1)[4..6], [0x71, 0x62], "HP: on the row under the name");
        assert_eq!(game.ui().row(3)[4..6], [0x71, 0x62]);
    }

    /// `wTopMenuItemY` is 1 while the names are on row 0, so the cursor sits beside the HP bar
    /// rather than the name. The lockstep is what settles this against the cartridge.
    #[test]
    fn the_cursor_starts_a_row_below_the_first_name() {
        let mut game = game(two());
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(1));
    }

    /// `wMenuWrappingEnabled`: the party menu is the one that runs off either end.
    #[test]
    fn the_cursor_wraps_at_both_ends() {
        let mut game = game(two());
        until_waiting(&mut game);
        press(&mut game, Joypad::UP);
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(3), "up from the first lands on the last");
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(cursor_row(&game), Some(1));
    }

    #[test]
    fn a_fainted_mon_reads_fnt_whatever_its_status_byte_says() {
        let mut party = two();
        party[0].mon.mon.hp = 0;
        party[0].mon.mon.status = 1 << 3;
        let mut game = game(party);
        until_waiting(&mut game);
        assert_eq!(game.ui().row(0)[17..20], encode("FNT").unwrap()[..]);
    }

    #[test]
    fn a_status_is_printed_where_one_is_set() {
        let mut party = two();
        party[0].mon.mon.status = 1 << 6;
        let mut game = game(party);
        until_waiting(&mut game);
        assert_eq!(game.ui().row(0)[17..20], encode("PAR").unwrap()[..]);
    }

    /// At 100 the number is three digits and writes over the `:L` itself.
    #[test]
    fn a_level_of_a_hundred_covers_the_lv_tile() {
        let mut game = game(two());
        until_waiting(&mut game);
        assert_eq!(game.ui().get(13, 0), LEVEL, "level 7 keeps it");
        assert_eq!(game.ui().row(2)[13..16], encode("100").unwrap()[..], "level 100 does not");
    }

    #[test]
    fn b_backs_out_and_a_chooses_the_slot() {
        let mut backed_out = game(two());
        until_waiting(&mut backed_out);
        press(&mut backed_out, Joypad::B);
        assert!(backed_out.modes().is_empty());

        let mut chose = game(two());
        until_waiting(&mut chose);
        press(&mut chose, Joypad::DOWN);
        until_waiting(&mut chose);
        press(&mut chose, Joypad::A);
        assert!(chose.modes().is_empty());
        assert_eq!(chose.menu().chosen_item, 1);
        assert_eq!(chose.menu().party_and_bills, 1, "and it reopens there");
    }

    #[test]
    fn a_command_walks_to_the_slot_it_names() {
        let mut game = game(two());
        until_waiting(&mut game);
        assert_eq!(game.frame(Input::Command(Command::ChooseOption(1))).reply, Some(Reply::Accepted));
        let mut events = vec![];
        for _ in 0..60 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::ChooseOption(1))]);
        assert_eq!(game.menu().chosen_item, 1);
    }

    #[test]
    fn a_save_mid_menu_resumes_identically() {
        let mut whole = game(two());
        until_waiting(&mut whole);
        whole.frame(Input::Buttons(Joypad::DOWN));
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..40 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
    }
}

#[cfg(test)]
mod swapping {
    use poke_core::charmap::encode;
    use poke_core::species::PokemonSpecies;
    use crate::mode::Mode;
    use crate::party::{Named, PartyMon};
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    fn mon(species: PokemonSpecies, nick: &str) -> Named<PartyMon> {
        let mon = new_party_mon(species, 10, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        Named { mon, ot: Vec::new(), nick: encode(nick).unwrap() }
    }

    fn armed(slot: u8) -> Game {
        let party = vec![mon(PokemonSpecies::Pidgey, "BIRD"), mon(PokemonSpecies::Rattata, "RAT"),
                         mon(PokemonSpecies::Ivysaur, "IVY")];
        let mut game = Game::new(World { party, ..World::default() }, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::PartyMenu(PartyMenu::swapping(slot)));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::PartyMenu) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the party menu never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn nicks(game: &Game) -> Vec<Vec<u8>> {
        game.world().party.iter().map(|held| held.nick.clone()).collect()
    }

    #[test]
    fn the_next_mon_chosen_changes_places_with_the_marked_one() {
        let mut game = armed(0);
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        press(&mut game, Joypad::A);
        until_waiting(&mut game);
        assert_eq!(nicks(&game), [encode("RAT").unwrap(), encode("BIRD").unwrap(), encode("IVY").unwrap()]);
        assert!(!game.modes().is_empty(), "the menu stays up afterwards");
        assert_eq!(game.ui().row(0)[3..6], encode("RAT").unwrap()[..], "and the list is redrawn");
    }

    /// A mon cannot change places with itself, and choosing it again simply ends the swap.
    #[test]
    fn a_mon_chosen_against_itself_leaves_the_party_alone() {
        let mut game = armed(0);
        until_waiting(&mut game);
        press(&mut game, Joypad::A);
        until_waiting(&mut game);
        assert_eq!(nicks(&game), [encode("BIRD").unwrap(), encode("RAT").unwrap(), encode("IVY").unwrap()]);
        let menu = match game.modes().last() {
            Some(Mode::PartyMenu(menu)) => menu,
            _ => panic!("no party menu"),
        };
        assert_eq!(menu.armed(), None, "and the marking is dropped either way");
    }

    #[test]
    fn b_drops_the_swap_and_leaves_the_menu_open() {
        let mut game = armed(1);
        until_waiting(&mut game);
        press(&mut game, Joypad::B);
        until_waiting(&mut game);
        assert_eq!(nicks(&game), [encode("BIRD").unwrap(), encode("RAT").unwrap(), encode("IVY").unwrap()]);
        assert!(!game.modes().is_empty(), "B here does not close the party menu");
    }

    /// Arming puts the prompt up over the list that is already drawn, which is why the marker the
    /// redraw would place never appears.
    #[test]
    fn arming_prints_the_prompt_without_redrawing_the_list() {
        let mut game = armed(0);
        until_waiting(&mut game);
        assert!(game.ui().row(0)[3..7].iter().all(|&tile| tile == UiSurface::BLANK),
            "the list was not drawn by the swap prompt");
        assert_eq!(game.ui().get(0, 0), UiSurface::BLANK, "and so no marker either");
    }
    /// The trap `SwitchPartyMon_ClearGfx` exists for: a two-digit HP moving into a slot a
    /// three-digit one has left.
    #[test]
    fn a_shorter_number_does_not_wear_the_head_of_a_longer_one() {
        let mut party = vec![mon(PokemonSpecies::Pidgey, "BIRD"), mon(PokemonSpecies::Rattata, "RAT")];
        party[0].mon.mon.hp = 119;
        party[0].mon.stats[0] = 119;
        party[1].mon.mon.hp = 35;
        party[1].mon.stats[0] = 35;
        let mut game = Game::new(World { party, ..World::default() }, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::PartyMenu(PartyMenu::swapping(0)));
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        press(&mut game, Joypad::A);
        until_waiting(&mut game);
        assert_eq!(game.ui().get(13, 1), UiSurface::BLANK, "no leading 1 left from the 119");
        assert_eq!(game.ui().row(1)[14..16], encode("35").unwrap()[..]);
    }
}
