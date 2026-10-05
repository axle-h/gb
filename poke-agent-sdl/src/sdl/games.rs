//! The two games the window holds, the cartridge on the emulator and the recreation, advanced a
//! frame together so a press routed to both lands on the same frame of each. They drift apart
//! anyway, by the cartridge's lag frames and the loading the recreation drops.

use std::path::Path;
use gb::cycles::MachineCycles;
use gb::game_boy::GameBoy;
use poke_agent::pokemon::PokemonApi;
use poke_agent::pokemon::agent::{AgentEvent, PokemonAgent};
use poke_agent::pokemon::map_metadata::MapMetadataCache;
use poke_agent::pokemon::native_agent::NativeAgent;
use poke_agent::pokemon::options::{SERVED_OPTIONS, keep_game_options};
use poke_agent::pokemon::policy::{ConsolePolicy, Policy, Trace};
use poke_agent::pokemon::speech::Speech;
use pokered::audio::Voices;
use pokered::audio::synth::{HEADROOM, Synth};
use pokered::input::Joypad;
use pokered::rng::GameRng;
use pokered::save_slots::{DirectoryStore, SavedAt, SlotRequest, SlotStore};
use pokered::{Game, Input, Pacing};
use crate::sdl::log::Source;
use crate::sdl::routing::{BUTTONS, Routing};
use crate::sdl::tour::Tour;

/// One of the two games, as a half of the window and as the kind of file that holds its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Emulated,
    Native,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Emulated => "emulated",
            Side::Native => "native",
        }
    }

    /// The game whose state `bytes` holds, by its magic: the emulator's save state, or a `Game::save`.
    pub fn of_file(bytes: &[u8]) -> Option<Side> {
        if bytes.starts_with(b"GBST") {
            Some(Side::Emulated)
        } else if bytes.starts_with(b"PKRD") {
            Some(Side::Native)
        } else {
            None
        }
    }
}

/// Each game boxed, as either is too big to be moved about on a test thread's stack.
pub struct Games {
    pub gb: Box<GameBoy>,
    pub agent: PokemonAgent,
    pub map_cache: MapMetadataCache,
    /// The emulated game is played by its agent rather than the keyboard.
    pub agent_running: bool,
    /// The agent's last failure, said once however many slices it fails in a row.
    agent_failure: Option<String>,
    /// Cycles the agent ran past the frames asked of it, taken off the next.
    ahead: MachineCycles,
    /// What the emulator's joypad holds, so only a change is pressed or let go.
    emulated_held: Joypad,
    emulated_trace: Trace,
    emulated_speech: Speech,
    pub native: Box<NativeAgent>,
    native_trace: Trace,
    native_speech: Speech,
    /// The recreation's sound, the only sound the window plays.
    synth: Synth,
    /// Where the recreation keeps its save slots, the directory `pokered-sdl` keeps them in.
    native_slots: Option<DirectoryStore>,
    /// The grand tour on both, once one is started; kept after it ends, for its standing.
    pub tour: Option<Tour>,
}

impl Games {
    /// Both from power-on: the cartridge cold, with `sram` as its battery if there is one, and the
    /// recreation with the slots in the directory `native_slots`.
    pub fn power_on(
        sram: Option<&str>,
        native_slots: Option<std::path::PathBuf>,
        mut emulated_policy: Box<dyn Policy>,
        mut native_policy: Box<dyn Policy>,
    ) -> Result<Self, String> {
        let mut gb = Box::new(GameBoy::sgb(poke_agent::pokemon::roms::POKERED));
        if let Some(sram) = sram && let Err(e) = gb.restore_sram_from_file(sram) {
            println!("Could not load save file: {e}");
        }
        gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
        let emulated_trace = Trace::listening();
        emulated_policy.trace_to(emulated_trace.clone());
        let native_trace = Trace::listening();
        native_policy.trace_to(native_trace.clone());
        let native_slots = native_slots.map(DirectoryStore::new);
        let game = native_power_on(native_slots.as_ref());
        Ok(Self {
            gb,
            agent: PokemonAgent::new(emulated_policy),
            map_cache: MapMetadataCache::default(),
            agent_running: false,
            agent_failure: None,
            ahead: MachineCycles::ZERO,
            emulated_held: Joypad::empty(),
            emulated_trace,
            emulated_speech: Speech::emulated(),
            synth: poke_agent::native::synth_for(&game),
            native: Box::new(NativeAgent::new(game, native_policy)?.hosted()),
            native_trace,
            native_speech: Speech::native(),
            native_slots,
            tour: None,
        })
    }

