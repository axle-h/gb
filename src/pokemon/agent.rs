use std::collections::{VecDeque};
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
use crate::pokemon::text::PokemonTextReader;

// too long and player veers off course on the overworld, too short and the game doesn't get chance to update values between turns
const RESOLUTION: MachineCycles = MachineCycles::from_duration(Duration::from_millis(20));

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
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, Default)]
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

    fn event(&mut self, event: AgentEvent) {
        println!("{:?}", event);
        self.event_buffer.push_back(event);
        while self.event_buffer.len() > 100 {
            self.event_buffer.pop_front();
        }
    }

    fn abort_overworld(&mut self, destination: MetaTile, reason: String) {
        self.event(AgentEvent::OverworldActionAborted { destination, reason });
        self.state = AgentState::Idle;
    }

    pub fn take_overworld_action(&mut self, action: OverworldAction) {
        self.event(AgentEvent::StartedOverworldAction { destination: action.tile.clone() });
        self.state = AgentState::OverworldMovement { destination: action.tile, map: action.map };
    }

    pub fn update(&mut self, gb: &mut GameBoy, delta_cycles: MachineCycles) -> Result<(), String> {
        // ── Throttled decision-making ─────────────────────────────────────────────
        self.cycles += delta_cycles;
        if self.cycles < RESOLUTION { return Ok(()); }

        let mut buffered_delta = MachineCycles::ZERO;
        while self.cycles >= RESOLUTION {
            buffered_delta += RESOLUTION;
            self.cycles -= RESOLUTION;
        }

        let api = PokemonApi::new(gb);

        let game_mode = api.game_mode()
            .ok_or_else(|| "Not in game".to_string())?;

        // If a map script triggers while navigating, abort and let RunningScript handle it.
        if game_mode == GameMode::Script {
            if let AgentState::OverworldMovement { destination, .. } = self.state {
                self.abort_overworld(destination, "map script started".into());
            }
        }

        if  matches!(game_mode, GameMode::WildBattle | GameMode::TrainerBattle) {
            // entering battle
            match self.state {
                AgentState::Battle(_) => {}
                AgentState::OverworldMovement { destination, .. } => {
                    let d = destination;
                    self.abort_overworld(d, "battle started".into());
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
                _ => {
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
            }
            return self.update_battle(gb, buffered_delta);
        }

        // Leaving battle.
        if let AgentState::Battle(battle_state) = &self.state {
            if let BattleState::WaitingForMenu { reader, .. } = battle_state {
                // dump remaining text
                println!("Battle text: {}", reader);
            }

            self.event(AgentEvent::BattleEnded);
            self.state = AgentState::Idle;
        }
        self.update_overworld(gb, game_mode, buffered_delta)
    }

    // ── Overworld ──────────────────────────────────────────────────────────────

    fn update_overworld(&mut self, gb: &mut GameBoy, game_mode: GameMode, delta_cycles: MachineCycles) -> Result<(), String> {
        let mut api = PokemonApi::new(gb);
        match self.state {
            AgentState::Idle => {
                match game_mode {
                    GameMode::TextBox => {
                        self.state = AgentState::ReadingTextBox { reader: PokemonTextReader::default() };
                    }
                    GameMode::Script => {
                        self.state = AgentState::RunningScript;
                    }
                    _ => {
                        self.state = AgentState::AwaitingOverworldAction { delay: DelayContext::long() }
                    }
                }
            }
            AgentState::AwaitingOverworldAction { ref mut delay } => {
                // Re-check mode before acting: game_mode may have transitioned to TextBox or Script
                // while the delay was counting down (e.g. a map script triggered mid-wait).
                match game_mode {
                    GameMode::TextBox => {
                        self.state = AgentState::ReadingTextBox { reader: PokemonTextReader::default() };
                        return Ok(());
                    }
                    GameMode::Script => {
                        self.state = AgentState::RunningScript;
                        return Ok(());
                    }
                    _ => {}
                }
                if !delay.tick(delta_cycles) { return Ok(()); }
                let game_state = api.game_state()?;
                if let Some(action) = self.policy.pick_overworld_action(&game_state) {
                    self.take_overworld_action(action);
                }
            }
            AgentState::RunningScript => {
                match game_mode {
                    GameMode::Script => api.toggle_button(JoypadButton::A),
                    GameMode::TextBox => {
                        api.release_all_buttons();
                        self.state = AgentState::ReadingTextBox { reader: PokemonTextReader::default() };
                    }
                    _ => {
                        api.release_all_buttons();
                        self.state = AgentState::Idle;
                    }
                }
            }
            AgentState::OverworldMovement { destination, map: expected_map } => {
                api.release_all_buttons();
                let game_state = api.game_state()?;
                if game_state.mode != GameMode::Overworld {
                    self.abort_overworld(destination,
                        format!("game state: {}", game_state.mode));
                    return Ok(());
                }
                if game_state.map.map != expected_map {
                    // Map changed — success for warps and connections (both take you off the map).
                    if matches!(destination, MetaTile::Warp(_) | MetaTile::Connection(_)) {
                        self.event(AgentEvent::OverworldActionCompleted { destination });
                        self.state = AgentState::Idle;
                    } else {
                        self.abort_overworld(destination,
                            format!("on map {:?}, expected {:?}", game_state.map.map, expected_map));
                    }
                    return Ok(());
                }
                if game_state.map.player_tile() == destination
                    && !matches!(destination, MetaTile::Warp(_))
                {
                    self.event(AgentEvent::OverworldActionCompleted { destination });
                    self.state = AgentState::Idle;
                    return Ok(());
                }
                let action = game_state.map.actions().into_iter()
                    .find(|a| a.tile == destination);
                match action {
                    None => self.abort_overworld(destination,
                                format!("no route to {:?}", destination)),
                    Some(a) => {
                        if let Some(&btn) = a.route.first() {
                            api.press_button(btn);
                        } else {
                            self.event(AgentEvent::OverworldActionCompleted { destination });
                            self.state = AgentState::Idle;
                        }
                    }
                }
            }
            AgentState::ReadingTextBox { ref reader } if game_mode != GameMode::TextBox => {
                println!("TextBox: {}", reader);
                api.release_all_buttons();
                self.state = AgentState::Idle;
            }
            AgentState::ReadingTextBox { ref mut reader } => {
                reader.update(&mut api);
            }
            _ => {}
        }
        Ok(())
    }

    // ── Battle ─────────────────────────────────────────────────────────────────

    fn set_battle_state(&mut self, state: BattleState) {
        // TODO emit an event
        println!("Battle state: {:?}", state);
        self.state = AgentState::Battle(state);
    }

    fn update_battle(&mut self, gb: &mut GameBoy, delta: MachineCycles) -> Result<(), String> {
        let battle_state = self.state.battle_state_mut()?;

        let mut api = PokemonApi::new(gb);

        match battle_state {
            BattleState::WaitingForMenu { reader, delay } => {
                if let Some(menu_state) = api.menu_state() {
                    match menu_state.battle_menu_state() {
                        Some(BattleMenuState::Fight) => {
                            println!("Battle text: {}", reader);

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
                            if delay.tick(delta) {
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
                if !delay.tick(delta) { return Ok(()); }
                let game_state = api.game_state()?;
                if let Some(action) = self.policy.pick_battle_action(&game_state) {
                    self.event(AgentEvent::BattleActionStarted { action });
                    self.set_battle_state(BattleState::Navigating { action, delay: DelayContext::default() });
                }
            }

            BattleState::Navigating { action, delay } => {
                if !delay.tick(delta) { return Ok(()); }
                let menu_state = api.menu_state().map(|s| s.battle_menu_state()).flatten();
                if menu_state.is_none() {
                    // wait for menu state
                    return Ok(());
                }
                let menu_state = menu_state.unwrap();

                let menu_target = BattleMenuState::from_action(*action);

                if menu_state == menu_target {
                    api.release_all_buttons();
                    self.set_battle_state(BattleState::default());
                    return Ok(());
                }

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

            _ => {}
        }
        Ok(())
    }
}
