//! An accepted command is carried out by pressing buttons into the same modes a player's go to.

use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::mode::{Mode, ModeUpdate, Status};
use crate::modes::naming_screen::NamingScreen;
use crate::modes::start_menu::StartMenuEntry;
use crate::world::World;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Answer a text box's `▼`.
    Advance,
    /// Pick a list menu's entry, by its index in the whole list.
    ChooseListEntry(u8),
    CancelList,
    /// Open one of the start menu's screens.
    ChooseStartMenuEntry(StartMenuEntry),
    CloseStartMenu,
    CloseOptions,
    /// Answer a two-option menu, or the mart's front menu, by row.
    ChooseOption(u8),
    /// Type a name into the naming screen and hand it back. Charmap bytes, as the game keeps them.
    EnterName(Vec<u8>),
    /// Open a Pokédex number's side menu, walking the list to it.
    ChooseDexEntry(u8),
    /// Back out of whichever of the Pokédex's three screens is up.
    CloseDex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    /// A text box waiting at its `▼`.
    Text,
    List,
    StartMenu,
    /// The option screen, which the host sets rather than the model: the only answer is to leave.
    Options,
    TwoOption,
    BuySellQuit,
    PartyMenu,
    NamingScreen,
    FieldMoveMenu,
    /// The Pokédex's list of numbers.
    Pokedex,
    /// Its DATA/CRY/AREA/QUIT menu, which `ChooseOption` answers.
    PokedexSideMenu,
    /// A data page, which is read and then left.
    PokedexData,
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
    StartMenu { target: Option<u8>, released: bool },
    Options { released: bool },
    Option { target: u8, released: bool },
    /// The grid is walked a letter at a time, so the target is kept and the next press worked out
    /// against whatever is typed so far.
    Name { target: Vec<u8>, released: bool },
    /// A dex number to walk to on the list, a row on the side menu, or nothing, which is the way
    /// out. The dex keeps all three of its screens on one mode, so a driver cannot finish by the
    /// mode going away: `from` is the screen it was asked on and `answers` catches the one row
    /// that answers without leaving.
    Dex { target: Option<u8>, from: Decision, answers: u32, released: bool },
}

