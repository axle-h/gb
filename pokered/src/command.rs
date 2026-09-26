//! An accepted command is carried out by pressing buttons into the same modes a player's go to.

use poke_core::item::ItemId;
use serde::{Deserialize, Serialize};
use crate::input::Joypad;
use crate::mode::{Mode, ModeUpdate, Status};
use crate::modes::battle::BattleDriver;
use crate::modes::naming_screen::NamingScreen;
use crate::modes::overworld::OverworldDriver;
use crate::modes::start_menu::StartMenuEntry;
use crate::systems::overworld::Direction;
use crate::world::World;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Answer a text box's `▼`, or press A at the title screen.
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
    /// Press B through an evolution's animation, which stops it unless an item forced it.
    CancelEvolution,
    /// Back out with B from the party menu, the bag's USE/TOSS, a mon's moves, or the list of moves
    /// to forget, which asks whether to abandon learning.
    CancelOption,
    /// Count up or down to a quantity and take it.
    ChooseQuantity(u8),
    CancelQuantity,
    /// Walk a square that way, turning first if the player faces elsewhere. Refused if something
    /// is in the way; a ledge or a warp is not in the way.
    Step(Direction),
    /// Turn to face that way without moving.
    Face(Direction),
    /// Shove the boulder that way with Strength on. The player stays put and the boulder moves a
    /// square; refused if there is no boulder there or it is blocked.
    Push(Direction),
    /// Press A at what is in front: a sign, a sprite to talk to, or a hidden event such as a PC.
    Interact,
    /// Press START in the overworld.
    OpenStartMenu,
    /// Use the move in this slot, from the battle menu or the move menu.
    Fight(u8),
    /// RUN from the battle menu.
    Run,
    /// Send out the party mon in this slot: PKMN and SWITCH, or the choice after a faint.
    SwitchPokemon(u8),
    /// Use a bag item in battle, on a party slot where it asks for one.
    UseItem { item: ItemId, target: Option<u8> },
    /// The Safari Zone's BALL, BAIT and THROW ROCK, which stand where FIGHT, PKMN and ITEM do.
    SafariBall,
    SafariBait,
    SafariRock,
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
    /// A page of the status screen, which is read and then left.
    StatusScreen,
    /// `LearnMove`'s list of the four moves, one of which is to be forgotten.
    ForgetMove,
    /// The bag's USE/TOSS menu, which `ChooseOption` answers.
    UseToss,
    /// `DisplayChooseQuantityMenu`.
    Quantity,
    /// A mon's moves, as the PP items ask for one; `ChooseOption` answers.
    MoveMenu,
    /// The trainer card, which `Advance` puts away.
    TrainerCard,
    /// The town map from the bag, walked with UP and DOWN, or a Pokédex nest map; either is left
    /// with `CancelOption`.
    TownMap,
    /// The town map FLY opens: a row per town visited, `ChooseOption` flies there and
    /// `CancelOption` backs out.
    FlyDestination,
    /// `CONTINUE`, `NEW GAME` and `OPTION`, which `ChooseOption` answers.
    MainMenu,
    /// The save's summary under the main menu: `Advance` takes it, `CancelOption` goes back.
    ContinueGame,
    /// FIGHT, PKMN, ITEM and RUN.
    BattleMenu,
    /// The battle's move menu, with the move's type and PP beside it.
    BattleMoves,
    /// The enemy's moves, one to be copied by the player's Mimic; `ChooseOption` answers, and B does
    /// nothing.
    MimicMove,
    /// SWITCH, STATS and CANCEL, for a party mon chosen in battle.
    SwitchStatsCancel,
    /// The title screen, cycling its mons until `Advance` presses A.
    TitleScreen,
    /// Oak's list of names, `NEW NAME` and three presets, which `ChooseOption` answers.
    IntroNameMenu,
    /// A menu an event draws for itself: the vending machine, the prize vendor, the fossils, Bill's
    /// list, the link cable help, the PC's menus. `ChooseOption` answers by row, `CancelOption` backs out with B.
    CursorMenu,
    /// The overworld, with the player free to move.
    Overworld,
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
    /// Holds A until the prompt answers. A box that takes the press can be replaced by another in
    /// the same frame, with the count it started from, so a wait that ends after a press is an answer
    /// too. `held` counts the frames A has been down.
    Advance { answered: u32, held: u8 },
    /// A press at a time, released on a frame the menu polls so that each is an edge.
    List { target: Option<u8>, released: bool },
    StartMenu { target: Option<u8>, released: bool },
    Options { released: bool },
    /// `chosen` is the stack's depth when A was pressed: a menu that answers by popping to another
    /// of the same kind, as a PC's LOG OFF does, cannot otherwise be told from the one asked.
    /// `stopped` is the menu having stopped waiting since: one that waits again at the same depth
    /// is another menu, as `LearnMove`'s second yes/no replaces its first inside one frame.
    Option { target: u8, from: Decision, released: bool, #[serde(default)] chosen: Option<usize>, #[serde(default)] stopped: bool },
    /// The grid is walked a letter at a time, so the target is kept and the next press worked out
    /// against whatever is typed so far.
    Name { target: Vec<u8>, released: bool },
    /// A dex number to walk to on the list, a row on the side menu, or nothing, which is the way
    /// out. The dex keeps all three of its screens on one mode, so a driver cannot finish by the
    /// mode going away: `from` is the screen it was asked on and `answers` catches the one row
    /// that answers without leaving.
    Dex { target: Option<u8>, from: Decision, answers: u32, released: bool },
    /// B while the evolution can still be stopped; one edge is enough.
    CancelEvolution,
    /// B on a polling frame; the menu has taken it by the next.
    Cancel { from: Decision, pressed: bool },
    Quantity { target: Option<u8>, released: bool },
    /// The overworld's own, which knows when a step or a turn has been taken.
    Overworld(OverworldDriver),
    /// The battle's own, which walks its menus to a move, a mon or an item.
    Battle(BattleDriver),
}

