use std::collections::VecDeque;
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use crate::pokemon::delay::DelayContext;
use crate::pokemon::encoding::{GameMode, MetaTile};
use crate::pokemon::map::Map;
use crate::pokemon::menu::BattleMenuState;
use crate::pokemon::policy::{Policy, RandomPolicy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::text::PokemonTextReader;

// too long and player veers off course on the overworld, too short and the game doesn't get chance to update values between turns
pub const AGENT_RESOLUTION: MachineCycles = MachineCycles::from_duration(Duration::from_millis(20));

pub struct PokemonAgent {
    state: AgentState,
    event_buffer: VecDeque<AgentEvent>,
    cycles: MachineCycles,
    policy: Box<dyn Policy>,
}

#[derive(Debug)]
pub enum AgentEvent {
    StartedOverworldAction { destination: MetaTile },
    OverworldActionAborted { destination: MetaTile, reason: String },
    OverworldActionCompleted { destination: MetaTile },
    BattleStarted,
    BattleActionStarted { action: BattleAction },
    BattleEnded,
    TextBox { message: String }
}

impl AgentEvent {
    pub fn text_box_from_reader(reader: &PokemonTextReader) -> Self {
        Self::TextBox { message: reader.to_string() }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum BattleState {
    /// Waiting for the battle menu (TextBoxID 0x0B/0x1B) to appear.
    WaitingForMenu { reader: PokemonTextReader, delay: DelayContext },

    /// Battle menu is up but policy hasn't returned an action yet.
    AwaitingPolicy { delay: DelayContext },

    /// Navigating the menus
    Navigating { action: BattleAction, delay: DelayContext },
}

impl Default for BattleState {
    fn default() -> Self {
        Self::WaitingForMenu {
            reader: PokemonTextReader::message_box_only(),
            delay: DelayContext::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
enum AgentState {
    #[default]
    Idle,
    /// Policy returned None for an overworld action — waiting out a delay then re-polling.
    AwaitingOverworldAction { delay: DelayContext },
    OverworldMovement { destination: MetaTile, map: Map },
    ReadingTextBox { reader: PokemonTextReader },
    /// A map script or NPC scripted walk is running.  The player is frozen; the agent
    /// toggles A each tick to advance the script and any subsequent dialogue.
    RunningScript,
    /// The Pokémon nickname entry screen is active.
    /// `decided` is false while waiting for the policy; once true the name has been
    /// written to the naming buffer and START is toggled each tick until the screen exits.
    NamingPokemon { species: PokemonSpecies, decided: bool },

    Battle(BattleState),
}

impl AgentState {
    pub fn battle_state_mut(&mut self) -> Result<&mut BattleState, String> {
        if let AgentState::Battle(s) = self {
            Ok(s)
        } else {
            Err("Not in battle".to_string())
        }
    }
}

impl Default for PokemonAgent {
    fn default() -> Self { Self::new(Box::new(RandomPolicy)) }
}

impl PokemonAgent {
    pub fn new(policy: Box<dyn Policy>) -> Self {
        Self {
            state: AgentState::default(),
            event_buffer: VecDeque::new(),
            cycles: MachineCycles::default(),
            policy,
        }
    }

    pub fn policy_exhausted(&self) -> bool {
        self.policy.is_exhausted()
    }

    fn event(&mut self, event: AgentEvent) {
        println!("{:?}", event);
        self.event_buffer.push_back(event);
        while self.event_buffer.len() > 100 {
            self.event_buffer.pop_front();
        }
    }

    fn set_state(&mut self, state: AgentState) {
        if self.state != state {
            self.state = state;

            if self.state != AgentState::Idle {
                println!("{:?}", &self.state);
            }
        }
    }

    fn set_battle_state(&mut self, state: BattleState) {
        self.set_state(AgentState::Battle(state));
    }

    fn abort_overworld(&mut self, destination: MetaTile, reason: String) {
        self.event(AgentEvent::OverworldActionAborted { destination, reason });
        self.set_state(AgentState::Idle);
    }

    pub fn take_overworld_action(&mut self, action: OverworldAction) {
        self.event(AgentEvent::StartedOverworldAction { destination: action.tile.clone() });
        self.set_state(AgentState::OverworldMovement { destination: action.tile, map: action.map });
    }

    /// Checks if a battle has just started or finished
    fn assert_battle_state(&mut self, game_mode: GameMode) {
        if matches!(game_mode, GameMode::WildBattle | GameMode::TrainerBattle) {
            match self.state {
                AgentState::Battle(_) => {}
                AgentState::OverworldMovement { destination, .. } => {
                    // entering battle from the overworld
                    let d = destination;
                    self.abort_overworld(d, "battle started".into());
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
                _ => {
                    // entering battle from somewhere else, maybe a textbox
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
            }
        } else if let AgentState::Battle(battle_state) = &self.state {
            // Leaving battle.
            if let BattleState::WaitingForMenu { reader, .. } = battle_state {
                // dump remaining text
                self.event(AgentEvent::text_box_from_reader(reader));
            }

            self.event(AgentEvent::BattleEnded);
            self.set_state(AgentState::Idle);
        }
    }

    /// Checks if the naming screen has just opened or closed
    fn assert_naming_screen(&mut self, game_mode: GameMode, api: &mut PokemonApi) -> Result<(), String> {
        if game_mode == GameMode::NamingScreen {
            if !matches!(self.state, AgentState::NamingPokemon { .. }) {
                // the naming screen has just opened
                let species = api.naming_screen_species()?;
                api.release_all_buttons();
                self.set_state(AgentState::NamingPokemon { species, decided: false });
            }
        } else if matches!(self.state, AgentState::NamingPokemon { decided: false, .. }) {
            // The naming screen closed before the policy reached a decision (unexpected).
            self.set_state(AgentState::Idle);
        }
        // If decided=true the strict NamingScreen detection no longer fires (buf0 is no
        // longer 0x50 after the name is written), so game_mode is TextBox.  The
        // NamingPokemon match branch handles its own exit once the font unloads.

        Ok(())
    }

    /// If a map script triggers while navigating, abort and let RunningScript handle it.
    fn assert_script_state(&mut self, game_mode: GameMode) {
        if game_mode == GameMode::Script {
            if self.state != AgentState::RunningScript {
                if let AgentState::OverworldMovement { destination, .. } = self.state {
                    self.abort_overworld(destination, "map script started".into());
                }
                self.set_state(AgentState::RunningScript);
            }
        } else if self.state == AgentState::RunningScript {
            self.set_state(AgentState::Idle);
        }
    }

    fn assert_text_box_state(&mut self, game_mode: GameMode) {
        // wFontLoaded=1 while the naming screen is active too; NamingPokemon{decided:true}
        // handles its own exit, so don't interfere.
        if matches!(self.state, AgentState::NamingPokemon { decided: true, .. }) {
            return;
        }
        if game_mode == GameMode::TextBox {
            if !matches!(self.state, AgentState::ReadingTextBox { .. }) {
                // text box opened
                if let AgentState::OverworldMovement { destination, .. } = self.state {
                    self.abort_overworld(destination, "text box opened".into());
                }
                let reader = if matches!(self.state, AgentState::Battle(_)) {
                    PokemonTextReader::message_box_only()
                } else {
                    PokemonTextReader::default()
                };
                self.set_state(AgentState::ReadingTextBox { reader });
            }
        } else if let AgentState::ReadingTextBox { reader } = &self.state {
            // text box closed
            self.event(AgentEvent::text_box_from_reader(&reader));
            self.set_state(AgentState::Idle);
        }
    }

    pub fn update(&mut self, gb: &mut GameBoy, delta_cycles: MachineCycles) -> Result<(), String> {
        // ── Throttled decision-making ─────────────────────────────────────────────
        self.cycles += delta_cycles;
        if self.cycles < AGENT_RESOLUTION { return Ok(()); }

        let mut delta_cycles = MachineCycles::ZERO;
        while self.cycles >= AGENT_RESOLUTION {
            delta_cycles += AGENT_RESOLUTION;
            self.cycles -= AGENT_RESOLUTION;
        }

        let mut api = PokemonApi::new(gb);

        let game_mode = api.game_mode()
            .ok_or_else(|| "Not in game".to_string())?;

        self.assert_naming_screen(game_mode, &mut api)?;
        self.assert_script_state(game_mode);
        self.assert_battle_state(game_mode);
        self.assert_text_box_state(game_mode);

        let mut new_events: Vec<AgentEvent> = vec!();

        match self.state {
            AgentState::Idle => {
                api.release_all_buttons();
                match game_mode {
                    GameMode::TextBox => {
                        self.set_state(AgentState::ReadingTextBox { reader: PokemonTextReader::default() });
                    }
                    GameMode::Script => {
                        self.set_state(AgentState::RunningScript);
                    }
                    GameMode::NamingScreen => {
                        self.set_state(AgentState::NamingPokemon {
                            species: api.naming_screen_species()?,
                            decided: false,
                        });
                    }
                    GameMode::WildBattle | GameMode::TrainerBattle => {
                        self.set_state(AgentState::Battle(BattleState::default()));
                    }
                    GameMode::Overworld => {
                        self.set_state(AgentState::AwaitingOverworldAction { delay: DelayContext::long() });
                    }
                }
            }
            AgentState::AwaitingOverworldAction { ref mut delay } => {
                if delay.tick(delta_cycles) {
                    let game_state = api.game_state()?;
                    if let Some(action) = self.policy.pick_overworld_action(&game_state) {
                        self.take_overworld_action(action);
                    }
                }
            }
            AgentState::RunningScript => {
                api.toggle_button(JoypadButton::A);
            }
            AgentState::OverworldMovement { destination, map: expected_map } => {
                let game_state = api.game_state()?;
                if game_state.mode != GameMode::Overworld {
                    self.abort_overworld(destination,
                        format!("game state: {}", game_state.mode));
                } else if game_state.map.map != expected_map {
                    // Map changed — success for warps and connections (both take you off the map).
                    if matches!(destination, MetaTile::Warp(_) | MetaTile::Connection(_)) {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    } else {
                        self.abort_overworld(destination,
                            format!("on map {:?}, expected {:?}", game_state.map.map, expected_map));
                    }
                } else if game_state.map.player_tile() == destination && !matches!(destination, MetaTile::Warp(_)) {
                    new_events.push(AgentEvent::OverworldActionCompleted { destination });
                    self.set_state(AgentState::Idle);
                } else {
                    let action = game_state.map.actions().into_iter()
                        .find(|a| a.tile == destination);
                    match action {
                        None => self.abort_overworld(destination,
                                                     format!("no route to {:?}", destination)),
                        Some(a) => match a.route.first() {
                            // Pulse A via toggle so hJoyPressed fires every other tick —
                            // press_button (after release_all) would only fire once since A
                            // stays held and hJoyPressed goes dark on the next frame.
                            Some(&JoypadButton::A) => api.toggle_button(JoypadButton::A),
                            // Hold direction buttons for continuous walking.
                            Some(&btn) => {
                                api.release_all_buttons();
                                api.press_button(btn);
                            }
                            None => {
                                new_events.push(AgentEvent::OverworldActionCompleted { destination });
                                self.set_state(AgentState::Idle);
                            }
                        }
                    }
                }
            }
            AgentState::ReadingTextBox { ref mut reader } => {
                reader.update(&mut api);
            }
            AgentState::Battle(ref mut battle_state) => {
                match battle_state {
                    BattleState::WaitingForMenu { reader, delay } => {
                        if let Some(menu_state) = api.menu_state() {
                            match menu_state.battle_menu_state() {
                                Some(BattleMenuState::Fight) => {
                                    new_events.push(AgentEvent::text_box_from_reader(reader));

                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::AwaitingPolicy { delay: DelayContext::default() });
                                }
                                Some(_) => {
                                    // battle menu is showing, do not read the text
                                    api.toggle_button(JoypadButton::A);
                                },
                                None => {
                                    // something other than the battle menu is showing — wait for
                                    // the text box to render before reading it
                                    if delay.tick(delta_cycles) {
                                        reader.update(&mut api);
                                    }
                                }
                            }
                        } else {
                            // no menu is showing, click mashing the A button
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    BattleState::AwaitingPolicy { delay } => {
                        if delay.tick(delta_cycles) {
                            let game_state = api.game_state()?;
                            if let Some(action) = self.policy.pick_battle_action(&game_state) {
                                new_events.push(AgentEvent::BattleActionStarted { action });
                                self.set_battle_state(BattleState::Navigating { action, delay: DelayContext::default() });
                            }
                        }
                    }

                    BattleState::Navigating { action, delay } => {
                        if delay.tick(delta_cycles) {
                            if let Some(menu_state) = api.menu_state().map(|s| s.battle_menu_state()).flatten() {
                                let menu_target = BattleMenuState::from_action(*action);

                                if menu_state == menu_target {
                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::default());
                                } else {
                                    let resolved_target = if let Some(target_parent) = menu_target.parent() {
                                        if menu_state.parent() == Some(target_parent) {
                                            menu_target
                                        } else {
                                            target_parent
                                        }
                                    } else {
                                        menu_target
                                    };

                                    let target_location = resolved_target.location();
                                    let current_location = menu_state.location();


                                    let btn = if target_location == current_location {
                                        JoypadButton::A
                                    } else if target_location.x > current_location.x {
                                        JoypadButton::Right
                                    } else if target_location.x < current_location.x {
                                        JoypadButton::Left
                                    } else if target_location.y > current_location.y {
                                        JoypadButton::Down
                                    } else {
                                        JoypadButton::Up
                                    };

                                    api.toggle_button(btn);
                                }
                            }
                        }
                    }
                }
            }
            AgentState::NamingPokemon { species, decided } => {
                if decided {
                    // Buffer already written; keep pulsing START until DisplayNamingScreen
                    // exits (wFontLoaded → 0, so game_mode leaves TextBox/NamingScreen).
                    api.toggle_button(JoypadButton::Start);
                    if game_mode != GameMode::TextBox && game_mode != GameMode::NamingScreen {
                        api.release_all_buttons();
                        self.set_state(AgentState::Idle);
                    }
                } else if let Some(decision) = self.policy.pick_nickname(species) {
                    // Write the nickname directly into the naming screen's string buffer,
                    // bypassing character-grid navigation.  The screen copies this buffer
                    // when START is pressed.  An empty/None nickname causes AskName to
                    // fall back to the default species name.
                    api.write_naming_screen_buffer(decision.as_deref())?;
                    self.set_state(AgentState::NamingPokemon { species, decided: true });
                } else {
                    api.release_all_buttons();
                }
            }
        }

        for x in new_events.into_iter() {
            self.event(x);
        }

        Ok(())
    }

}
