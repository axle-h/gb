pub mod audio;
pub mod command;
pub mod gfx;
pub mod input;
pub mod mode;
pub mod modes;
pub mod party;
pub mod rng;
pub mod scripts;
pub mod sequence;
pub mod systems;
pub mod world;

#[cfg(test)]
mod fixtures;

use serde::{Deserialize, Serialize};
use audio::data::AudioBank;
use audio::engine::AudioEngine;
use audio::Write;
use command::{Command, Drive, Executor, Reply};
use gfx::ui::UiSurface;
use gfx::Screen;
use input::{Joypad, Pad};
use mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use modes::menu_input::CursorMemory;
use modes::movie::Movie;
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
    /// This frame's register writes, for the host to play. The game holds no audio backend, so a
    /// save carries no oscillator state.
    pub audio: Vec<Write>,
    /// The game's own save, written this frame: what the SAVE menu, a box change and the Hall of
    /// Fame hand the host to keep. `Game::save`'s bytes.
    pub save: Option<Vec<u8>>,
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
    audio: AudioEngine,
    executor: Option<Executor>,
    /// Whose save file the host has, for `CheckPreviousSaveFile`.
    #[serde(default)]
    saved_player_id: Option<u16>,
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
            // `PlayMusic` carries the bank its song lives in, so whatever plays first corrects this.
            audio: AudioEngine::new(AudioBank::One),
            executor: None,
            saved_player_id: None,
            pacing,
        }
    }

    /// Power on: the splash, the intro and the title screen, then the main menu into a new game, or
    /// into `save` when there is one to continue.
    pub fn power_on(save: Option<World>, rng: GameRng, pacing: Pacing) -> Self {
        let saved_player_id = save.as_ref().map(|world| world.player_id);
        let mut game = Self::new(save.unwrap_or_default(), rng, pacing);
        game.saved_player_id = saved_player_id;
        game.push(Mode::Movie(Movie::power_on(saved_player_id.is_some())));
        game
    }

    /// Whose save file the host holds. `power_on` takes it from the save it is given, a save the
    /// game writes replaces it, and a host with no file says `None`.
    pub fn set_saved_player_id(&mut self, id: Option<u16>) {
        self.saved_player_id = id;
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

    /// What the menus left behind: the chosen row and how the menu was left.
    pub fn menu(&self) -> &CursorMemory {
        &self.menu
    }

    pub fn menu_mut(&mut self) -> &mut CursorMemory {
        &mut self.menu
    }

    pub fn audio(&self) -> &AudioEngine {
        &self.audio
    }

    pub fn audio_mut(&mut self) -> &mut AudioEngine {
        &mut self.audio
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
        let audio = self.audio.frame();
        self.world.play_time.track();
        self.frame_counter = self.frame_counter.saturating_sub(1);
        self.frames += 1;

        let save_game = self.with_ctx(&mut events, |modes, ctx| {
            if let Some(top) = modes.last_mut() {
                let transition = top.update(ctx);
                apply(modes, transition, ctx);
            }
        });
        // `SaveGameData`. The save is taken after the frame's transitions, so it holds the screen the
        // player is looking at, and it becomes the file the next `CheckPreviousSaveFile` sees.
        let save = save_game.then(|| {
            self.saved_player_id = Some(self.world.player_id);
            self.save()
        });

        Frame { events, status: self.status(), reply, audio, save }
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

    fn with_ctx(&mut self, events: &mut Vec<Event>, f: impl FnOnce(&mut Vec<Mode>, &mut Ctx)) -> bool {
        let Self { world, modes, rng, pad, frame_counter, screen, menu, audio, pacing, saved_player_id, .. } = self;
        let mut ctx = Ctx { world, pad, rng, screen, menu, audio, frame_counter, events, pacing: *pacing,
                            update_sprites: false, save_game: false, saved_player_id: *saved_player_id };
        f(modes, &mut ctx);
        if ctx.update_sprites
            && let Some(Mode::Overworld(overworld)) = modes.iter_mut().rev().find(|mode| matches!(mode, Mode::Overworld(_)))
        {
            overworld.update_sprites_under(&mut ctx);
        }
        ctx.save_game
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
            opened(modes, ctx);
        }
        Transition::Pop(outcome) => {
            modes.pop();
            resume(modes, outcome, ctx);
        }
        Transition::Replace(mut mode) => {
            modes.pop();
            mode.enter(ctx);
            modes.push(mode);
            opened(modes, ctx);
        }
    }
}

