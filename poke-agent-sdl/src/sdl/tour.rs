//! The grand tour on both games at once, kept together a step at a time by [`Paced`]: the
//! cartridge's from the fresh save as `grand_tour` plays it, in the host's 20 ms ticks, and the
//! recreation's from a new game a frame at a time as `native_grand_tour` plays it, each answered by
//! a brain of its own in this process.

use std::time::{Duration, Instant};
use gb::cycles::MachineCycles;
use gb::game_boy::GameBoy;
use poke_agent::llm::battle_script::BattleScript;
use poke_agent::llm::config::{DEFAULT_COMPACT_ABOVE, DEFAULT_MAX_TOKENS, DEFAULT_REQUEST_TIMEOUT_SECS, LlmConfig};
use poke_agent::llm::history::History;
use poke_agent::llm::todo::TodoList;
use poke_agent::llm::worker;
use poke_agent::pokemon::agent::PokemonAgent;
use poke_agent::pokemon::llm_policy::LlmPolicy;
use poke_agent::pokemon::map_metadata::MapMetadataCache;
use poke_agent::pokemon::native_agent::NativeAgent;
use poke_agent::pokemon::options::SERVED_OPTIONS;
use poke_agent::pokemon::policy::Policy;
use poke_agent::pokemon::{PokemonApi, PokemonApiTrait};
use poke_agent::published::Published;
use poke_agent::tour::brain::{FinishFlag, TourProgress};
use poke_agent::tour::cheats::Cheats;
use poke_agent::tour::completion::{Ledger, WorldFlags, checklist};
use poke_agent::tour::endpoint::BrainEndpoint;
use pokered::save_slots::MemoryStore;
use poke_agent::tour::pace::{Paced, Progress, waits};
use poke_agent::tour::phases::all_phases;
use poke_agent::tour::{Ceremony, hold_stock, hold_stock_native, native_ceremony};
use pokered::rng::GameRng;
use pokered::world::{BattleStyle, Options, TextSpeed};
use pokered::{Game, Pacing};
use crate::sdl::games::Side;

/// The tick the deployed host drives the cartridge's agent in, and so the one `grand_tour` is
/// proven at.
const TICK: Duration = Duration::from_millis(20);
/// The recreation's tour is proven on this seed.
const NATIVE_SEED: u64 = 1;
/// Money the story cheats keep topped up, as both tours' tests keep it.
const MONEY: u32 = 999_999;

/// Where one side's tour stands, for the window to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    Playing,
    /// Ahead of the other side, held until it catches up.
    Waiting,
    Finished,
    /// Stuck, failed, or stopped from the window, and why.
    Over(String),
}

impl Standing {
    pub fn label(&self) -> &str {
        match self {
            Standing::Playing => "playing",
            Standing::Waiting => "waiting",
            Standing::Finished => "finished",
            Standing::Over(why) => why,
        }
    }
}

/// What a side's driver keeps between frames, beyond its progress.
pub struct Leg {
    pub progress: TourProgress,
    cheats: Cheats,
    /// Said into the log once, when the side's tour ends.
    reported: bool,
}

impl Leg {
    fn new(progress: TourProgress) -> Self {
        Self { progress, cheats: Cheats::story(MONEY), reported: false }
    }

    pub fn turns(&self) -> usize {
        *self.progress.turns.lock().expect("not poisoned")
    }

    /// Ends this side's tour for `why`, unless it is over already.
    fn end(&self, why: String) {
        let mut stuck = self.progress.stuck.lock().expect("not poisoned");
        if !*self.progress.finished.lock().expect("not poisoned") && stuck.is_none() {
            *stuck = Some(why);
        }
    }
}

pub struct Tour {
    pub emulated: Leg,
    pub native: Leg,
    /// The recreation's save slots for the tour alone; the Hall of Fame's autosave is continued from.
    pub slots: MemoryStore,
    ceremony: Ceremony,
    warped_from: (u8, u8),
    pub started: Instant,
    /// The cartridge's game time since the fresh save.
    pub emulated_played: MachineCycles,
    /// Set once both are over and the games are back with the keyboard.
    pub handed_back: bool,
}

/// The two policies a tour plays through, each [`Paced`] by the other side's progress.
pub struct Policies {
    pub emulated: Box<dyn Policy>,
    pub native: Box<dyn Policy>,
}

