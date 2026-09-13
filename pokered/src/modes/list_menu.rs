//! `DisplayListMenuID` for `ITEMLISTMENU`: a scrolling list of items and quantities, four rows at a
//! time, with a cursor over three of them and `CANCEL` after the last.

use poke_core::item::{self, ItemId};
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::{MenuInput, UNFILLED_CURSOR};
use crate::systems::print_num::{print_number, NumberFormat};

const TIMES: u8 = 0xF1;
const DOWN_ARROW: u8 = 0xEE;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListMenu {
    entries: Vec<(ItemId, u8)>,
    /// `wListScrollOffset`.
    scroll: u8,
    input: MenuInput,
    phase: Phase,
    /// `wMenuItemToSwap`, counting from 1 over the whole list. Zeroed as the menu opens and as it
    /// closes, so it never outlives the list it marks.
    to_swap: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// The `DelayFrames 10` after drawing the box.
    Opening(u8),
    /// The `Delay3` after printing the entries.
    Printed(u8),
    /// `HandleItemListSwapping`'s `DelayFrames 20`, which passes before the list is drawn again.
    Swapping(u8),
    Input,
}

impl ListMenu {
    /// `current` and `scroll` are the caller's `wCurrentMenuItem` and `wListScrollOffset`.
    pub fn items(entries: Vec<(ItemId, u8)>, current: u8, scroll: u8) -> Self {
        let max = if entries.len() < 2 { 1 } else { 2 };
        let mut input = MenuInput::new(current, max, (5, 4), Joypad::A | Joypad::B | Joypad::SELECT);
        input.return_at_ends = true;
        Self { entries, scroll, input, phase: Phase::Opening(10), to_swap: 0 }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The entry under the cursor, counting `CANCEL` as the one after the last.
    pub fn selected(&self) -> usize {
        self.scroll as usize + self.input.current as usize
    }

    /// `DisplayListMenuIDLoop`, up to its `Delay3`.
    fn redraw(&mut self, ctx: &mut Ctx) {
        self.print_entries(&mut ctx.screen.ui);
        self.phase = Phase::Printed(3);
    }

    /// `PrintListMenuEntries`.
    fn print_entries(&self, ui: &mut UiSurface) {
        ui.fill(5, 3, 14, 9, UiSurface::BLANK);
        let mut at = 4 * SCREEN_TILES_X + 6;
        for row in 0..4 {
            let Some(&(item, quantity)) = self.entries.get(self.scroll as usize + row) else {
                ui.place(at % SCREEN_TILES_X, at / SCREEN_TILES_X, &poke_core::charmap::encode("CANCEL").unwrap());
                return;
            };
            ui.place(at % SCREEN_TILES_X, at / SCREEN_TILES_X, &item::name(item));
            if self.to_swap != 0 && self.scroll as usize + row == self.to_swap as usize - 1 {
                ui.set(5, 4 + 2 * row, UNFILLED_CURSOR);
            }
            if !(item.is_key_item() || item.is_hm()) {
                let times = at + SCREEN_TILES_X + 8;
                ui.set(times % SCREEN_TILES_X, times / SCREEN_TILES_X, TIMES);
                print_number(ui, times + 1, quantity as u32, NumberFormat { digits: 2, ..NumberFormat::default() });
            }
            at += 2 * SCREEN_TILES_X;
        }
        ui.set((at - 8) % SCREEN_TILES_X, (at - 8) / SCREEN_TILES_X, DOWN_ARROW);
    }

    /// `HandleItemListSwapping`. Neither the `CANCEL` row nor an entry against itself can be
    /// swapped, and both go back to the loop without the twenty frames the real thing costs.
    fn select(&mut self, ctx: &mut Ctx) -> Transition {
        let chosen = self.selected();
        if chosen >= self.entries.len() {
            self.redraw(ctx);
            return Transition::Stay;
        }
        let numbered = chosen as u8 + 1;
        if self.to_swap == 0 {
            self.to_swap = numbered;
        } else if self.to_swap == numbered {
            self.redraw(ctx);
            return Transition::Stay;
        } else {
            let first = self.to_swap as usize - 1;
            self.to_swap = 0;
            self.swap(first, chosen);
        }
        self.phase = Phase::Swapping(20);
        Transition::Stay
    }

    /// The three ways two slots come together. The quantities are summed in one byte before the
    /// hundred is tested, so two slots that overflow it merge into one small one instead of capping.
    fn swap(&mut self, first: usize, second: usize) {
        if self.entries[first].0 != self.entries[second].0 {
            self.entries.swap(first, second);
            return;
        }
        let sum = self.entries[first].1.wrapping_add(self.entries[second].1);
        if sum >= 100 {
            // The donor keeps what ninety-nine leaves behind, which is one more than a hundred would.
            self.entries[first].1 = sum - 99;
            self.entries[second].1 = 99;
            return;
        }
        self.entries[second].1 = sum;
        self.entries.remove(first);
        self.scroll = 0;
        self.input.current = 0;
        self.input.max = if self.entries.len() < 2 { 1 } else { 2 };
    }

    /// `ExitListMenu`, or the chosen entry's return; either way `hJoy7` and the text delay reset.
    fn close(&self, ctx: &mut Ctx, outcome: Outcome) -> Transition {
        ctx.pad.repeat_held = false;
        ctx.world.no_text_delay = false;
        Transition::Pop(outcome)
    }
}

impl ModeUpdate for ListMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        ctx.pad.repeat_held = true;
        ctx.world.no_text_delay = true;
        ctx.screen.ui.text_box_border(4, 2, 14, 9);
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Opening(frames) | Phase::Printed(frames) if frames > 1 => {
                self.phase = match self.phase {
                    Phase::Opening(_) => Phase::Opening(frames - 1),
                    _ => Phase::Printed(frames - 1),
                };
                Transition::Stay
            }
            Phase::Opening(_) => {
                self.redraw(ctx);
                Transition::Stay
            }
            Phase::Printed(_) => {
                self.input.call(ctx);
                self.phase = Phase::Input;
                Transition::Stay
            }
            Phase::Swapping(frames) if frames > 1 => {
                self.phase = Phase::Swapping(frames - 1);
                Transition::Stay
            }
            Phase::Swapping(_) => {
                self.redraw(ctx);
                Transition::Stay
            }
            Phase::Input => {
                let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
                self.input.place_cursor(&mut ctx.screen.ui, ctx.menu);
                if keys.contains(Joypad::A) {
                    ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
                    let chosen = self.selected();
                    let outcome = if chosen < self.entries.len() { Outcome::Chosen(chosen as u8) } else { Outcome::Cancelled };
                    return self.close(ctx, outcome);
                }
                if keys.contains(Joypad::B) {
                    return self.close(ctx, Outcome::Cancelled);
                }
                if keys.contains(Joypad::SELECT) {
                    return self.select(ctx);
                }
                if keys.contains(Joypad::DOWN) {
                    if self.entries.len() >= self.scroll as usize + 3 {
                        self.scroll += 1;
                    }
                } else if self.scroll > 0 {
                    self.scroll -= 1;
                }
                self.redraw(ctx);
                Transition::Stay
            }
        }
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::Input if self.input.is_polling() => Status::Waiting(Decision::List),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::command::{Command, Refusal, Reply};
    use crate::mode::Mode;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const BAG: [(ItemId, u8); 6] = [
        (ItemId::Potion, 3), (ItemId::Antidote, 1), (ItemId::PokeBall, 12),
        (ItemId::Bicycle, 1), (ItemId::Hm01Cut, 1), (ItemId::Repel, 4),
    ];

    fn game(pacing: Pacing) -> Game {
        let mut game = Game::new(World::default(), GameRng::seeded(0), pacing);
        game.push(Mode::ListMenu(ListMenu::items(BAG.to_vec(), 0, 0)));
        game
    }

    fn text(game: &Game, y: usize) -> Vec<u8> {
        game.ui().row(y)[6..18].iter().copied().collect()
    }

    fn padded(s: &str) -> Vec<u8> {
        let mut bytes = encode(s).unwrap();
        bytes.resize(12, UiSurface::BLANK);
        bytes
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..100 {
            if game.status() == Status::Waiting(Decision::List) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the list never waited");
    }

    #[test]
    fn four_rows_a_down_arrow_and_quantities_except_for_key_items_and_hms() {
        let mut game = game(Pacing::Faithful);
        until_waiting(&mut game);
        assert_eq!(text(&game, 4), padded("POTION"));
        assert_eq!(text(&game, 5), padded("        × 3"), "PrintNumber leaves the tens place blank");
        assert_eq!(text(&game, 9), padded("        ×12"));
        assert_eq!(text(&game, 11), padded(""), "the Bicycle is a key item");
        assert_eq!(game.ui().get(18, 11), DOWN_ARROW);
        assert_eq!(game.ui().get(5, 4), 0xED, "the cursor");
    }

    #[test]
    fn choosing_an_entry_off_screen_scrolls_to_it_and_marks_it() {
        let mut game = game(Pacing::Faithful);
        until_waiting(&mut game);
        assert_eq!(game.frame(Input::Command(Command::ChooseListEntry(4))).reply, Some(Reply::Accepted));
        let mut events = vec![];
        while !game.modes().is_empty() {
            events.extend(game.frame(Input::None).events);
        }
        events.extend(game.frame(Input::None).events);
        assert_eq!(events, [Event::CommandDone(Command::ChooseListEntry(4))]);
        let marked = (4..12).step_by(2).find(|&y| game.ui().get(5, y) == UNFILLED_CURSOR).expect("a ▷");
        assert_eq!(text(&game, marked), padded("HM01"));
    }

    #[test]
    fn cancel_closes_without_marking_anything() {
        let mut game = game(Pacing::Faithful);
        until_waiting(&mut game);
        game.frame(Input::Command(Command::CancelList));
        while !game.modes().is_empty() {
            game.frame(Input::None);
        }
        assert!((4..12).all(|y| game.ui().get(5, y) != UNFILLED_CURSOR));
    }

    #[test]
    fn an_entry_past_the_end_is_refused() {
        let mut game = game(Pacing::Faithful);
        until_waiting(&mut game);
        let reply = game.frame(Input::Command(Command::ChooseListEntry(6))).reply;
        assert!(matches!(reply, Some(Reply::Refused(Refusal::Invalid(_)))), "{reply:?}");
    }

    #[test]
    fn scrolling_stops_with_cancel_on_the_last_cursor_row() {
        let mut game = game(Pacing::Instant);
        for _ in 0..12 {
            until_waiting(&mut game);
            game.frame(Input::Buttons(Joypad::DOWN));
            game.frame(Input::None);
        }
        until_waiting(&mut game);
        assert_eq!(text(&game, 8), padded("CANCEL"), "the fifth entry is the last above CANCEL");
        assert_eq!(game.ui().get(5, 8), 0xED);
    }

    #[test]
    fn a_save_mid_scroll_resumes_identically() {
        let mut whole = game(Pacing::Faithful);
        until_waiting(&mut whole);
        whole.frame(Input::Command(Command::ChooseListEntry(5)));
        for _ in 0..7 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..80 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
        assert!(restored.modes().is_empty());
    }
}

