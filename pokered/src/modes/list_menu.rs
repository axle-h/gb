//! `DisplayListMenuID` for `ITEMLISTMENU`: a scrolling list of items and quantities, four rows at a
//! time, with a cursor over three of them and `CANCEL` after the last.

use poke_core::item::{self, ItemId};
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::modes::menu_input::MenuInput;
use crate::systems::print_num::{print_number, NumberFormat};

const TIMES: u8 = 0xF1;
const DOWN_ARROW: u8 = 0xEE;
const UNFILLED_CURSOR: u8 = 0xEC;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListMenu {
    entries: Vec<(ItemId, u8)>,
    /// `wListScrollOffset`.
    scroll: u8,
    input: MenuInput,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// The `DelayFrames 10` after drawing the box.
    Opening(u8),
    /// The `Delay3` after printing the entries.
    Printed(u8),
    Input,
}

impl ListMenu {
    /// `current` and `scroll` are the caller's `wCurrentMenuItem` and `wListScrollOffset`.
    pub fn items(entries: Vec<(ItemId, u8)>, current: u8, scroll: u8) -> Self {
        let max = if entries.len() < 2 { 1 } else { 2 };
        let mut input = MenuInput::new(current, max, (5, 4), Joypad::A | Joypad::B | Joypad::SELECT);
        input.return_at_ends = true;
        Self { entries, scroll, input, phase: Phase::Opening(10) }
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
                place(ui, at, &poke_core::charmap::encode("CANCEL").unwrap());
                return;
            };
            place(ui, at, &item::name(item));
            if !(item.is_key_item() || item.is_hm()) {
                let times = at + SCREEN_TILES_X + 8;
                ui.set(times % SCREEN_TILES_X, times / SCREEN_TILES_X, TIMES);
                print_number(ui, times + 1, quantity as u32, NumberFormat { digits: 2, ..NumberFormat::default() });
            }
            at += 2 * SCREEN_TILES_X;
        }
        ui.set((at - 8) % SCREEN_TILES_X, (at - 8) / SCREEN_TILES_X, DOWN_ARROW);
    }

    /// `ExitListMenu`, or the chosen entry's return; either way `hJoy7` and the text delay reset.
    fn close(&self, ctx: &mut Ctx, outcome: Outcome) -> Transition {
        ctx.pad.repeat_held = false;
        ctx.world.no_text_delay = false;
        Transition::Pop(outcome)
    }
}

fn place(ui: &mut UiSurface, at: usize, bytes: &[u8]) {
    for (i, &byte) in bytes.iter().enumerate() {
        ui.set((at + i) % SCREEN_TILES_X, (at + i) / SCREEN_TILES_X, byte);
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
            Phase::Input => {
                let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
                self.input.place_cursor(&mut ctx.screen.ui, ctx.menu);
                if keys.contains(Joypad::A) {
                    let at = self.input.cursor_at;
                    ctx.screen.ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, UNFILLED_CURSOR);
                    let chosen = self.selected();
                    let outcome = if chosen < self.entries.len() { Outcome::Chosen(chosen as u8) } else { Outcome::Cancelled };
                    return self.close(ctx, outcome);
                }
                if keys.contains(Joypad::B) {
                    return self.close(ctx, Outcome::Cancelled);
                }
                if keys.contains(Joypad::DOWN) {
                    if self.entries.len() >= self.scroll as usize + 3 {
                        self.scroll += 1;
                    }
                } else if !keys.contains(Joypad::SELECT) && self.scroll > 0 {
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
