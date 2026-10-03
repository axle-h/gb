//! What the host plays: the cartridge on the emulator under `PokemonAgent`, or the recreation under
//! `NativeAgent`. The streams take a [`Frame`] and interleaved stereo samples from either and
//! nothing else.

use gb::cycles::MachineCycles;
use gb::game_boy::GameBoy;
use gb::lcd_palette::LcdColor;
use gb::model::Model;
use poke_agent::pokemon::agent::{AgentEvent, PokemonAgent};
use poke_agent::pokemon::map_metadata::MapMetadataCache;
use poke_agent::pokemon::native_agent::NativeAgent;
use poke_agent::pokemon::observe::{self, StatusView};
use poke_agent::pokemon::options::{SERVED_OPTIONS, keep_game_options};
use poke_agent::pokemon::policy::Policy;
use poke_agent::pokemon::{PokemonApi, PokemonApiTrait};
use poke_agent::run::hall_of_fame::PartyMember;
use poke_agent::run::{GameKind, RunDir, RunProgress, files};
use pokered::audio::Voices;
use pokered::audio::synth::{HEADROOM, Synth};
use pokered::command::{Command, Decision};
use pokered::gfx::colour::{BYTES_PER_PIXEL, ColourMode};
use pokered::mode::{Mode, Status};
use pokered::rng::GameRng;
use pokered::save_slots::{MemoryStore, SavedAt, SlotRequest, SlotStore};
use pokered::{Game, Input, Pacing};

use crate::web::audio;
use crate::web::video::{Frame, PIXELS};

/// A winning run's tally for the ledger: badges, dex owned and seen, money, party, clock maxed.
pub type FinalState = (u32, usize, usize, u32, Vec<PartyMember>, bool);

pub enum Console {
    Emulated(Box<Emulated>),
    Native(Box<Native>),
}

pub struct Emulated {
    pub gb: GameBoy,
    pub agent: PokemonAgent,
    pub map_cache: MapMetadataCache,
    model: Model,
    target_speed: f64,
}

pub struct Native {
    pub agent: NativeAgent,
    colours: ColourMode,
    /// Built on the edge into listening and dropped on the edge out, so nothing is synthesised
    /// for nobody. A save carries no oscillator state, so a new one is told what the engine holds.
    /// It makes a second of samples per second of game, so the stream is right only at 1x.
    synth: Option<Synth>,
    /// The game's save slots, kept in the run directory at each checkpoint.
    slots: MemoryStore,
}

/// Re-apply everything about the APU that a save state does not carry.
fn tune_audio(gb: &mut GameBoy, target_speed: f64) {
    let audio = gb.core_mut().mmu_mut().audio_mut();
    audio.set_output_sample_rate(audio::SAMPLE_RATE);
    audio.set_emulation_speed(target_speed.max(f64::MIN_POSITIVE));
}

/// The pacing a served native game plays at: what a person watching would see.
const SERVED_PACING: Pacing = Pacing::Faithful;

/// A new native game, through the intro to Red's room on the served options, as bytes a
/// [`Console::new`] of [`GameKind::Native`] starts from.
pub fn native_start_of_game() -> Result<Vec<u8>, String> {
    let game = poke_agent::pokemon::native_agent::new_game(GameRng::from_entropy, SERVED_OPTIONS.native(), SERVED_PACING)?;
    Ok(game.save())
}

impl Console {
    /// `state` is a `state.gbst` for an emulated game and a `game.pkrd` for a native one.
    pub fn new(kind: GameKind, state: &[u8], policy: Box<dyn Policy>, model: Model, target_speed: f64) -> Result<Self, String> {
        match kind {
            GameKind::Emulated => {
                let mut gb = GameBoy::new(poke_agent::pokemon::roms::POKERED, model);
                gb.load_state(state).map_err(|e| format!("could not load the starting state: {e}"))?;
                tune_audio(&mut gb, target_speed);
                Ok(Self::Emulated(Box::new(Emulated {
                    gb,
                    agent: PokemonAgent::new(policy),
                    map_cache: MapMetadataCache::default(),
                    model,
                    target_speed,
                })))
            }
            GameKind::Native => {
                let mut game = Game::load(state, SERVED_PACING).map_err(|e| format!("could not load the starting game: {e}"))?;
                let slots = MemoryStore::default();
                game.set_slots(slots.slots());
                Ok(Self::Native(Box::new(Native {
                    agent: NativeAgent::new(game, policy)?.hosted(),
                    colours: match model {
                        Model::Dmg => ColourMode::Dmg,
                        Model::Cgb => ColourMode::Gbc,
                    },
                    synth: None,
                    slots,
                })))
            }
        }
    }