    /// A grand tour is playing both games, and the keyboard plays neither.
    pub fn touring(&self) -> bool {
        self.tour.as_ref().is_some_and(Tour::running)
    }

    /// Both games replaced by the grand tour's fresh starts, each played by its own tour.
    pub fn start_tour(&mut self) -> Result<(), String> {
        let (tour, policies) = Tour::new()?;
        let game = Tour::native_game()?;
        self.gb = Tour::emulated_game()?;
        self.map_cache = MapMetadataCache::default();
        self.emulator_restored();
        self.agent = self.emulated_agent(policies.emulated);
        self.agent_running = false;
        self.agent_failure = None;
        self.synth = poke_agent::native::synth_for(&game);
        let mut native_policy = policies.native;
        native_policy.trace_to(self.native_trace.clone());
        self.native = Box::new(NativeAgent::new(game, native_policy)?.hosted());
        self.emulated_speech = Speech::emulated();
        self.native_speech = Speech::native();
        tour.begin(&self.gb, &self.native);
        self.tour = Some(tour);
        Ok(())
    }

    /// The emulated agent on `policy`, its trace into the log.
    fn emulated_agent(&self, mut policy: Box<dyn Policy>) -> PokemonAgent {
        policy.trace_to(self.emulated_trace.clone());
        PokemonAgent::new(policy)
    }

    /// One frame of each, `held` routed between them, and whatever either said into `log`.
    pub fn frame(&mut self, routing: Routing, held: Joypad, log: &mut impl FnMut(Source, String)) {
        let touring = self.touring();
        let (emulated, native) = if touring { (Joypad::empty(), Joypad::empty()) } else { routing.route(held) };
        self.emulated_frame(emulated, touring, log);
        self.native_frame(native, touring, log);
        // The in-game clock, which a loaded slot carries on and the game's frame count does not.
        let clock = self.native.game().world().play_time;
        let played = std::time::Duration::from_secs(u64::from(clock.hours) * 3600 + u64::from(clock.minutes) * 60 + u64::from(clock.seconds));
        let Some(tour) = self.tour.as_mut() else { return };
        for (side, line) in tour.take_endings(played) {
            log(Source::of(side), line);
        }
        // Back to the keyboard: F5 plays the console agent again rather than a tour's.
        if !tour.running() && !tour.handed_back {
            tour.handed_back = true;
            self.agent = self.emulated_agent(Box::new(ConsolePolicy::default()));
        }
    }

