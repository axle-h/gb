pub mod audio;
pub mod command;
pub mod gfx;
pub mod input;
pub mod mode;
pub mod modes;
pub mod party;
pub mod rng;
pub mod sequence;
pub mod systems;
pub mod world;

#[cfg(test)]
mod fixtures;

use serde::{Deserialize, Serialize};
use command::{Command, Drive, Executor, Reply};
use gfx::ui::UiSurface;
use gfx::Screen;
use input::{Joypad, Pad};
use mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use modes::menu_input::CursorMemory;
use rng::GameRng;
use world::World;

pub enum Input {
    Buttons(Joypad),
    Command(Command),
    None,
}

/// `Instant` drops every delay that is presentation, leaving the waits for the player; the state
/// changes are the same either way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pacing {
    #[default]
    Faithful,
    Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    CommandDone(Command),
    CommandInterrupted { command: Command, reason: String },
}

#[derive(Debug)]
pub struct Frame {
    pub events: Vec<Event>,
    pub status: Status,
    pub reply: Option<Reply>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    world: World,
    modes: Vec<Mode>,
    rng: GameRng,
    pad: Pad,
    /// `hFrameCounter`, counted down by VBlank.
    frame_counter: u8,
    frames: u64,
    screen: Screen,
    menu: CursorMemory,
    executor: Option<Executor>,
    #[serde(skip)]
    pacing: Pacing,
}

const SAVE_MAGIC: &[u8; 4] = b"PKRD";
const SAVE_VERSION: u16 = 1;

impl Game {
    pub fn new(world: World, rng: GameRng, pacing: Pacing) -> Self {
        Self {
            world,
            modes: Vec::new(),
            rng,
            pad: Pad::default(),
            frame_counter: 0,
            frames: 0,
            screen: Screen::default(),
            menu: CursorMemory::default(),
            executor: None,
            pacing,
        }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn ui(&self) -> &UiSurface {
        &self.screen.ui
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn screen_mut(&mut self) -> &mut Screen {
        &mut self.screen
    }

    pub fn modes(&self) -> &[Mode] {
        &self.modes
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn push(&mut self, mode: Mode) {
        let mut events = Vec::new();
        self.with_ctx(&mut events, |modes, ctx| apply(modes, Transition::Push(mode), ctx));
    }

    /// One VBlank and one pass of the main loop.
    pub fn frame(&mut self, input: Input) -> Frame {
        let mut events = Vec::new();
        let mut reply = None;
        let mut buttons = Joypad::empty();
        match input {
            Input::Buttons(held) if self.executor.is_none() => buttons = held,
            Input::Buttons(_) | Input::None => {}
            Input::Command(command) => reply = Some(self.accept(command)),
        }
        if let Some(executor) = &mut self.executor {
            match executor.drive(&self.modes, &self.world) {
                Drive::Press(held) => buttons = held,
                Drive::Done => {
                    events.push(Event::CommandDone(executor.command.clone()));
                    self.executor = None;
                }
                Drive::Interrupted(reason) => {
                    events.push(Event::CommandInterrupted { command: executor.command.clone(), reason });
                    self.executor = None;
                }
            }
        }

        self.pad.input = buttons;
        self.screen.tiles.update_moving_bg_tiles();
        self.frame_counter = self.frame_counter.saturating_sub(1);
        self.frames += 1;

        self.with_ctx(&mut events, |modes, ctx| {
            if let Some(top) = modes.last_mut() {
                let transition = top.update(ctx);
                apply(modes, transition, ctx);
            }
        });

        Frame { events, status: self.status(), reply }
    }

    pub fn status(&self) -> Status {
        match (&self.executor, self.modes.last()) {
            (Some(_), _) => Status::Busy,
            (None, Some(top)) => top.status(),
            (None, None) => Status::Idle,
        }
    }

    fn accept(&mut self, command: Command) -> Reply {
        if self.executor.is_some() {
            return Reply::Refused(command::Refusal::Busy);
        }
        match Executor::accept(command, &self.modes, &self.world) {
            Ok(executor) => {
                self.executor = Some(executor);
                Reply::Accepted
            }
            Err(refusal) => Reply::Refused(refusal),
        }
    }

    fn with_ctx(&mut self, events: &mut Vec<Event>, f: impl FnOnce(&mut Vec<Mode>, &mut Ctx)) {
        let Self { world, modes, rng, pad, frame_counter, screen, menu, pacing, .. } = self;
        let mut ctx = Ctx { world, pad, rng, screen, menu, frame_counter, events, pacing: *pacing };
        f(modes, &mut ctx);
    }

    pub fn save(&self) -> Vec<u8> {
        let mut bytes = SAVE_MAGIC.to_vec();
        bytes.extend(SAVE_VERSION.to_le_bytes());
        bytes.extend(rmp_serde::to_vec_named(self).expect("a game always serialises"));
        bytes
    }

    pub fn load(bytes: &[u8], pacing: Pacing) -> Result<Self, String> {
        let body = bytes.strip_prefix(SAVE_MAGIC).ok_or("not a pokered save")?;
        let (version, body) = body.split_at_checked(2).ok_or("a truncated save")?;
        let version = u16::from_le_bytes([version[0], version[1]]);
        if version != SAVE_VERSION {
            return Err(format!("save version {version}, expected {SAVE_VERSION}"));
        }
        let mut game: Self = rmp_serde::from_slice(body).map_err(|e| e.to_string())?;
        game.pacing = pacing;
        Ok(game)
    }
}

fn apply(modes: &mut Vec<Mode>, transition: Transition, ctx: &mut Ctx) {
    match transition {
        Transition::Stay => {}
        Transition::Push(mut mode) => {
            mode.enter(ctx);
            modes.push(mode);
        }
        Transition::Pop(outcome) => {
            modes.pop();
            resume(modes, outcome, ctx);
        }
        Transition::Replace(mut mode) => {
            modes.pop();
            mode.enter(ctx);
            modes.push(mode);
        }
    }
}

fn resume(modes: &mut Vec<Mode>, outcome: Outcome, ctx: &mut Ctx) {
    if let Some(parent) = modes.last_mut() {
        let transition = parent.resume(outcome, ctx);
        apply(modes, transition, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_round_trips() {
        let game = Game::new(World::default(), GameRng::seeded(1), Pacing::Faithful);
        let loaded = Game::load(&game.save(), Pacing::Faithful).unwrap();
        assert_eq!(loaded.save(), game.save());
    }

    #[test]
    fn a_save_from_something_else_is_an_error() {
        assert!(Game::load(b"GBST\x01\x00", Pacing::Faithful).is_err());
        let mut save = Game::new(World::default(), GameRng::seeded(1), Pacing::Faithful).save();
        save[4] = 99;
        assert_eq!(Game::load(&save, Pacing::Faithful).unwrap_err(), "save version 99, expected 1");
    }
}
