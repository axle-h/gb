//! The grand tour: a fresh save played through the whole game by a scripted model, through
//! `LlmPolicy` and the worker, with every story gate live and the ledger kept.
//!
//! A driver builds [`brain::FinishFlag::new`] from [`phases::all_phases`], hands the brain to the
//! worker behind [`endpoint::BrainEndpoint`] (or a server), and between ticks applies
//! [`cheats::Cheats::story`], [`hold_stock`] and the Hall of Fame's ceremony, feeding the ledger
//! as it goes.

pub mod brain;
pub mod cheats;
pub mod completion;
pub mod endpoint;
pub mod intent;
pub mod pace;
pub mod phases;
pub mod turn;

use gb::joypad::JoypadButton;

use crate::pokemon::encoding::GameMode;
use crate::pokemon::map::Map;
use crate::pokemon::native_agent::NativeAgent;
use crate::pokemon::symbols::DmgPointerRead;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};
use crate::tour::brain::STOCK;

/// Top the bag up to [`STOCK`] on the cartridge, in the overworld only.
pub fn hold_stock(api: &mut PokemonApi<'_>, state: &GameState) {
    if state.mode != GameMode::Overworld {
        return;
    }
    for (item, low, full) in STOCK {
        let held = state.bag.iter().find(|held| held.id == item).map_or(0, |held| held.quantity);
        if held < low {
            // A full bag with none held refuses, and the next tick asks again.
            api.debug_give_item(item, full - held).ok();
        }
    }
}

/// [`hold_stock`] on the recreation.
pub fn hold_stock_native(world: &mut pokered::world::World) {
    for (item, low, full) in STOCK {
        let held = world.bag.quantity_of(item);
        if held < low {
            world.bag.add(item, full - held);
        }
    }
}

/// The ceremony, the credits, the save and the title screen it resets to, on the cartridge: the
/// agent stops at the Hall of Fame and a deployed run ends there, so the buttons that see a run
/// into the postgame are the driver's own. Mash, a tick on and a tick off, so every press is a
/// fresh rising edge. A run that starts from a won game is past it, so the ceremony is over before
/// it began.
#[derive(Debug, Default)]
pub struct Ceremony {
    /// `None` until a win is seen, then whether the ceremony is still playing.
    playing: Option<bool>,
    mash: u32,
}

impl Ceremony {
    pub fn is_over(&self) -> bool {
        self.playing == Some(false)
    }

    /// Before every tick, while [`Self::is_over`] is false. `map` is `None` while it is unreadable.
    pub fn press(&mut self, api: &mut PokemonApi<'_>, map: Option<Map>) {
        let won = api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wNumHoFTeams) > 0;
        // WRAM is cleared by the reset, so the counter reads zero again until the save is loaded:
        // what says the ceremony is over is a playable overworld somewhere else.
        let playable = api.game_mode() == Some(GameMode::Overworld) && map.is_some_and(|map| map != Map::HallOfFame);
        match self.playing {
            None if won && !playable => self.playing = Some(true),
            None if won => self.playing = Some(false),
            Some(true) if playable => { self.playing = Some(false); api.release_all_buttons() }
            Some(true) => {
                self.mash += 1;
                if self.mash % 2 == 0 { api.press_button(JoypadButton::A) }
                else { api.release_all_buttons() }
            }
            _ => {}
        }
    }
}

/// [`Ceremony`] on the recreation, in place of the agent's tick: `true` when the frame was the
/// driver's. `slots` keeps the Hall of Fame's autosave, and after the credits the main menu is
/// answered by loading it, as CONTINUE and LOAD would.
pub fn native_ceremony(agent: &mut NativeAgent, slots: &mut pokered::save_slots::MemoryStore) -> bool {
    use pokered::command::{Command, Decision};
    use pokered::mode::{Mode, Status};
    use pokered::save_slots::{newest, SavedAt, SlotRequest, SlotStore};
    match agent.game().modes().last() {
        Some(Mode::Movie(movie)) if !movie.is_trade() => {}
        Some(Mode::MainMenu(_)) => {}
        _ => return false,
    }
    agent.host_took_the_screen();
    let saved_at = SavedAt::from_system_time(std::time::SystemTime::now(), 0);
    let continued = match agent.game().status() {
        Status::Waiting(Decision::MainMenu) => newest(agent.game().slots()),
        _ => None,
    };
    if let Some(slot) = continued {
        slots.answer(agent.game_mut(), SlotRequest::Load(slot), saved_at).expect("a slot in memory loads");
        agent.host_took_the_screen();
        return true;
    }
    let command = match agent.game().status() {
        Status::Waiting(Decision::MainMenu) => Some(Command::ChooseOption(0)),
        Status::Waiting(_) => Some(Command::Advance),
        _ => None,
    };
    let frame = agent.game_mut().frame(command.map_or(pokered::Input::None, pokered::Input::Command));
    if let Some(request) = frame.slot {
        slots.answer(agent.game_mut(), request, saved_at).expect("a slot in memory is kept");
    }
    true
}
