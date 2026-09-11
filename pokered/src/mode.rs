//! One frame updates only the top mode. A transition applies at once: a pushed mode's `enter` and
//! a parent's `resume` run in the same frame, and frame counts in a mode's spec assume this.

use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::Screen;
use crate::input::Pad;
use crate::modes::list_menu::ListMenu;
use crate::modes::menu_input::CursorMemory;
use crate::modes::text_box::TextBox;
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
}

macro_rules! each_mode {
    ($mode:expr, $inner:ident => $body:expr) => {
        match $mode {
            Mode::TextBox($inner) => $body,
            Mode::ListMenu($inner) => $body,
        }
    };
}

impl ModeUpdate for Mode {
    fn enter(&mut self, ctx: &mut Ctx) {
        each_mode!(self, m => m.enter(ctx))
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
