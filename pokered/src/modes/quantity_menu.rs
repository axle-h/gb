//! `DisplayChooseQuantityMenu`: how many, from 1 to a maximum, and in a mart what that many costs.
//!
//! Up counts up and Down counts down, each wrapping at the ends. The loop has no `DelayFrame`: it
//! spins on the joypad, so a press is read and the count redrawn in the frame it lands, and only new
//! presses count, however long one is held. A chooses and B backs out. In a mart the total is
//! `money::total_price`, halved for a sale, and printed against the box's right edge.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Ctx, ModeUpdate, Outcome, Status, Transition};
use crate::systems::money::total_price;
use crate::systems::print_num::{print_bcd, print_number, BcdFormat, NumberFormat};

const TIMES: u8 = 0xF1;

/// `hItemPrice` and `hHalveItemPrices`: what one costs, and whether this is a sale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    pub each: [u8; 3],
    pub halved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantityMenu {
    /// `wMaxItemQuantity`.
    max: u8,
    price: Option<Price>,
    /// `wItemQuantity`.
    quantity: u8,
}

impl QuantityMenu {
    pub fn new(max: u8, price: Option<Price>) -> Self {
        Self { max, price, quantity: 0 }
    }

    pub fn selected(&self) -> u8 {
        self.quantity
    }

    pub fn max(&self) -> u8 {
        self.max
    }

    /// `hMoney` as it stands: what the count on screen costs.
    pub fn total(&self) -> Option<[u8; 3]> {
        self.price.map(|price| total_price(price.each, self.quantity, price.halved))
    }

    /// The press that brings the count toward `target` the shorter way round, or backs out when
    /// there is none.
    pub fn press_toward(&self, target: Option<u8>) -> Joypad {
        let Some(target) = target else { return Joypad::B };
        if target == self.quantity || self.max == 0 {
            return Joypad::A;
        }
        let span = self.max as i16;
        let up = (target as i16 - self.quantity as i16).rem_euclid(span);
        if up <= span - up { Joypad::UP } else { Joypad::DOWN }
    }

    /// `.incrementQuantity`: past the maximum is 1, tested in a byte, so a maximum of 255 wraps
    /// through 0.
    fn increment(&mut self) {
        self.quantity = self.quantity.wrapping_add(1);
        if self.quantity == self.max.wrapping_add(1) {
            self.quantity = 1;
        }
    }

    /// `.decrementQuantity`: below 1 is the maximum.
    fn decrement(&mut self) {
        self.quantity = self.quantity.wrapping_sub(1);
        if self.quantity == 0 {
            self.quantity = self.max;
        }
    }

    /// `.handleNewQuantity`.
    fn print(&self, ui: &mut UiSurface) {
        let digits = NumberFormat { digits: 2, leading_zeroes: true, left_align: false };
        let row = 10 * SCREEN_TILES_X;
        let Some(total) = self.total() else {
            print_number(ui, row + 17, self.quantity as u32, digits);
            return;
        };
        // Six blanks, and the seventh tile of the price lands on whatever was there.
        ui.fill(12, 10, 6, 1, UiSurface::BLANK);
        let money = BcdFormat { skip_leading_zeroes: true, left_align: false, money_sign: true };
        print_bcd(ui, row + 12, &total, money);
        print_number(ui, row + 9, self.quantity as u32, digits);
    }
}