    fn emulated_frame(&mut self, held: Joypad, touring: bool, log: &mut impl FnMut(Source, String)) {
        let joypad = self.gb.core_mut().mmu_mut().joypad_mut();
        for (pad, button) in BUTTONS {
            match (self.emulated_held.contains(pad), held.contains(pad)) {
                (false, true) => joypad.press_button(button),
                (true, false) => joypad.release_button(button),
                _ => {}
            }
        }
        self.emulated_held = held;

        let frame = MachineCycles::PER_FRAME;
        let budget = if self.ahead >= frame { MachineCycles::ZERO } else { frame - self.ahead };
        let tour = self.tour.as_mut().filter(|tour| !tour.emulated.progress.is_over());
        let ran = if budget == MachineCycles::ZERO {
            MachineCycles::ZERO
        } else if self.agent_running || tour.is_some() {
            let (ran, result) = match tour {
                Some(tour) => tour.emulated_tick(&mut self.gb, &mut self.agent, &mut self.map_cache),
                None => agent_slice(&mut self.agent, &mut self.gb, &mut self.map_cache, budget),
            };
            match result {
                Err(failure) if self.agent_failure.as_ref() != Some(&failure) => {
                    log(Source::Window, format!("the emulated agent failed: {failure}"));
                    self.agent_failure = Some(failure);
                }
                Err(_) => {}
                Ok(()) => self.agent_failure = None,
            }
            ran
        } else {
            self.gb.run(budget)
        };
        let played = self.ahead + ran;
        self.ahead = if played >= frame { played - frame } else { MachineCycles::ZERO };

        for event in self.agent.drain_events() {
            say(Source::Emulated, event, touring, log);
        }
        for line in self.emulated_trace.drain() {
            log(Source::Emulated, line);
        }
        if !touring && let Some(message) = self.emulated_speech.hear_emulated(&PokemonApi::with_cache(&mut self.gb, &mut self.map_cache)) {
            log(Source::Emulated, message);
        }
    }

    fn native_frame(&mut self, held: Joypad, touring: bool, log: &mut impl FnMut(Source, String)) {
        // The Hall of Fame's script ends the game with a restart, and so does a blackout on the
        // title screen: what is left is an empty stack, and the console comes back on.
        if self.native.game().modes().is_empty() {
            *self.native.game_mut() = match self.tour.as_ref() {
                // A tour's game is not the one the window keeps, so its slots stay in memory.
                Some(tour) if touring => {
                    let mut game = Game::power_on(GameRng::from_entropy(), Pacing::Faithful);
                    game.set_slots(tour.slots.slots());
                    game
                }
                _ => native_power_on(self.native_slots.as_ref()),
            };
            self.native.host_took_the_screen();
        }
        match self.tour.as_mut().filter(|tour| !tour.native.progress.is_over()) {
            // The agent reads its own text, and says it as an event.
            Some(tour) => tour.native_frame(&mut self.native),
            None => {
                let frame = self.native.frame(Input::Buttons(held));
                if !touring && let Some(message) = self.native_speech.hear_native(&frame.printed, self.native.game()) {
                    log(Source::Native, message);
                }
            }
        }
        for request in self.native.take_slot_requests() {
            let store: &mut dyn SlotStore = match (self.tour.as_mut(), self.native_slots.as_mut()) {
                (Some(tour), _) if touring => &mut tour.slots,
                (_, Some(store)) => store,
                _ => continue,
            };
            let loading = matches!(request, SlotRequest::Load(_));
            // The window knows no time zone, so the slots are stamped in UTC.
            let saved_at = SavedAt::from_system_time(std::time::SystemTime::now(), 0);
            match store.answer(self.native.game_mut(), request, saved_at) {
                Err(e) => log(Source::Window, format!("a save slot: {e}")),
                Ok(()) if loading => {
                    self.native.host_took_the_screen();
                    self.native.drain_audio();
                    self.synth = poke_agent::native::synth_for(self.native.game());
                }
                Ok(()) => {}
            }
        }
        self.native_outputs(touring, log);
    }

    /// The recreation's sound into the synth, and what its agent said into `log`.
    fn native_outputs(&mut self, touring: bool, log: &mut impl FnMut(Source, String)) {
        for write in self.native.drain_audio() {
            self.synth.write(write);
        }
        self.synth.end_frame();
        for event in self.native.drain_events() {
            say(Source::Native, event, touring, log);
        }
        for line in self.native_trace.drain() {
            log(Source::Native, line);
        }
    }

    /// Interleaved stereo at the synth's rate, at the level a sink is fed, returning the frames read.
    pub fn read_samples(&mut self, out: &mut [f32]) -> usize {
        let frames = self.synth.read_samples(out);
        for sample in &mut out[..frames * 2] {
            *sample *= HEADROOM;
        }
        frames
    }