impl Tour {
    /// Both brains behind their workers, the policies the games are to be played by, and the tour
    /// that reads their progress. The workers end when the policies are dropped.
    pub fn new() -> Result<(Self, Policies), String> {
        let (emulated_brain, emulated) = FinishFlag::new(all_phases());
        let (native_brain, native) = FinishFlag::new(all_phases());
        let emulated_llm = llm_policy(Box::new(emulated_brain))?;
        let native_llm = llm_policy(Box::new(native_brain))?;
        let policies = Policies {
            emulated: Box::new(Paced::new(emulated_llm, emulated.clone(), native.clone())),
            native: Box::new(Paced::new(native_llm, native.clone(), emulated.clone())),
        };
        let tour = Self {
            emulated: Leg::new(emulated),
            native: Leg::new(native),
            slots: MemoryStore::default(),
            ceremony: Ceremony::default(),
            warped_from: (0, 0),
            started: Instant::now(),
            emulated_played: MachineCycles::ZERO,
            handed_back: false,
        };
        Ok((tour, policies))
    }

    /// Either side still going.
    pub fn running(&self) -> bool {
        !self.emulated.progress.is_over() || !self.native.progress.is_over()
    }

    /// `side`'s steps taken, of the tour's.
    pub fn progress(&self, side: Side) -> (usize, usize) {
        let leg = match side {
            Side::Emulated => &self.emulated,
            Side::Native => &self.native,
        };
        (leg.progress.steps(), leg.progress.total)
    }

    pub fn standing(&self, side: Side) -> Standing {
        let (mine, other) = match side {
            Side::Emulated => (&self.emulated, &self.native),
            Side::Native => (&self.native, &self.emulated),
        };
        if let Some(why) = mine.progress.stuck.lock().expect("not poisoned").clone() {
            Standing::Over(why)
        } else if *mine.progress.finished.lock().expect("not poisoned") {
            Standing::Finished
        } else if waits(&mine.progress, &other.progress) {
            Standing::Waiting
        } else {
            Standing::Playing
        }
    }

    /// Ends whatever is still going, as the window's stop does.
    pub fn stop(&self) {
        self.emulated.end("stopped from the window".to_string());
        self.native.end("stopped from the window".to_string());
    }

    /// The fresh save `grand_tour` starts from, on the machine it is proven on.
    pub fn emulated_game() -> Result<Box<GameBoy>, String> {
        let mut gb = Box::new(GameBoy::dmg(poke_agent::pokemon::roms::POKERED));
        gb.load_state(poke_agent::pokemon::data::START_OF_GAME)?;
        Ok(gb)
    }

    /// The new game `native_grand_tour` starts from: Red's room, at a served game's pace.
    pub fn native_game() -> Result<Game, String> {
        let options = Options { text_speed: TextSpeed::Fast, battle_animation: true, battle_style: BattleStyle::Set };
        poke_agent::pokemon::native_agent::new_game(|| GameRng::seeded(NATIVE_SEED), options, Pacing::Faithful)
    }

    /// The ledgers, once both games are in place.
    pub fn begin(&self, gb: &GameBoy, native: &NativeAgent) {
        *self.emulated.progress.ledger.lock().expect("not poisoned") = Ledger::new(&checklist(gb.core().mmu()));
        *self.native.progress.ledger.lock().expect("not poisoned") = Ledger::new(&checklist(native.native().rom()));
    }

    /// One tick of the cartridge's tour: the ceremony's presses, the ledger and the stock before it,
    /// the cheats after, as `grand_tour`'s driver does between ticks.
    pub fn emulated_tick(&mut self, gb: &mut GameBoy, agent: &mut PokemonAgent, map_cache: &mut MapMetadataCache)
        -> (MachineCycles, Result<(), String>)
    {
        let mut api = PokemonApi::with_cache(gb, map_cache);
        let state = api.game_state();
        if !self.ceremony.is_over() {
            self.ceremony.press(&mut api, state.as_ref().ok().map(|state| state.map.map));
        }
        if let Ok(state) = &state {
            self.emulated.progress.ledger.lock().expect("not poisoned").observe(state, api.mmu());
            hold_stock(&mut api, state);
        }
        api.debug_set_options(&SERVED_OPTIONS);
        let played = agent.run(gb, map_cache, MachineCycles::from_duration(TICK));
        self.emulated_played += played.0;
        let mut api = PokemonApi::with_cache(gb, map_cache);
        // Mid-transition or mid-load there is nothing to hold the game to yet.
        if let Ok(state) = api.game_state() {
            self.emulated.cheats.apply(&mut api, &state);
        }
        played
    }