    /// Hold the game to [`SERVED_OPTIONS`], a new run and a resume alike.
    pub fn keep_served_options(&mut self) {
        match self {
            Self::Emulated(e) => {
                keep_game_options(e.gb.core_mut().mmu_mut(), &SERVED_OPTIONS);
            }
            Self::Native(n) => n.agent.game_mut().world_mut().options = SERVED_OPTIONS.native(),
        }
    }

    /// Put `name` on the trainer card.
    pub fn write_player_name(&mut self, name: &str) -> Result<(), String> {
        match self {
            Self::Emulated(e) => PokemonApi::with_cache(&mut e.gb, &mut e.map_cache).write_player_name(name),
            Self::Native(n) => {
                let mut bytes = poke_agent::pokemon::player_name_bytes(name)?;
                // The recreation keeps a name unterminated.
                bytes.pop();
                n.agent.game_mut().world_mut().player_name = bytes;
                Ok(())
            }
        }
    }

    pub fn policy_name(&self) -> &'static str {
        match self {
            Self::Emulated(e) => e.agent.policy_name(),
            Self::Native(n) => n.agent.policy().name(),
        }
    }

    pub fn policy_player_name(&self) -> Option<String> {
        match self {
            Self::Emulated(e) => e.agent.policy_player_name(),
            Self::Native(n) => n.agent.policy().player_name(),
        }
    }

    pub fn state_debug(&self) -> String {
        match self {
            Self::Emulated(e) => e.agent.state_debug(),
            Self::Native(n) => n.agent.state_debug(),
        }
    }

    /// Play at least `budget`, returning what was played and how the agent fared.
    pub fn advance(&mut self, budget: MachineCycles) -> (MachineCycles, Result<(), String>) {
        match self {
            Self::Emulated(e) => e.agent.run(&mut e.gb, &mut e.map_cache, budget),
            Self::Native(n) => n.advance(budget),
        }
    }

    pub fn drain_events(&mut self) -> Vec<AgentEvent> {
        match self {
            Self::Emulated(e) => e.agent.drain_events(),
            Self::Native(n) => n.agent.drain_events(),
        }
    }

    /// The whole game, as the run directory keeps it.
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Emulated(e) => e.gb.save_state(),
            Self::Native(n) => Ok(n.agent.game().save()),
        }
    }

    /// Write `state`, from [`Self::save_state`], into `run`.
    pub fn checkpoint(&self, run: &RunDir, state: &[u8], progress: RunProgress) -> Result<(), String> {
        match self {
            Self::Emulated(e) => run.checkpoint(state, &e.gb.dump_sram(), progress),
            Self::Native(n) => run.checkpoint_game(state, &n.slots, progress),
        }
    }

    /// The files a finished run is filed with, by name.
    pub fn archive_files(&self, state: Vec<u8>) -> Vec<(&'static str, Vec<u8>)> {
        match self {
            Self::Emulated(e) => vec![(files::STATE, state), (files::SRAM, e.gb.dump_sram())],
            Self::Native(_) => vec![(files::GAME, state)],
        }
    }

    /// The winning run's final tally, for the ledger row.
    pub fn final_state(&mut self) -> FinalState {
        let (tally, maxed) = match self {
            Self::Emulated(e) => {
                use poke_agent::pokemon::symbols::{DmgPointerRead, pokered_symbols};
                let api = PokemonApi::with_cache(&mut e.gb, &mut e.map_cache);
                let maxed = api.mmu().read_pointer(&pokered_symbols::wPlayTimeMaxed) != 0;
                let tally = api.game_state().map(|state| (
                    state.badges,
                    state.pokedex_owned.species().len(),
                    state.pokedex_seen.species().len(),
                    state.money,
                    state.pokemon,
                ));
                (tally, maxed)
            }
            // Not `game_state`: the ceremony has replaced the overworld it needs.
            Self::Native(n) => (n.agent.native().final_tally(), n.agent.game().world().play_time.maxed),
        };
        let Ok((badges, owned, seen, money, pokemon)) = tally else { return (0, 0, 0, 0, Vec::new(), maxed) };
        let party = pokemon
            .iter()
            .map(|mon| PartyMember {
                nickname: mon.nickname.to_default_string(),
                species: format!("{:?}", mon.species),
                level: mon.level,
            })
            .collect();
        (badges.bits().count_ones(), owned, seen, money, party, maxed)
    }

    /// The heartbeat's view of the game; `None` mid-transition, when it cannot be read.
    pub fn status(&mut self) -> Option<StatusView> {
        match self {
            Self::Emulated(e) => {
                let api = PokemonApi::with_cache(&mut e.gb, &mut e.map_cache);
                api.game_state().ok().map(|state| observe::status(&state, &api))
            }
            Self::Native(n) => n.agent.game_state().ok().map(|state| observe::status(&state, n.agent.native())),
        }
    }

    /// Start the game over from its beginning under the same policy, now writing to `run_dir`.
    pub fn restart(&mut self, run_dir: &std::path::Path) -> Result<(), String> {
        match self {
            Self::Emulated(e) => {
                e.gb = GameBoy::new(poke_agent::pokemon::roms::POKERED, e.model);
                e.gb
                    .load_state(poke_agent::pokemon::data::START_OF_GAME)
                    .map_err(|e| format!("could not load the start-of-game state: {e}"))?;
                e.map_cache = MapMetadataCache::default();
                keep_game_options(e.gb.core_mut().mmu_mut(), &SERVED_OPTIONS);
                e.agent.restart(Some(run_dir));
                // The reload above dropped both APU settings.
                tune_audio(&mut e.gb, e.target_speed);
                Ok(())
            }
            Self::Native(n) => {
                let game = Game::load(&native_start_of_game()?, SERVED_PACING)?;
                n.agent.restart(game, Some(run_dir))?;
                n.synth = None;
                n.slots = MemoryStore::default();
                n.agent.game_mut().set_slots(n.slots.slots());
                Ok(())
            }
        }
    }

    pub fn clear_conversation(&mut self, run_dir: Option<&std::path::Path>) -> Result<(), String> {
        match self {
            Self::Emulated(e) => e.agent.clear_conversation(run_dir),
            Self::Native(n) => n.agent.clear_conversation(run_dir),
        }
    }

    /// A resumed native run's slots, read back from its run directory. A slot that cannot be read
    /// is left empty, as the slot list shows it.
    pub fn restore_slots(&mut self, kept: &dyn SlotStore) {
        if let Self::Native(n) = self {
            for (index, slot) in kept.slots().into_iter().enumerate() {
                let index = index as u8;
                if let (Some(slot), Ok(bytes)) = (slot, kept.read(index)) {
                    n.slots.write(index, &bytes, slot).expect("a slot in memory is written");
                }
            }
            n.agent.game_mut().set_slots(n.slots.slots());
        }
    }

    /// The screen as it stands.
    pub fn screen(&self) -> Box<Frame> {
        match self {
            // Copied out: the encoder borrows the host mutably and the LCD lives inside the `GameBoy`.
            Self::Emulated(e) => Box::new(*e.gb.core().mmu().ppu().lcd()),
            Self::Native(n) => {
                let rgba = n.colours.rgba(n.agent.game().screen());
                let mut frame = Box::new([LcdColor::WHITE; PIXELS]);
                for (pixel, rgba) in frame.iter_mut().zip(rgba.chunks_exact(BYTES_PER_PIXEL)) {
                    *pixel = LcdColor::rgb(rgba[0], rgba[1], rgba[2]);
                }
                frame
            }
        }
    }

    /// Make samples for [`Self::read_samples`], or stop making them.
    pub fn set_audio_output(&mut self, enabled: bool) {
        match self {
            Self::Emulated(e) => e.gb.core_mut().mmu_mut().audio_mut().set_output_enabled(enabled),
            Self::Native(n) => match (enabled, n.synth.is_some()) {
                (true, false) => n.synth = Some(poke_agent::native::synth_for(n.agent.game())),
                (false, true) => n.synth = None,
                _ => {}
            },
        }
    }

    /// Interleaved stereo at [`audio::SAMPLE_RATE`], returning how many frames were read.
    pub fn read_samples(&mut self, out: &mut [f32]) -> usize {
        match self {
            Self::Emulated(e) => e.gb.core_mut().mmu_mut().audio_mut().read_samples_f32(out),
            Self::Native(n) => {
                let Some(synth) = n.synth.as_mut() else { return 0 };
                let frames = synth.read_samples(out);
                for sample in &mut out[..frames * 2] {
                    *sample *= HEADROOM;
                }
                frames
            }
        }
    }

    #[cfg(test)]
    pub fn emulated(&self) -> &Emulated {
        match self {
            Self::Emulated(e) => e,
            Self::Native(_) => panic!("an emulated console was expected"),
        }
    }

    #[cfg(test)]
    pub fn emulated_mut(&mut self) -> &mut Emulated {
        match self {
            Self::Emulated(e) => e,
            Self::Native(_) => panic!("an emulated console was expected"),
        }
    }

    #[cfg(test)]
    pub fn api(&mut self) -> PokemonApi<'_> {
        let e = self.emulated_mut();
        PokemonApi::with_cache(&mut e.gb, &mut e.map_cache)
    }

    #[cfg(test)]
    pub fn native(&mut self) -> &mut Native {
        match self {
            Self::Native(n) => n,
            Self::Emulated(_) => panic!("a native console was expected"),
        }
    }
}