    /// Replaces `side`'s game with the state in the file at `path`. A file that holds the other
    /// game's state, or neither's, is refused with the reason, and the game left as it was.
    pub fn load_file(&mut self, side: Side, path: &Path) -> Result<(), String> {
        self.load_files(&[(side, path)])
    }

    /// Replaces each `(side, path)`'s game with the state in its file. Every file is read and its
    /// kind checked before either game is touched, so a missing or wrong one loads neither.
    pub fn load_files(&mut self, files: &[(Side, &Path)]) -> Result<(), String> {
        let states = files.iter().map(|&(side, path)| {
            let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
            let bytes = std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
            match Side::of_file(&bytes) {
                Some(kind) if kind == side => Ok((side, name, bytes)),
                Some(kind) => Err(format!("{name} is a {} state, for the {} half", kind.label(), kind.label())),
                None => Err(format!("{name} is neither an emulator save state nor a pokered save")),
            }
        }).collect::<Result<Vec<_>, String>>()?;
        for (side, name, bytes) in states {
            self.load(side, &bytes).map_err(|e| format!("could not load {name}: {e}"))?;
        }
        Ok(())
    }

    /// `side`'s game as the kind of file [`Side::of_file`] tells apart.
    pub fn save(&self, side: Side) -> Result<Vec<u8>, String> {
        match side {
            Side::Emulated => self.gb.save_state(),
            Side::Native => Ok(self.native.game().save()),
        }
    }

    /// Writes each `(side, path)`'s game to its file.
    pub fn save_files(&self, files: &[(Side, &Path)]) -> Result<(), String> {
        for &(side, path) in files {
            std::fs::write(path, self.save(side)?).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        }
        Ok(())
    }

    /// Replaces `side`'s game with the state in `bytes`, of the kind [`Side::of_file`] names.
    pub fn load(&mut self, side: Side, bytes: &[u8]) -> Result<(), String> {
        if self.touring() {
            return Err("a tour is playing both games: stop it before loading a game".to_string());
        }
        match side {
            Side::Emulated => {
                self.gb.load_state(bytes)?;
                self.emulator_restored();
            }
            Side::Native => {
                let mut game = Game::load(bytes, Pacing::Faithful)?;
                if let Some(store) = &self.native_slots {
                    game.set_slots(store.slots());
                }
                *self.native.game_mut() = game;
                self.native.host_took_the_screen();
                self.native.drain_audio();
                // A save carries no oscillator state, so the synth starts over from what the
                // engine is holding, or a note playing across the load goes silent.
                self.synth = poke_agent::native::synth_for(self.native.game());
            }
        }
        Ok(())
    }

    /// After the emulator's state is replaced: the sound stays off, and the pad is let go.
    pub fn emulator_restored(&mut self) {
        self.gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
        self.emulated_held = Joypad::empty();
        self.ahead = MachineCycles::ZERO;
    }
}

/// An agent's event, unless it is the text it read, which [`Speech`] has said already for whoever
/// was playing; on a tour the agent's reading is said in its place, as Speech says it.
fn say(source: Source, event: AgentEvent, touring: bool, log: &mut impl FnMut(Source, String)) {
    match event {
        AgentEvent::TextBox { message } if touring => log(source, message),
        AgentEvent::TextBox { .. } => {}
        event => log(source, event.to_string()),
    }
}

/// The recreation from power-on, with the slots on disk for CONTINUE to offer.
fn native_power_on(slots: Option<&DirectoryStore>) -> Game {
    let mut game = Game::power_on(GameRng::from_entropy(), Pacing::Faithful);
    if let Some(store) = slots {
        game.set_slots(store.slots());
    }
    game
}

