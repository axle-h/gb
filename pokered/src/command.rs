//! An accepted command is carried out by pressing buttons into the same modes a player's go to.

use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::mode::{Mode, ModeUpdate, Status};
use crate::world::World;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Answer a text box's `▼`.
    Advance,
    /// Pick a list menu's entry, by its index in the whole list.
    ChooseListEntry(u8),
    CancelList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    /// A text box waiting at its `▼`.
    Text,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Accepted,
    Refused(Refusal),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    Busy,
    Invalid(String),
}

pub enum Drive {
    Press(Joypad),
    Done,
    /// A wild battle mid-walk, say.
    Interrupted(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Executor {
    pub command: Command,
    driver: Driver,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Driver {
    Advance { answered: u32 },
    /// A press at a time, released on a frame the menu polls so that each is an edge.
    List { target: Option<u8>, released: bool },
}

impl Executor {
    pub fn accept(command: Command, modes: &[Mode], _world: &World) -> Result<Self, Refusal> {
        let driver = match &command {
            Command::Advance => match modes.last() {
                Some(Mode::TextBox(text)) if text.status() == Status::Waiting(Decision::Text) =>
                    Driver::Advance { answered: text.answered() },
                _ => return Err(Refusal::Invalid("no text box is waiting".into())),
            },
            Command::ChooseListEntry(_) | Command::CancelList => match modes.last() {
                Some(Mode::ListMenu(list)) if list.status() == Status::Waiting(Decision::List) => {
                    let target = match command {
                        Command::ChooseListEntry(index) if (index as usize) < list.len() => Some(index),
                        Command::ChooseListEntry(index) =>
                            return Err(Refusal::Invalid(format!("the list has {} entries, not {}", list.len(), index + 1))),
                        _ => None,
                    };
                    Driver::List { target, released: true }
                }
                _ => return Err(Refusal::Invalid("no list menu is waiting".into())),
            },
        };
        Ok(Self { command, driver })
    }

    pub fn drive(&mut self, modes: &[Mode], _world: &World) -> Drive {
        match &mut self.driver {
            Driver::Advance { answered } => match modes.last() {
                Some(Mode::TextBox(text)) if text.answered() == *answered => Drive::Press(Joypad::A),
                _ => Drive::Done,
            },
            Driver::List { target, released } => match modes.last() {
                Some(Mode::ListMenu(list)) => {
                    if list.status() != Status::Waiting(Decision::List) || !*released {
                        *released = true;
                        return Drive::Press(Joypad::empty());
                    }
                    *released = false;
                    let button = match *target {
                        None => Joypad::B,
                        Some(target) if (target as usize) < list.selected() => Joypad::UP,
                        Some(target) if (target as usize) > list.selected() => Joypad::DOWN,
                        Some(_) => Joypad::A,
                    };
                    Drive::Press(button)
                }
                _ => Drive::Done,
            },
        }
    }
}