impl Native {
    #[cfg(test)]
    pub fn synthesising(&self) -> bool {
        self.synth.is_some()
    }

    #[cfg(test)]
    pub fn slots(&self) -> &MemoryStore {
        &self.slots
    }

    /// The game's request answered at the host's clock, in UTC, which is all the host knows. A
    /// loaded game carries no oscillator state, so a synth starts over from what its engine holds.
    fn answer(&mut self, request: SlotRequest) -> Result<(), String> {
        let loading = matches!(request, SlotRequest::Load(_));
        let saved_at = SavedAt::from_system_time(std::time::SystemTime::now(), 0);
        self.slots.answer(self.agent.game_mut(), request, saved_at).map_err(|e| format!("a save slot: {e}"))?;
        if loading {
            self.agent.host_took_the_screen();
            self.agent.drain_audio();
            if self.synth.is_some() {
                self.synth = Some(poke_agent::native::synth_for(self.agent.game()));
            }
        }
        Ok(())
    }

    /// Whole frames until `budget` is spent. The ceremony, the credits and the title screen they
    /// end on take no decisions, so the host plays them itself, and an empty mode stack is the
    /// console coming back on. CONTINUE is answered by loading the newest slot, the autosave the
    /// credits wrote, as choosing it in the slots would.
    fn advance(&mut self, budget: MachineCycles) -> (MachineCycles, Result<(), String>) {
        let mut ran = MachineCycles::ZERO;
        let mut result = Ok(());
        while ran < budget {
            ran += MachineCycles::PER_FRAME;
            if self.agent.game().modes().is_empty() {
                let world = self.agent.game().world().clone();
                let mut game = Game::new(world, GameRng::from_entropy(), SERVED_PACING);
                game.push(Mode::Movie(pokered::modes::movie::Movie::power_on()));
                game.set_slots(self.slots.slots());
                *self.agent.game_mut() = game;
                self.agent.host_took_the_screen();
            }
            if let Some(Mode::Movie(_) | Mode::MainMenu(_)) = self.agent.game().modes().last() {
                self.agent.host_took_the_screen();
                let continued = match self.agent.game().status() {
                    Status::Waiting(Decision::MainMenu) => pokered::save_slots::newest(self.agent.game().slots()),
                    _ => None,
                };
                if let Some(slot) = continued {
                    if let Err(failure) = self.answer(SlotRequest::Load(slot)) {
                        result = Err(failure);
                    }
                    continue;
                }
                let command = match self.agent.game().status() {
                    // With no slot: a new game, on the first preset name, as `new_game` answers it.
                    Status::Waiting(Decision::MainMenu) => Some(Command::ChooseOption(0)),
                    Status::Waiting(Decision::IntroNameMenu) => Some(Command::ChooseOption(1)),
                    Status::Waiting(_) => Some(Command::Advance),
                    _ => None,
                };
                self.agent.frame(command.map_or(Input::None, Input::Command));
            } else if let Err(failure) = self.agent.tick() {
                result = Err(failure);
            }
            for request in self.agent.take_slot_requests() {
                if let Err(failure) = self.answer(request) {
                    result = Err(failure);
                }
            }
            let writes = self.agent.drain_audio();
            if let Some(synth) = self.synth.as_mut() {
                for write in writes {
                    synth.write(write);
                }
                synth.end_frame();
            }
        }
        (ran, result)
    }
}