/// One slice of the agent's play, on [`SERVED_OPTIONS`]. Only while the agent drives: a human at
/// the keyboard may set the OPTION menu however they like, and the agent still copes with SHIFT.
pub fn agent_slice(
    agent: &mut PokemonAgent,
    gb: &mut GameBoy,
    map_cache: &mut MapMetadataCache,
    min_cycles: MachineCycles,
) -> (MachineCycles, Result<(), String>) {
    keep_game_options(gb.core_mut().mmu_mut(), &SERVED_OPTIONS);
    agent.run(gb, map_cache, min_cycles)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Both games cold, with no save of either, so each offers NEW GAME first.
    pub fn fresh() -> Games {
        Games::power_on(None, None, Box::new(ConsolePolicy::default()), Box::new(ConsolePolicy::default())).unwrap()
    }

    /// A tapped for two frames in every `every`, routed to both, `frames` times over.
    pub fn tap_a(games: &mut Games, frames: usize, every: usize, log: &mut impl FnMut(Source, String)) {
        for frame in 0..frames {
            let held = if frame % every < 2 { Joypad::A } else { Joypad::empty() };
            games.frame(Routing::Both, held, log);
        }
    }

    /// A file in the system's temporary directory, removed when dropped.
    pub struct TempFile(pub std::path::PathBuf);

    impl TempFile {
        pub fn new(name: &str, bytes: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!("poke-agent-sdl-{}-{name}", std::process::id()));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn a_file_is_told_apart_by_its_magic() {
        let games = fresh();
        assert_eq!(Side::of_file(&games.gb.save_state().unwrap()), Some(Side::Emulated));
        assert_eq!(Side::of_file(&games.native.game().save()), Some(Side::Native));
        assert_eq!(Side::of_file(b"PKSL"), None);
        assert_eq!(Side::of_file(b""), None);
    }

    #[test]
    fn an_emulator_state_loads_into_the_emulated_game_alone() {
        let mut games = fresh();
        tap_a(&mut games, 300, 40, &mut |_, _| {});
        let state = TempFile::new("emulated.gbst", &games.gb.save_state().unwrap());
        let wram = games.gb.core().mmu().work_ram().to_vec();
        tap_a(&mut games, 300, 40, &mut |_, _| {});
        assert_ne!(games.gb.core().mmu().work_ram(), wram.as_slice());

        let refused = games.load_file(Side::Native, &state.0).unwrap_err();
        assert!(refused.contains("emulated half"), "{refused}");
        games.load_file(Side::Emulated, &state.0).unwrap();
        assert_eq!(games.gb.core().mmu().work_ram(), wram.as_slice());
        games.frame(Routing::Both, Joypad::empty(), &mut |_, _| {});
        let mut samples = vec![0.0f32; 48_000 / 8 * 2];
        assert_eq!(games.gb.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut samples), 0, "the emulator stays mute");
    }

    #[test]
    fn a_pokered_save_loads_into_the_native_game_alone() {
        let mut games = fresh();
        tap_a(&mut games, 300, 40, &mut |_, _| {});
        let saved = games.native.game().save();
        let state = TempFile::new("native.pkrd", &saved);
        tap_a(&mut games, 300, 40, &mut |_, _| {});
        assert_ne!(games.native.game().save(), saved);

        let refused = games.load_file(Side::Emulated, &state.0).unwrap_err();
        assert!(refused.contains("native half"), "{refused}");
        let garbage = TempFile::new("garbage.bin", b"not a save");
        assert!(games.load_file(Side::Native, &garbage.0).unwrap_err().contains("neither"));
        assert!(games.load_file(Side::Native, std::path::Path::new("/nonexistent/x.pkrd")).is_err());

        games.load_file(Side::Native, &state.0).unwrap();
        assert_eq!(games.native.game().save(), saved);
        tap_a(&mut games, 60, 40, &mut |_, _| {});
    }

    /// The quick-save: both games written together and read back together, and a pair with a file
    /// missing loads neither.
    #[test]
    fn both_games_save_to_files_and_load_back_together() {
        let mut games = fresh();
        tap_a(&mut games, 300, 40, &mut |_, _| {});
        let emulated = TempFile::new("both.gbst", b"");
        let native = TempFile::new("both.pkrd", b"");
        let files = [(Side::Emulated, emulated.0.as_path()), (Side::Native, native.0.as_path())];
        games.save_files(&files).unwrap();
        let (wram, saved) = (games.gb.core().mmu().work_ram().to_vec(), games.native.game().save());
        tap_a(&mut games, 300, 40, &mut |_, _| {});
        let (moved_wram, moved) = (games.gb.core().mmu().work_ram().to_vec(), games.native.game().save());
        assert_ne!(moved_wram, wram);
        assert_ne!(moved, saved);

        let missing = Path::new("/nonexistent/x.pkrd");
        assert!(games.load_files(&[(Side::Emulated, emulated.0.as_path()), (Side::Native, missing)]).is_err());
        assert_eq!(games.gb.core().mmu().work_ram(), moved_wram.as_slice(), "the emulated game left as it was");

        games.load_files(&files).unwrap();
        assert_eq!(games.gb.core().mmu().work_ram(), wram.as_slice());
        assert_eq!(games.native.game().save(), saved);
    }

    #[test]
    fn only_the_native_game_is_heard() {
        let mut games = fresh();
        let mut heard = false;
        let mut samples = vec![0.0f32; 48_000 / 8 * 2];
        for _ in 0..600 {
            games.frame(Routing::Both, Joypad::empty(), &mut |_, _| {});
            while games.read_samples(&mut samples) > 0 {
                heard |= samples.iter().any(|&sample| sample != 0.0);
            }
        }
        assert!(heard, "the recreation's intro reached the synth");
        assert_eq!(games.gb.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut samples), 0);
    }

    #[test]
    fn a_press_routed_to_one_game_never_reaches_the_other() {
        let mut games = fresh();
        games.frame(Routing::Native, Joypad::A, &mut |_, _| {});
        assert_eq!(games.emulated_held, Joypad::empty());
        games.frame(Routing::Emulated, Joypad::A | Joypad::UP, &mut |_, _| {});
        assert_eq!(games.emulated_held, Joypad::A | Joypad::UP);
        games.frame(Routing::Native, Joypad::A, &mut |_, _| {});
        assert_eq!(games.emulated_held, Joypad::empty(), "switching away lets go");
    }

    #[test]
    fn a_tour_takes_both_games_from_the_keyboard_and_from_loads_until_stopped() {
        let mut games = fresh();
        let saved = games.native.game().save();
        games.start_tour().unwrap();
        assert!(games.touring());
        assert!(games.load(Side::Native, &saved).unwrap_err().contains("tour"));
        games.frame(Routing::Both, Joypad::A, &mut |_, _| {});
        assert_eq!(games.emulated_held, Joypad::empty());

        games.tour.as_ref().unwrap().stop();
        let mut said = Vec::new();
        games.frame(Routing::Both, Joypad::empty(), &mut |source, line| said.push((source, line)));
        assert!(!games.touring());
        assert!(said.iter().any(|(source, line)| *source == Source::Emulated && line.contains("stopped from the window")), "{said:?}");
        assert!(said.iter().any(|(source, line)| *source == Source::Native && line.contains("stopped from the window")), "{said:?}");
        games.frame(Routing::Both, Joypad::A, &mut |_, _| {});
        assert_eq!(games.emulated_held, Joypad::A, "the keyboard is back");
        games.load(Side::Native, &saved).unwrap();
    }

    /// The emulator is held to a frame of cycles a host frame, the agent's overshoot paid back.
    #[test]
    fn the_emulator_plays_one_frame_of_cycles_a_frame() {
        let mut games = fresh();
        games.ahead = MachineCycles::PER_FRAME + MachineCycles::ONE;
        games.frame(Routing::Both, Joypad::empty(), &mut |_, _| {});
        assert_eq!(games.ahead, MachineCycles::ONE);
    }
}