impl Executor {
    pub fn accept(command: Command, modes: &[Mode], world: &World) -> Result<Self, Refusal> {
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
            Command::ChooseStartMenuEntry(_) | Command::CloseStartMenu => match modes.last() {
                Some(Mode::StartMenu(menu)) if menu.status() == Status::Waiting(Decision::StartMenu) => {
                    let target = match command {
                        Command::ChooseStartMenuEntry(entry) => match menu.index_of(entry) {
                            Some(index) => Some(index),
                            None => return Err(Refusal::Invalid(format!("the start menu has no {entry:?} row"))),
                        },
                        _ => None,
                    };
                    Driver::StartMenu { target, released: true }
                }
                _ => return Err(Refusal::Invalid("no start menu is waiting".into())),
            },
            Command::ChooseOption(row) => {
                let rows = match modes.last() {
                    Some(Mode::TwoOptionMenu(menu)) if menu.status() == Status::Waiting(Decision::TwoOption) => 2,
                    Some(Mode::BuySellQuitMenu(menu)) if menu.status() == Status::Waiting(Decision::BuySellQuit) => 3,
                    Some(Mode::PartyMenu(menu)) if menu.status() == Status::Waiting(Decision::PartyMenu) =>
                        world.party.len(),
                    Some(Mode::FieldMoveMenu(menu)) if menu.status() == Status::Waiting(Decision::FieldMoveMenu) =>
                        menu.rows() as usize,
                    Some(Mode::Pokedex(dex)) if dex.status() == Status::Waiting(Decision::PokedexSideMenu) => 4,
                    _ => return Err(Refusal::Invalid("no menu of options is waiting".into())),
                };
                if *row as usize >= rows {
                    return Err(Refusal::Invalid(format!("the menu has {rows} rows, not {}", row + 1)));
                }
                match modes.last() {
                    Some(Mode::Pokedex(dex)) => Driver::Dex {
                        target: Some(*row),
                        from: Decision::PokedexSideMenu,
                        answers: dex.answered(),
                        released: true,
                    },
                    _ => Driver::Option { target: *row, released: true },
                }
            }
            Command::EnterName(name) => match modes.last() {
                Some(Mode::NamingScreen(screen)) if screen.status() == Status::Waiting(Decision::NamingScreen) => {
                    if name.len() > screen.limit() {
                        return Err(Refusal::Invalid(format!("a name here is {} letters at most", screen.limit())));
                    }
                    if let Some(byte) = name.iter().find(|&&byte| NamingScreen::position_of(byte).is_none()) {
                        return Err(Refusal::Invalid(format!("the grid has no ${byte:02X} to type")));
                    }
                    Driver::Name { target: name.clone(), released: true }
                }
                _ => return Err(Refusal::Invalid("no naming screen is waiting".into())),
            },
            Command::ChooseDexEntry(_) | Command::CloseDex => match modes.last() {
                Some(Mode::Pokedex(dex)) => {
                    let Status::Waiting(from) = dex.status() else {
                        return Err(Refusal::Invalid("the Pokédex is not waiting".into()));
                    };
                    let target = match command {
                        Command::ChooseDexEntry(number) => {
                            if from != Decision::Pokedex {
                                return Err(Refusal::Invalid("the Pokédex's list is not the screen that is up".into()));
                            }
                            if number == 0 || number > dex.max_seen() {
                                return Err(Refusal::Invalid(format!("the list stops at {}", dex.max_seen())));
                            }
                            // An entry the player has not seen answers nothing: the menu never opens.
                            if !crate::systems::pokedex::is_set(&world.pokedex.seen, number) {
                                return Err(Refusal::Invalid(format!("nothing has been seen at {number}")));
                            }
                            Some(number)
                        }
                        _ => None,
                    };
                    Driver::Dex { target, from, answers: dex.answered(), released: true }
                }
                _ => return Err(Refusal::Invalid("the Pokédex is not open".into())),
            },
            Command::CloseOptions => match modes.last() {
                Some(Mode::OptionMenu(menu)) if menu.status() == Status::Waiting(Decision::Options) =>
                    Driver::Options { released: true },
                _ => return Err(Refusal::Invalid("the option screen is not open".into())),
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
            Driver::StartMenu { target, released } => match modes.last() {
                Some(Mode::StartMenu(menu)) => {
                    if menu.status() != Status::Waiting(Decision::StartMenu) || !*released {
                        *released = true;
                        return Drive::Press(Joypad::empty());
                    }
                    *released = false;
                    let button = match *target {
                        None => Joypad::B,
                        Some(target) if target < menu.selected() => Joypad::UP,
                        Some(target) if target > menu.selected() => Joypad::DOWN,
                        Some(_) => Joypad::A,
                    };
                    Drive::Press(button)
                }
                _ => Drive::Done,
            },
            Driver::Option { target, released } => {
                let (waiting, selected) = match modes.last() {
                    Some(Mode::TwoOptionMenu(menu)) =>
                        (menu.status() == Status::Waiting(Decision::TwoOption), menu.selected()),
                    Some(Mode::BuySellQuitMenu(menu)) =>
                        (menu.status() == Status::Waiting(Decision::BuySellQuit), menu.selected()),
                    Some(Mode::PartyMenu(menu)) =>
                        (menu.status() == Status::Waiting(Decision::PartyMenu), menu.selected()),
                    Some(Mode::FieldMoveMenu(menu)) =>
                        (menu.status() == Status::Waiting(Decision::FieldMoveMenu), menu.selected()),
                    _ => return Drive::Done,
                };
                if !waiting || !*released {
                    *released = true;
                    return Drive::Press(Joypad::empty());
                }
                *released = false;
                Drive::Press(match (*target).cmp(&selected) {
                    std::cmp::Ordering::Less => Joypad::UP,
                    std::cmp::Ordering::Greater => Joypad::DOWN,
                    std::cmp::Ordering::Equal => Joypad::A,
                })
            }
            Driver::Name { target, released } => match modes.last() {
                Some(Mode::NamingScreen(screen)) => {
                    if screen.status() != Status::Waiting(Decision::NamingScreen) || !*released {
                        *released = true;
                        return Drive::Press(Joypad::empty());
                    }
                    *released = false;
                    Drive::Press(screen.press_toward(target))
                }
                _ => Drive::Done,
            },
            Driver::Dex { target, from, answers, released } => match modes.last() {
                // The side menu answered where it stood, which is what a chosen `CRY` looks like.
                Some(Mode::Pokedex(dex)) if dex.answered() != *answers => Drive::Done,
                Some(Mode::Pokedex(dex)) => match dex.status() {
                    // Another of the dex's screens is up, so the one asked about has answered.
                    Status::Waiting(decision) if decision != *from => Drive::Done,
                    // Drawing, printing or clearing: nothing to press at yet.
                    Status::Busy | Status::Idle => {
                        *released = true;
                        Drive::Press(Joypad::empty())
                    }
                    Status::Waiting(_) if !*released => {
                        *released = true;
                        Drive::Press(Joypad::empty())
                    }
                    Status::Waiting(_) => {
                        *released = false;
                        match (*target, *from == Decision::Pokedex) {
                            (None, _) => Drive::Press(Joypad::B),
                            (Some(number), true) => Drive::Press(dex.press_toward(number)),
                            (Some(row), false) => Drive::Press(match row.cmp(&dex.selected()) {
                                std::cmp::Ordering::Less => Joypad::UP,
                                std::cmp::Ordering::Greater => Joypad::DOWN,
                                std::cmp::Ordering::Equal => Joypad::A,
                            }),
                        }
                    }
                },
                _ => Drive::Done,
            },
            Driver::Options { released } => match modes.last() {
                Some(Mode::OptionMenu(menu)) => {
                    if menu.status() != Status::Waiting(Decision::Options) || !*released {
                        *released = true;
                        return Drive::Press(Joypad::empty());
                    }
                    *released = false;
                    Drive::Press(Joypad::B)
                }
                _ => Drive::Done,
            },
        }
    }
}