impl Executor {
    pub fn accept(command: Command, modes: &[Mode], world: &World) -> Result<Self, Refusal> {
        let driver = match &command {
            Command::Advance => match modes.last() {
                Some(Mode::TextBox(text)) if text.status() == Status::Waiting(Decision::Text) =>
                    Driver::Advance { answered: text.answered(), held: 0 },
                Some(Mode::StatusScreen(screen)) if screen.status() == Status::Waiting(Decision::StatusScreen) =>
                    Driver::Advance { answered: screen.answered(), held: 0 },
                Some(Mode::UseItem(flow)) if flow.status() == Status::Waiting(Decision::Text) =>
                    Driver::Advance { answered: flow.answered(), held: 0 },
                Some(Mode::Overworld(overworld)) if overworld.status() == Status::Waiting(Decision::Text) =>
                    Driver::Advance { answered: overworld.answered(), held: 0 },
                Some(Mode::TrainerCard(card)) if card.status() == Status::Waiting(Decision::TrainerCard) =>
                    Driver::Advance { answered: card.answered(), held: 0 },
                Some(Mode::MainMenu(menu)) if menu.status() == Status::Waiting(Decision::ContinueGame) =>
                    Driver::Advance { answered: menu.answered(), held: 0 },
                Some(Mode::Battle(battle)) if battle.status() == Status::Waiting(Decision::Text) =>
                    Driver::Advance { answered: battle.answered(), held: 0 },
                Some(Mode::Movie(movie)) if movie.status() == Status::Waiting(Decision::TitleScreen) =>
                    Driver::Advance { answered: movie.answered(), held: 0 },
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
                    Some(Mode::LearnMove(learn)) if learn.status() == Status::Waiting(Decision::ForgetMove) =>
                        learn.rows() as usize,
                    Some(Mode::ItemMenu(menu)) if menu.status() == Status::Waiting(Decision::UseToss) => 2,
                    Some(Mode::MoveSelectionMenu(menu)) if menu.status() == Status::Waiting(Decision::MoveMenu) =>
                        menu.rows() as usize,
                    Some(Mode::Pokedex(dex)) if dex.status() == Status::Waiting(Decision::PokedexSideMenu) => 4,
                    Some(Mode::MainMenu(menu)) if menu.status() == Status::Waiting(Decision::MainMenu) =>
                        menu.rows() as usize,
                    Some(Mode::Movie(movie)) if movie.status() == Status::Waiting(Decision::IntroNameMenu) => 4,
                    Some(Mode::CursorMenu(menu)) if menu.status() == Status::Waiting(Decision::CursorMenu) =>
                        menu.rows() as usize,
                    Some(Mode::TownMap(map)) if map.status() == Status::Waiting(Decision::FlyDestination) =>
                        map.rows() as usize,
                    Some(Mode::Battle(battle)) if battle.status() == Status::Waiting(Decision::MimicMove) =>
                        battle.mimic_rows() as usize,
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
                    Some(top) => {
                        let Status::Waiting(from) = top.status() else { unreachable!("only a waiting menu has rows") };
                        Driver::Option { target: *row, from, released: true, chosen: None, stopped: false }
                    }
                    None => unreachable!("a menu was matched above"),
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
            Command::CancelOption => match modes.last().map(|mode| mode.status()) {
                Some(Status::Waiting(from @ (Decision::PartyMenu | Decision::UseToss | Decision::MoveMenu | Decision::ContinueGame
                                              | Decision::BattleMoves | Decision::SwitchStatsCancel | Decision::CursorMenu
                                              | Decision::TownMap | Decision::FlyDestination | Decision::ForgetMove))) =>
                    Driver::Cancel { from, pressed: false },
                _ => return Err(Refusal::Invalid("no menu that B backs out of is waiting".into())),
            },
            Command::ChooseQuantity(_) | Command::CancelQuantity => match modes.last() {
                Some(Mode::QuantityMenu(menu)) if menu.status() == Status::Waiting(Decision::Quantity) => {
                    let target = match command {
                        Command::ChooseQuantity(n) if (1..=menu.max()).contains(&n) => Some(n),
                        Command::ChooseQuantity(n) =>
                            return Err(Refusal::Invalid(format!("the count runs from 1 to {}, not {n}", menu.max()))),
                        _ => None,
                    };
                    Driver::Quantity { target, released: true }
                }
                _ => return Err(Refusal::Invalid("no quantity is being chosen".into())),
            },
            Command::CancelEvolution => match modes.last() {
                Some(Mode::Evolution(evolution)) if evolution.can_cancel() => Driver::CancelEvolution,
                _ => return Err(Refusal::Invalid("no evolution can be stopped".into())),
            },
            Command::CloseOptions => match modes.last() {
                Some(Mode::OptionMenu(menu)) if menu.status() == Status::Waiting(Decision::Options) =>
                    Driver::Options { released: true },
                _ => return Err(Refusal::Invalid("the option screen is not open".into())),
            },
            Command::Step(_) | Command::Face(_) | Command::Push(_) | Command::Interact | Command::OpenStartMenu => match modes.last() {
                Some(Mode::Overworld(overworld)) => Driver::Overworld(match &command {
                    Command::Step(direction) => OverworldDriver::step(*direction, overworld, world)?,
                    Command::Push(direction) => OverworldDriver::push(*direction, overworld, world)?,
                    Command::Face(direction) => OverworldDriver::face(*direction, overworld, world)?,
                    Command::Interact => OverworldDriver::interact(overworld, world)?,
                    _ => OverworldDriver::open_start_menu(overworld, world)?,
                }),
                _ => return Err(Refusal::Invalid("the overworld is not on top".into())),
            },
            Command::Fight(_) | Command::Run | Command::SwitchPokemon(_) | Command::UseItem { .. }
            | Command::SafariBall | Command::SafariBait | Command::SafariRock =>
                Driver::Battle(BattleDriver::accept(&command, modes, world)?),
        };
        Ok(Self { command, driver })
    }

    pub fn drive(&mut self, modes: &[Mode], world: &World) -> Drive {
        match &mut self.driver {
            Driver::Advance { answered, held } => {
                let (count, status, decision) = match modes.last() {
                    Some(Mode::TextBox(text)) => (text.answered(), text.status(), Decision::Text),
                    Some(Mode::StatusScreen(screen)) => (screen.answered(), screen.status(), Decision::StatusScreen),
                    Some(Mode::UseItem(flow)) => (flow.answered(), flow.status(), Decision::Text),
                    Some(Mode::Overworld(overworld)) => (overworld.answered(), overworld.status(), Decision::Text),
                    Some(Mode::TrainerCard(card)) => (card.answered(), card.status(), Decision::TrainerCard),
                    Some(Mode::MainMenu(menu)) => (menu.answered(), menu.status(), Decision::ContinueGame),
                    Some(Mode::Battle(battle)) => (battle.answered(), battle.status(), Decision::Text),
                    Some(Mode::Movie(movie)) => (movie.answered(), movie.status(), Decision::TitleScreen),
                    _ => return Drive::Done,
                };
                if count != *answered || (*held > 0 && status != Status::Waiting(decision)) {
                    return Drive::Done;
                }
                // A release made while nothing polled is never seen, since an edge is against the
                // last poll: a press still unanswered after two frames goes up and comes down again.
                if *held >= 2 {
                    *held = 0;
                    return Drive::Press(Joypad::empty());
                }
                *held += 1;
                Drive::Press(Joypad::A)
            }
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
            Driver::Option { target, from, released, chosen, stopped } => {
                if chosen.is_some_and(|depth| depth != modes.len()) {
                    return Drive::Done;
                }
                let (kind, status, selected) = match modes.last() {
                    Some(Mode::TwoOptionMenu(menu)) => (Decision::TwoOption, menu.status(), menu.selected()),
                    Some(Mode::BuySellQuitMenu(menu)) => (Decision::BuySellQuit, menu.status(), menu.selected()),
                    Some(Mode::PartyMenu(menu)) => (Decision::PartyMenu, menu.status(), menu.selected()),
                    Some(Mode::FieldMoveMenu(menu)) => (Decision::FieldMoveMenu, menu.status(), menu.selected()),
                    Some(Mode::LearnMove(learn)) => (Decision::ForgetMove, learn.status(), learn.selected()),
                    Some(Mode::ItemMenu(menu)) => (Decision::UseToss, menu.status(), menu.selected()),
                    Some(Mode::MoveSelectionMenu(menu)) => (Decision::MoveMenu, menu.status(), menu.selected()),
                    Some(Mode::CursorMenu(menu)) => (Decision::CursorMenu, menu.status(), menu.selected()),
                    Some(Mode::TownMap(map)) => (Decision::FlyDestination, map.status(), map.selected()),
                    // The main menu stays up after a choice, so which of its screens is waiting says
                    // whether the choice has answered.
                    Some(Mode::MainMenu(menu)) => match menu.status() {
                        Status::Waiting(decision) => (decision, menu.status(), menu.selected()),
                        status => (Decision::MainMenu, status, menu.selected()),
                    },
                    Some(Mode::Movie(movie)) => match movie.status() {
                        Status::Waiting(decision) => (decision, movie.status(), movie.selected()),
                        status => (Decision::IntroNameMenu, status, movie.selected()),
                    },
                    // The battle stays up after the choice, so the same holds for Mimic's menu.
                    Some(Mode::Battle(battle)) => match battle.status() {
                        Status::Waiting(decision) => (decision, battle.status(), battle.selected()),
                        status => (Decision::MimicMove, status, battle.selected()),
                    },
                    _ => return Drive::Done,
                };
                // A different menu of options is up, so the one asked has answered and led to it.
                if kind != *from {
                    return Drive::Done;
                }
                let waiting = status == Status::Waiting(kind);
                if chosen.is_some() && !waiting {
                    *stopped = true;
                }
                if chosen.is_some() && waiting && *stopped {
                    return Drive::Done;
                }
                if !waiting || !*released || chosen.is_some() {
                    *released = true;
                    return Drive::Press(Joypad::empty());
                }
                *released = false;
                let button = match (*target).cmp(&selected) {
                    std::cmp::Ordering::Less => Joypad::UP,
                    std::cmp::Ordering::Greater => Joypad::DOWN,
                    std::cmp::Ordering::Equal => Joypad::A,
                };
                if button == Joypad::A {
                    *chosen = Some(modes.len());
                }
                Drive::Press(button)
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
            Driver::CancelEvolution => match modes.last() {
                Some(Mode::Evolution(evolution)) if evolution.can_cancel() => Drive::Press(Joypad::B),
                _ => Drive::Done,
            },
            Driver::Cancel { from, pressed } => {
                if *pressed {
                    return Drive::Done;
                }
                match modes.last().map(|mode| mode.status()) {
                    Some(Status::Waiting(decision)) if decision == *from => {
                        *pressed = true;
                        Drive::Press(Joypad::B)
                    }
                    Some(_) => Drive::Press(Joypad::empty()),
                    None => Drive::Done,
                }
            }
            Driver::Quantity { target, released } => match modes.last() {
                Some(Mode::QuantityMenu(menu)) => {
                    if menu.status() != Status::Waiting(Decision::Quantity) || !*released {
                        *released = true;
                        return Drive::Press(Joypad::empty());
                    }
                    *released = false;
                    Drive::Press(menu.press_toward(*target))
                }
                _ => Drive::Done,
            },
            Driver::Overworld(driver) => driver.drive(modes, world),
            Driver::Battle(driver) => driver.drive(modes, world),
        }
    }
}
