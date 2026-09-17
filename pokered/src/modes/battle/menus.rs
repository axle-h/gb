//! The battle's two menus of its own: FIGHT/PKMN/ITEM/RUN, and the moves.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::modes::menu_input::MenuInput;

/// The battle menu's rows as `.handleMenuSelection` numbers them once ITEM and PKMN are swapped back.
pub const FIGHT: u8 = 0;
pub const ITEM: u8 = 1;
pub const PKMN: u8 = 2;
pub const RUN: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Menu {
    /// `DisplayBattleMenu`'s two columns, each a menu of two rows: FIGHT and ITEM on the left, PKMN
    /// and RUN on the right.
    Battle { input: MenuInput, right: bool },
    /// `MoveSelectionMenu`'s regular menu, whose items count from the box's top border.
    Moves { input: MenuInput },
    /// `MoveSelectionMenu`'s Mimic menu over the enemy's moves: no B, no SELECT.
    Mimic { input: MenuInput },
    /// SWITCH, STATS and CANCEL for the party mon in `slot`.
    SwitchStatsCancel { input: MenuInput, slot: u8 },
}

impl Menu {
    pub fn battle(right: bool, current: u8) -> Self {
        let watched = if right { Joypad::LEFT | Joypad::A } else { Joypad::RIGHT | Joypad::A };
        let x = if right { 15 } else { 9 };
        Menu::Battle { input: MenuInput::new(current, 1, (x, 14), watched), right }
    }

    /// The Safari Zone's columns: BALL and THROW ROCK, BAIT and RUN.
    pub fn safari(right: bool, current: u8) -> Self {
        let watched = if right { Joypad::LEFT | Joypad::A } else { Joypad::RIGHT | Joypad::A };
        let x = if right { 13 } else { 1 };
        Menu::Battle { input: MenuInput::new(current, 1, (x, 14), watched), right }
    }

    pub fn moves(current: u8, max: u8) -> Self {
        let watched = Joypad::A | Joypad::B | Joypad::SELECT | Joypad::UP | Joypad::DOWN;
        let mut input = MenuInput::new(current, max, (5, 12), watched);
        input.single_spaced = true;
        Menu::Moves { input }
    }

    pub fn mimic(max: u8) -> Self {
        let mut input = MenuInput::new(1, max, (1, 7), Joypad::A | Joypad::UP | Joypad::DOWN);
        input.single_spaced = true;
        Menu::Mimic { input }
    }

    pub fn switch_stats_cancel(slot: u8) -> Self {
        Menu::SwitchStatsCancel { input: MenuInput::new(0, 2, (12, 12), Joypad::A | Joypad::B), slot }
    }

    fn input(&self) -> &MenuInput {
        match self {
            Menu::Battle { input, .. } | Menu::Moves { input } | Menu::Mimic { input } | Menu::SwitchStatsCancel { input, .. } => input,
        }
    }

    fn input_mut(&mut self) -> &mut MenuInput {
        match self {
            Menu::Battle { input, .. } | Menu::Moves { input } | Menu::Mimic { input } | Menu::SwitchStatsCancel { input, .. } => input,
        }
    }

    pub fn call(&mut self, ctx: &mut Ctx) {
        self.input_mut().call(ctx);
    }

    pub fn update(&mut self, ctx: &mut Ctx) -> Option<Joypad> {
        self.input_mut().update(ctx)
    }

    pub fn is_polling(&self) -> bool {
        self.input().is_polling()
    }

    pub fn decision(&self) -> Decision {
        match self {
            Menu::Battle { .. } => Decision::BattleMenu,
            Menu::Moves { .. } => Decision::BattleMoves,
            Menu::Mimic { .. } => Decision::MimicMove,
            Menu::SwitchStatsCancel { .. } => Decision::SwitchStatsCancel,
        }
    }

    /// The row under the cursor: the battle menu's number, or the move's slot from 0.
    pub fn selected(&self) -> u8 {
        match self {
            Menu::Battle { input, right } => input.current + if *right { 2 } else { 0 },
            Menu::Moves { input } | Menu::Mimic { input } => input.current.saturating_sub(1),
            Menu::SwitchStatsCancel { input, .. } => input.current,
        }
    }

    pub fn current(&self) -> u8 {
        self.input().current
    }

    pub fn set_current(&mut self, current: u8) {
        self.input_mut().current = current;
    }

    /// The press that brings the cursor a step nearer `target`, or A on it.
    pub fn press_toward(&self, target: u8) -> Joypad {
        match self {
            Menu::Battle { input, right } => {
                let (column, row) = (target >= 2, target % 2);
                if column != *right {
                    if column { Joypad::RIGHT } else { Joypad::LEFT }
                } else if row != input.current {
                    if row > input.current { Joypad::DOWN } else { Joypad::UP }
                } else {
                    Joypad::A
                }
            }
            Menu::SwitchStatsCancel { input, .. } => match target.cmp(&input.current) {
                std::cmp::Ordering::Less => Joypad::UP,
                std::cmp::Ordering::Greater => Joypad::DOWN,
                std::cmp::Ordering::Equal => Joypad::A,
            },
            Menu::Moves { input } | Menu::Mimic { input } => match (target + 1).cmp(&input.current) {
                std::cmp::Ordering::Less => Joypad::UP,
                std::cmp::Ordering::Greater => Joypad::DOWN,
                std::cmp::Ordering::Equal => Joypad::A,
            },
        }
    }
}