fn opened(modes: &mut Vec<Mode>, ctx: &mut Ctx) {
    let transition = modes.last_mut().expect("a mode was just pushed").open(ctx);
    apply(modes, transition, ctx);
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

    use std::hash::{Hash, Hasher};
    use poke_core::charmap::encode;
    use poke_core::map::Map;
    use poke_core::species::PokemonSpecies;
    use crate::command::Decision;
    use crate::modes::battle::BattleMode;
    use crate::modes::slots::SlotMachine;
    use crate::party::Named;
    use crate::systems::add_mon::{new_party_mon, Origin};

    /// What one frame is fed, kept so a loaded copy can be fed it again.
    #[derive(Clone)]
    enum Fed {
        Buttons(Joypad),
        Command(Command),
        Nothing,
    }

    impl Fed {
        fn input(&self) -> Input {
            match self {
                Fed::Buttons(held) => Input::Buttons(*held),
                Fed::Command(command) => Input::Command(command.clone()),
                Fed::Nothing => Input::None,
            }
        }
    }

    fn digest(bytes: &[u8]) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        bytes.hash(&mut hasher);
        hasher.finish()
    }

    /// `NR50`, `NR51`, wave RAM and the wave DAC as a backend fed `writes` holds them: what no note
    /// writes again.
    #[derive(Debug, Clone, Default, PartialEq)]
    struct Kept {
        master_volume: u8,
        panning: u8,
        wave_ram: [u8; 16],
        wave_dac: bool,
    }

    impl Kept {
        fn feed(&mut self, writes: &[Write]) {
            for &write in writes {
                match write {
                    Write::MasterVolume { .. } => self.master_volume = write.register().1,
                    Write::Panning(panning) => self.panning = panning,
                    Write::WaveRam { index, samples } => self.wave_ram[index as usize] = samples,
                    Write::WaveDac(on) => self.wave_dac = on,
                    _ => {}
                }
            }
        }
    }

    /// One frame of the original run, as a copy loaded before it must reproduce it.
    struct Played {
        fed: Fed,
        audio: Vec<Write>,
        status: Status,
        save: u64,
        saved: Option<u64>,
        kept: Kept,
    }

    /// Plays `game` for `frames` frames, feeding what `drive` picks and saving wherever `save_here`
    /// says; then loads every save and plays it on with the same inputs. Each copy must give the same
    /// `save()` bytes, status, game save and audio writes from then on, and a backend that starts on
    /// the copy with its standing writes must hold the registers no note rewrites exactly as one that
    /// heard the whole run does. Returns the frames saved at.
    fn resumes_identically(mut game: Game, frames: usize, mut drive: impl FnMut(&Game) -> Fed,
        mut save_here: impl FnMut(&Game, usize) -> bool) -> Vec<usize>
    {
        let mut saves = vec![];
        let mut played = vec![];
        let mut kept = Kept::default();
        for frame in 0..frames {
            if save_here(&game, frame) {
                saves.push((frame, game.save()));
            }
            let fed = drive(&game);
            let out = game.frame(fed.input());
            kept.feed(&out.audio);
            played.push(Played { fed, audio: out.audio, status: out.status, save: digest(&game.save()),
                saved: out.save.as_deref().map(digest), kept: kept.clone() });
        }
        assert!(!saves.is_empty(), "nowhere was saved");
        for (from, bytes) in &saves {
            let mut copy = Game::load(bytes, Pacing::Faithful).unwrap();
            assert_eq!(copy.save(), *bytes, "frame {from}: the load round trips");
            let mut backend = Kept::default();
            backend.feed(&copy.audio().standing_writes());
            for (frame, original) in played.iter().enumerate().skip(*from) {
                let out = copy.frame(original.fed.input());
                let what = format!("saved at frame {from}, frame {frame}");
                assert_eq!(out.audio, original.audio, "{what}: the audio");
                backend.feed(&out.audio);
                assert_eq!(backend, original.kept, "{what}: what a backend started on the copy holds");
                assert_eq!(out.status, original.status, "{what}: the status");
                assert_eq!(out.save.as_deref().map(digest), original.saved, "{what}: the game's own save");
                assert_eq!(digest(&copy.save()), original.save, "{what}: the save bytes");
            }
        }
        saves.into_iter().map(|(frame, _)| frame).collect()
    }

    fn named(species: PokemonSpecies, level: u8, nick: &str) -> Named<crate::party::PartyMon> {
        Named {
            mon: new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![])),
            ot: encode("RED").unwrap(),
            nick: encode(nick).unwrap(),
        }
    }

    fn battle_game(opponent: BattleMode) -> Game {
        let world = World {
            player_name: encode("RED").unwrap(),
            party: vec![named(PokemonSpecies::Pidgey, 50, "BIRD"), named(PokemonSpecies::Rattata, 40, "RAT")],
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(7), Pacing::Faithful);
        game.push(Mode::Battle(opponent));
        game
    }

    fn battle(game: &Game) -> Option<&BattleMode> {
        game.modes().iter().find_map(|mode| match mode {
            Mode::Battle(battle) => Some(battle),
            _ => None,
        })
    }

    /// Every question answered with move `fight`, No or the text's `▼`.
    fn answer(game: &Game, fight: u8) -> Fed {
        match game.status() {
            Status::Waiting(Decision::Text) => Fed::Command(Command::Advance),
            Status::Waiting(Decision::BattleMenu | Decision::BattleMoves) => Fed::Command(Command::Fight(fight)),
            Status::Waiting(Decision::TwoOption) => Fed::Command(Command::ChooseOption(1)),
            _ => Fed::Nothing,
        }
    }

    /// Saves where `interesting` first turns true and a few frames on from there, `limit` times.
    fn at_starts_of(limit: usize, mut interesting: impl FnMut(&Game) -> bool) -> impl FnMut(&Game, usize) -> bool {
        let (mut was, mut since, mut taken) = (false, None, 0);
        move |game, _| {
            let now = interesting(game);
            let started = now && !was;
            was = now;
            if started && taken < limit {
                since = Some(0);
            }
            let save = match &mut since {
                Some(frames) => {
                    let save = matches!(*frames, 0 | 7);
                    *frames += 1;
                    if *frames > 7 {
                        since = None;
                        taken += 1;
                    }
                    save
                }
                None => false,
            };
            save
        }
    }

    #[test]
    fn a_wild_battle_saved_mid_turn_resumes_identically() {
        // WING ATTACK, which ONIX shrugs off for long enough to hit back.
        let game = battle_game(BattleMode::wild(PokemonSpecies::Onix, 20));
        let (mut animations, mut bars) = (0, 0);
        let mut animating = at_starts_of(3, |game| battle(game).is_some_and(BattleMode::animating));
        let mut draining = at_starts_of(3, |game| battle(game).is_some_and(BattleMode::draining_hp_bar));
        let saved = resumes_identically(game, 3000, |game| answer(game, 1), |game, frame| {
            let (a, b) = (animating(game, frame), draining(game, frame));
            animations += (a && battle(game).is_some_and(BattleMode::animating)) as u32;
            bars += (b && battle(game).is_some_and(BattleMode::draining_hp_bar)) as u32;
            a || b
        });
        assert!(animations >= 3 && bars >= 3, "saved in {animations} animations and {bars} bars, at {saved:?}");
    }

    #[test]
    fn a_trainer_battle_saved_across_a_faint_and_a_send_out_resumes_identically() {
        let game = battle_game(BattleMode::trainer(1, 1, 0, 0));
        let enemy = |game: &Game| battle(game).and_then(BattleMode::battle).map(|battle| (battle.enemy.mon.species, battle.enemy.mon.hp));
        let (mut fainted, mut sent_out) = (at_starts_of(2, move |game| enemy(game).is_some_and(|(_, hp)| hp == 0)), None);
        let mut sends = 0;
        let saved = resumes_identically(game, 4000, |game| answer(game, 1), |game, frame| {
            let species = enemy(game).map(|(species, _)| species);
            let new_mon = species.is_some() && sent_out.is_some_and(|last| last != species) ;
            sent_out = Some(species);
            sends += new_mon as u32;
            fainted(game, frame) || new_mon
        });
        assert!(sends >= 1, "no send-out was saved at: {saved:?}");
        assert!(saved.len() >= 5, "saved at {saved:?}");
    }

    #[test]
    fn the_intro_movie_saved_anywhere_resumes_identically() {
        let game = Game::power_on(None, GameRng::seeded(1), Pacing::Faithful);
        let saved = resumes_identically(game, 900, |_| Fed::Nothing, |_, frame| frame % 97 == 13);
        assert_eq!(saved.len(), 10);
    }

    /// A champion in the Hall of Fame, saved through the ceremony and the credits.
    #[test]
    fn the_hall_of_fame_and_the_credits_saved_anywhere_resume_identically() {
        let mut world = World { player_name: encode("RED").unwrap(), ..World::default() };
        world.party = vec![named(PokemonSpecies::Squirtle, 85, "MON"), named(PokemonSpecies::Pidgey, 9, "MON")];
        world.location.map = Map::HallOfFame;
        let mut game = Game::new(world, GameRng::seeded(4), Pacing::Faithful);
        game.push(Mode::Movie(Movie::hall_of_fame()));
        let text = |game: &Game| match game.status() {
            Status::Waiting(Decision::Text) => Fed::Command(Command::Advance),
            _ => Fed::Nothing,
        };
        let saved = resumes_identically(game, 7000, text, |_, frame| frame % 700 == 350);
        assert_eq!(saved.len(), 10);
    }

    /// A slot machine taken from the bet through the spin to the next offer, saved as the wheels turn.
    #[test]
    fn a_slot_machine_saved_mid_spin_resumes_identically() {
        let world = World { coins: [0x10, 0x00], ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(3), Pacing::Faithful);
        game.push(Mode::SlotMachine(SlotMachine::new(crate::systems::slots::NOT_LUCKY)));
        let mut frames = 0;
        let play = move |game: &Game| {
            frames += 1;
            match game.status() {
                Status::Waiting(Decision::TwoOption) => Fed::Command(Command::ChooseOption(1)),
                Status::Waiting(Decision::Text) => Fed::Command(Command::Advance),
                Status::Waiting(_) => Fed::Command(Command::ChooseOption(0)),
                _ if frames % 2 == 0 => Fed::Buttons(Joypad::A),
                _ => Fed::Nothing,
            }
        };
        let spinning = |game: &Game| matches!(game.modes().last(), Some(Mode::SlotMachine(machine)) if machine.spinning());
        let saved = resumes_identically(game, 1500, play, move |game, frame| spinning(game) && frame % 11 == 0);
        assert!(saved.len() >= 5, "saved at {saved:?}");
    }
}