#[cfg(test)]
mod swapping {
    use poke_core::item::ItemId;
    use crate::input::Joypad;
    use crate::mode::{Mode, Status};
    use crate::command::Decision;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    fn game(bag: Vec<(ItemId, u8)>) -> Game {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ListMenu(ListMenu::items(bag, 0, 0)));
        game
    }

    fn until_waiting(game: &mut Game) {
        for _ in 0..200 {
            if game.status() == Status::Waiting(Decision::List) {
                return;
            }
            game.frame(Input::None);
        }
        panic!("the list never waited");
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn list(game: &Game) -> &ListMenu {
        match game.modes().last() {
            Some(Mode::ListMenu(list)) => list,
            _ => panic!("the list closed"),
        }
    }

    fn marker_row(game: &Game) -> Option<usize> {
        (0..18).find(|&y| game.ui().get(5, y) == UNFILLED_CURSOR)
    }

    const PAIR: [(ItemId, u8); 3] = [(ItemId::Potion, 3), (ItemId::Antidote, 1), (ItemId::Potion, 5)];

    /// The cursor and the marker are drawn on the same column, and the cursor goes on last, so an
    /// armed row shows its `▷` only once the cursor has moved off and put back what it covered.
    #[test]
    fn select_arms_the_row_it_is_pressed_on() {
        let mut game = game(PAIR.to_vec());
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(list(&game).to_swap, 1, "counting from one");
        assert_eq!(marker_row(&game), None, "the cursor is still covering it");
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(marker_row(&game), Some(4), "and now the first row shows it");
    }

    /// The marker is compared against a counter seeded with the scroll, so it stays with its item
    /// rather than with the row it was armed on.
    /// The marker is compared against a counter seeded with the scroll offset, so it stays with its
    /// entry and moves up the screen as the list scrolls under it.
    #[test]
    fn the_marker_follows_its_item_when_the_list_scrolls() {
        let bag = vec![(ItemId::Potion, 3), (ItemId::Antidote, 1), (ItemId::PokeBall, 12),
                       (ItemId::Repel, 4), (ItemId::Elixer, 2), (ItemId::Ether, 1)];
        let mut game = game(bag);
        until_waiting(&mut game);
        for _ in 0..2 {
            press(&mut game, Joypad::DOWN);
            until_waiting(&mut game);
        }
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(list(&game).to_swap, 3, "the third entry, counting from one");
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        assert_eq!(list(&game).scroll, 1, "the list scrolled under it");
        assert_eq!(marker_row(&game), Some(6), "so the marker moved up a row with its item");
    }

    #[test]
    fn neither_cancel_nor_an_entry_against_itself_can_be_swapped() {
        let mut game = game(PAIR.to_vec());
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(list(&game).to_swap, 1, "still armed, nothing swapped");
        assert_eq!(list(&game).entries[0], (ItemId::Potion, 3));
    }

    #[test]
    fn two_different_items_change_places() {
        let mut game = game(PAIR.to_vec());
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(list(&game).entries[..2], [(ItemId::Antidote, 1), (ItemId::Potion, 3)]);
        assert_eq!(list(&game).to_swap, 0, "and the marker is put away");
    }

    #[test]
    fn two_stacks_of_one_item_merge_and_close_the_gap() {
        let mut game = game(PAIR.to_vec());
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        for _ in 0..2 {
            press(&mut game, Joypad::DOWN);
            until_waiting(&mut game);
        }
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(list(&game).entries, [(ItemId::Antidote, 1), (ItemId::Potion, 8)]);
        assert_eq!((list(&game).scroll, list(&game).input.current), (0, 0), "back to the top");
    }

    /// Ninety-nine is the most a slot takes, and the donor keeps one more than a hundred would leave.
    #[test]
    fn a_sum_over_a_hundred_caps_the_second_slot_at_ninety_nine() {
        let mut game = game(vec![(ItemId::Potion, 60), (ItemId::Potion, 50)]);
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(list(&game).entries, [(ItemId::Potion, 11), (ItemId::Potion, 99)]);
    }

    /// The sum is one byte before the hundred is tested, so two big slots wrap and merge.
    #[test]
    fn a_sum_that_overflows_the_byte_merges_instead_of_capping() {
        let mut game = game(vec![(ItemId::Potion, 200), (ItemId::Potion, 100)]);
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        press(&mut game, Joypad::DOWN);
        until_waiting(&mut game);
        press(&mut game, Joypad::SELECT);
        until_waiting(&mut game);
        assert_eq!(list(&game).entries, [(ItemId::Potion, 44)], "300 in a byte is 44");
    }
}