    /// One frame of the recreation's tour, the ledger and the cheats before it as
    /// `native_grand_tour`'s driver does. A failure ends this side's tour.
    pub fn native_frame(&mut self, agent: &mut NativeAgent) {
        self.warped_from = agent.native().warped_from().unwrap_or(self.warped_from);
        {
            let world = agent.game().world();
            let flags = WorldFlags { world, warped_from: self.warped_from };
            self.native.progress.ledger.lock().expect("not poisoned")
                .observe_on(world.location.map, world.bag.items.iter().map(|item| item.id as u8), &flags);
        }
        if agent.is_free() && agent.game().frames() % 8 == 0 && let Ok(state) = agent.game_state() {
            let world = agent.game_mut().world_mut();
            self.native.cheats.apply_native(world, &state, true);
            hold_stock_native(world);
        }
        if native_ceremony(agent, &mut self.slots) {
            return;
        }
        if let Err(why) = agent.tick() {
            self.native.end(format!("the agent found no answer: {why}"));
        }
    }

    /// A line for each side whose tour has just ended, once.
    pub fn take_endings(&mut self, native_played: Duration) -> Vec<(Side, String)> {
        let mut said = Vec::new();
        let sides = [(Side::Emulated, self.emulated_played.to_duration()), (Side::Native, native_played)];
        for (side, played) in sides {
            let standing = self.standing(side);
            let leg = match side {
                Side::Emulated => &mut self.emulated,
                Side::Native => &mut self.native,
            };
            if leg.reported || !leg.progress.is_over() {
                continue;
            }
            leg.reported = true;
            let how = match standing {
                Standing::Over(why) => format!("ended: {why}"),
                _ => "finished".to_string(),
            };
            let seconds = played.as_secs();
            said.push((side, format!(
                "the {} tour {how}, {} of {} steps and {} turns in {}:{:02}:{:02} of game time",
                side.label(), leg.progress.steps(), leg.progress.total, leg.turns(),
                seconds / 3600, seconds / 60 % 60, seconds % 60,
            )));
        }
        said
    }
}

/// `LlmPolicy` on a worker answered by `brain` in this process, configured as the tests' runs are.
fn llm_policy(brain: Box<dyn poke_agent::tour::turn::Brain>) -> Result<Box<dyn Policy>, String> {
    let config = LlmConfig {
        base_url: String::new(),
        api_key: "mock".to_string(),
        model: "mock".to_string(),
        context_limit: 128_000,
        compact_above: DEFAULT_COMPACT_ABOVE,
        temperature: None,
        max_tool_steps: 6,
        request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
        max_tokens: Some(DEFAULT_MAX_TOKENS),
        reasoning_effort: None,
        stuck_timeout: None,
    };
    let (worker, handles) = worker::channels(
        Box::new(BrainEndpoint::new(brain)),
        config,
        Published::new(),
        TodoList::open(None),
        BattleScript::open(None),
        History::open(None),
    );
    // Not joined: the worker ends when the policy it answers is dropped.
    drop(worker.spawn()?);
    Ok(Box::new(LlmPolicy::new(handles, None)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pokered::input::Joypad;
    use crate::sdl::games::tests::fresh;
    use crate::sdl::log::Source;
    use crate::sdl::routing::Routing;

    /// Both tours from their fresh games to the end, unthrottled, through the frames the window
    /// plays, each side's steps and turns printed.
    #[test]
    #[ignore = "about half an hour: both grand tours to the end of the tour"]
    fn both_tours_play_to_the_end_together() {
        let mut games = fresh();
        games.start_tour().unwrap();
        let started = Instant::now();
        let mut widest = 0;
        let mut samples = vec![0.0f32; 48_000 / 8 * 2];
        while games.touring() && started.elapsed() < Duration::from_secs(3 * 3600) {
            games.frame(Routing::Both, Joypad::empty(), &mut |source: Source, line: String| {
                if line.starts_with("the ") && line.contains(" tour ") {
                    println!("[tour] {source:?}: {line}");
                }
            });
            // As the window drains it, or the synth holds every sample made.
            while games.read_samples(&mut samples) > 0 {}
            let tour = games.tour.as_ref().unwrap();
            if !tour.emulated.progress.is_over() && !tour.native.progress.is_over() {
                widest = widest.max(tour.emulated.progress.steps().abs_diff(tour.native.progress.steps()));
            }
        }
        let tour = games.tour.as_ref().unwrap();
        for (side, leg) in [(Side::Emulated, &tour.emulated), (Side::Native, &tour.native)] {
            println!("[tour] {}: {} of {} steps, {} turns, {:?}", side.label(), leg.progress.steps(),
                     leg.progress.total, leg.turns(), tour.standing(side));
        }
        println!("[tour] {:?} of wall clock, at most {widest} steps apart", started.elapsed());
        for side in [Side::Emulated, Side::Native] {
            assert_eq!(tour.standing(side), Standing::Finished, "{}", side.label());
        }
        assert_eq!(tour.emulated.progress.steps(), tour.emulated.progress.total);
        assert_eq!(tour.native.progress.steps(), tour.native.progress.total);
    }
}