impl ModeUpdate for QuantityMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        let ui = &mut ctx.screen.ui;
        let x = if self.price.is_some() {
            ui.text_box_border(7, 9, 11, 1);
            8
        } else {
            ui.text_box_border(15, 9, 3, 1);
            16
        };
        ui.place(x, 10, &[TIMES, 0xF6, 0xF7]);
        self.quantity = 0;
        self.increment();
        self.print(ui);
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.pad.low_sensitivity(ctx.frame_counter);
        let pressed = ctx.pad.pressed;
        if pressed.contains(Joypad::A) {
            return Transition::Pop(Outcome::Chosen(self.quantity));
        }
        if pressed.contains(Joypad::B) {
            return Transition::Pop(Outcome::Cancelled);
        }
        if pressed.contains(Joypad::UP) {
            self.increment();
        } else if pressed.contains(Joypad::DOWN) {
            self.decrement();
        } else {
            return Transition::Stay;
        }
        self.print(&mut ctx.screen.ui);
        Transition::Stay
    }

    fn status(&self) -> Status {
        Status::Waiting(Decision::Quantity)
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

    fn game(max: u8, price: Option<Price>) -> Game {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::QuantityMenu(QuantityMenu::new(max, price)));
        game
    }

    fn press(game: &mut Game, button: Joypad) {
        game.frame(Input::Buttons(button));
        game.frame(Input::None);
    }

    fn menu(game: &Game) -> &QuantityMenu {
        match game.modes().last() {
            Some(Mode::QuantityMenu(menu)) => menu,
            _ => panic!("the menu closed"),
        }
    }

    #[test]
    fn it_opens_on_one_and_wraps_both_ways() {
        let mut game = game(3, None);
        assert_eq!(game.ui().row(10)[16..19], encode("×01").unwrap()[..]);
        press(&mut game, Joypad::DOWN);
        assert_eq!(menu(&game).selected(), 3, "below 1 is the maximum");
        assert_eq!(game.ui().row(10)[17..19], encode("03").unwrap()[..]);
        press(&mut game, Joypad::UP);
        assert_eq!(menu(&game).selected(), 1, "and past it is 1");
    }

    #[test]
    fn a_held_button_counts_once() {
        let mut game = game(9, None);
        for _ in 0..60 {
            game.frame(Input::Buttons(Joypad::UP));
        }
        assert_eq!(menu(&game).selected(), 2);
    }

    #[test]
    fn a_mart_prints_the_total_against_the_right_edge() {
        let mut game = game(99, Some(Price { each: [0, 3, 0], halved: false }));
        press(&mut game, Joypad::UP);
        press(&mut game, Joypad::UP);
        assert_eq!(game.ui().row(10)[8..19], encode("×03    ¥900").unwrap()[..]);
    }

    #[test]
    fn a_and_b_answer_it() {
        let mut chose = game(5, None);
        press(&mut chose, Joypad::UP);
        press(&mut chose, Joypad::A);
        assert!(chose.modes().is_empty());
        let mut backed_out = game(5, None);
        press(&mut backed_out, Joypad::B);
        assert!(backed_out.modes().is_empty());
    }

    #[test]
    fn a_command_counts_the_short_way_round() {
        let mut game = game(10, None);
        game.frame(Input::None);
        assert_eq!(game.frame(Input::Command(Command::ChooseQuantity(9))).reply, Some(Reply::Accepted));
        let mut events = vec![];
        let mut downs = 0;
        for _ in 0..20 {
            if let Some(Mode::QuantityMenu(menu)) = game.modes().last() && menu.selected() == 9 {
                break;
            }
            let frame = game.frame(Input::None);
            events.extend(frame.events);
            downs += 1;
        }
        assert!(downs <= 6, "1 to 9 is two presses down, not eight up: {downs} frames");
        for _ in 0..5 {
            events.extend(game.frame(Input::None).events);
        }
        assert_eq!(events, [Event::CommandDone(Command::ChooseQuantity(9))]);
        let refused = game_refusal();
        assert!(matches!(refused, Some(Reply::Refused(Refusal::Invalid(_)))), "{refused:?}");
    }

    fn game_refusal() -> Option<Reply> {
        let mut game = game(10, None);
        game.frame(Input::None);
        game.frame(Input::Command(Command::ChooseQuantity(11))).reply
    }

    #[test]
    fn a_save_mid_count_resumes_identically() {
        let mut whole = game(20, Some(Price { each: [0, 1, 0x50], halved: true }));
        press(&mut whole, Joypad::UP);
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..10 {
            let input = || if frame % 2 == 0 { Input::Buttons(Joypad::DOWN) } else { Input::None };
            whole.frame(input());
            restored.frame(input());
            assert_eq!(whole.ui(), restored.ui(), "frame {frame}");
        }
    }
}
