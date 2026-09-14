//! One frame updates only the top mode. A transition applies at once: a pushed mode's `enter` and
//! a parent's `resume` run in the same frame, and frame counts in a mode's spec assume this.

use serde::{Deserialize, Serialize};
use crate::audio::engine::AudioEngine;
use crate::command::Decision;
use crate::gfx::Screen;
use crate::input::Pad;
use crate::modes::buy_sell_quit::BuySellQuitMenu;
use crate::modes::evolution::Evolution;
use crate::modes::field_move_menu::FieldMoveMenu;
use crate::modes::item_menu::ItemMenu;
use crate::modes::learn_move::LearnMove;
use crate::modes::list_menu::ListMenu;
use crate::modes::menu_input::CursorMemory;
use crate::modes::move_selection_menu::MoveSelectionMenu;
use crate::modes::naming_screen::NamingScreen;
use crate::modes::option_menu::OptionMenu;
use crate::modes::party_menu::PartyMenu;
use crate::modes::pokedex::PokedexMenu;
use crate::modes::pokemart::Pokemart;
use crate::modes::pokemon_menu::PokemonMenu;
use crate::modes::quantity_menu::QuantityMenu;
use crate::modes::start_menu::StartMenu;
use crate::modes::status_screen::StatusScreen;
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::TwoOptionMenu;
use crate::modes::use_item::UseItem;
use crate::rng::GameRng;
use crate::world::World;
use crate::{Event, Pacing};

pub struct Ctx<'a> {
    pub world: &'a mut World,
    pub pad: &'a mut Pad,
    pub rng: &'a mut GameRng,
    /// `hFrameCounter`.
    pub frame_counter: &'a mut u8,
    pub screen: &'a mut Screen,
    pub menu: &'a mut CursorMemory,
    pub audio: &'a mut AudioEngine,
    pub events: &'a mut Vec<Event>,
    pub pacing: Pacing,
}

pub enum Transition {
    Stay,
    Push(Mode),
    Pop(Outcome),
    Replace(Mode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Done,
    Chosen(u8),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Printing, animating or running a command.
    Busy,
    /// Free to walk.
    Idle,
    Waiting(Decision),
}

pub trait ModeUpdate {
    fn enter(&mut self, _ctx: &mut Ctx) {}
    /// Runs once the mode is on the stack, for a mode whose first act is to push another: the
    /// cartridge calls straight into it in the same frame.
    fn open(&mut self, _ctx: &mut Ctx) -> Transition {
        Transition::Stay
    }
    fn update(&mut self, ctx: &mut Ctx) -> Transition;
    fn resume(&mut self, _outcome: Outcome, _ctx: &mut Ctx) -> Transition {
        Transition::Stay
    }
    fn status(&self) -> Status;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    TextBox(TextBox),
    ListMenu(ListMenu),
    StartMenu(StartMenu),
    OptionMenu(OptionMenu),
    TwoOptionMenu(TwoOptionMenu),
    BuySellQuitMenu(BuySellQuitMenu),
    PartyMenu(PartyMenu),
    NamingScreen(NamingScreen),
    FieldMoveMenu(FieldMoveMenu),
    Pokedex(PokedexMenu),
    StatusScreen(StatusScreen),
    LearnMove(LearnMove),
    Evolution(Evolution),
    PokemonMenu(PokemonMenu),
    ItemMenu(ItemMenu),
    QuantityMenu(QuantityMenu),
    UseItem(UseItem),
    MoveSelectionMenu(MoveSelectionMenu),
    Pokemart(Pokemart),
}

macro_rules! each_mode {
    ($mode:expr, $inner:ident => $body:expr) => {
        match $mode {
            Mode::TextBox($inner) => $body,
            Mode::ListMenu($inner) => $body,
            Mode::StartMenu($inner) => $body,
            Mode::OptionMenu($inner) => $body,
            Mode::TwoOptionMenu($inner) => $body,
            Mode::BuySellQuitMenu($inner) => $body,
            Mode::PartyMenu($inner) => $body,
            Mode::NamingScreen($inner) => $body,
            Mode::FieldMoveMenu($inner) => $body,
            Mode::Pokedex($inner) => $body,
            Mode::StatusScreen($inner) => $body,
            Mode::LearnMove($inner) => $body,
            Mode::Evolution($inner) => $body,
            Mode::PokemonMenu($inner) => $body,
            Mode::ItemMenu($inner) => $body,
            Mode::QuantityMenu($inner) => $body,
            Mode::UseItem($inner) => $body,
            Mode::MoveSelectionMenu($inner) => $body,
            Mode::Pokemart($inner) => $body,
        }
    };
}

impl ModeUpdate for Mode {
    fn enter(&mut self, ctx: &mut Ctx) {
        each_mode!(self, m => m.enter(ctx))
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        each_mode!(self, m => m.open(ctx))
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        each_mode!(self, m => m.update(ctx))
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        each_mode!(self, m => m.resume(outcome, ctx))
    }

    fn status(&self) -> Status {
        each_mode!(self, m => m.status())
    }
}
