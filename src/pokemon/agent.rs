use std::collections::{VecDeque};
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::{PokemonApi, PokemonApiTrait};
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

#[derive(Debug, Clone, Copy, Default)]
enum BattleState {
    /// Waiting for the battle menu (TextBoxID 0x0B/0x1B) to appear.
    #[default]
    WaitingForMenu,
    /// Battle menu is up but policy hasn't returned an action yet.
    AwaitingPolicy,

    /// Navigating the menus
    Navigating { action: BattleAction },
}

#[derive(Debug, Clone, Default)]
enum AgentState {
    #[default]
    Idle,
    /// Policy returned None for an overworld action — re-polling each frame.
    AwaitingOverworldAction,
    OverworldMovement { destination: MetaTile, map: Map },
    ReadingTextBox { reader: PokemonTextReader },

    Battle(BattleState),
}

impl AgentState {
    pub fn battle_state(&self) -> Result<BattleState, String> {
        if let AgentState::Battle(s) = self {
            Ok(s.clone()) // TODO make the state copyable
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
        while self.cycles >= RESOLUTION { self.cycles -= RESOLUTION; }

        let api = PokemonApi::new(gb);

        let game_mode = api.game_mode()
            .ok_or_else(|| "Not in game".to_string())?;

        if  matches!(game_mode, GameMode::WildBattle | GameMode::TrainerBattle) {
            // entering battle
            match self.state {
                AgentState::Battle(_) => {}
                AgentState::OverworldMovement { destination, .. } => {
                    let d = destination;
                    self.abort_overworld(d, "battle started".into());
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::WaitingForMenu);
                }
                _ => {
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::WaitingForMenu);
                }
            }
            return self.update_battle(gb, delta_cycles);
        }

        // Leaving battle.
        if matches!(self.state, AgentState::Battle(_)) {
            self.event(AgentEvent::BattleEnded);
            self.state = AgentState::Idle;
        }
        self.update_overworld(gb, game_mode)
    }

    // ── Overworld ──────────────────────────────────────────────────────────────

    fn update_overworld(&mut self, gb: &mut GameBoy, game_mode: GameMode) -> Result<(), String> {
        let mut api = PokemonApi::new(gb);
        match self.state {
            AgentState::Idle => {
                match game_mode {
                    GameMode::TextBox => {
                        self.state = AgentState::ReadingTextBox { reader: PokemonTextReader::default() };
                    }
                    _ => {
                        let game_state = api.game_state()?;
                        match self.policy.pick_overworld_action(&game_state) {
                            Some(action) => self.take_overworld_action(action),
                            None         => self.state = AgentState::AwaitingOverworldAction,
                        }
                    }
                }
            }
            AgentState::AwaitingOverworldAction => {
                // Policy returned None last time — keep polling each frame.
                let game_state = api.game_state()?;
                if let Some(action) = self.policy.pick_overworld_action(&game_state) {
                    self.take_overworld_action(action);
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
        let battle_state = self.state.battle_state()?;

        let mut api = PokemonApi::new(gb);

        match battle_state {
            BattleState::WaitingForMenu => {
                if api.menu_state().map(|s| s.battle_menu_state()).flatten() == Some(BattleMenuState::Fight) {
                    api.release_all_buttons();
                    self.set_battle_state(BattleState::AwaitingPolicy);
                } else {
                    // pulse the A button until the menu is up
                    api.toggle_button(JoypadButton::A);
                }
            }

            BattleState::AwaitingPolicy => {
                let game_state = api.game_state()?;
                if let Some(action) = self.policy.pick_battle_action(&game_state) {
                    self.event(AgentEvent::BattleActionStarted { action });
                    self.set_battle_state(BattleState::Navigating { action });
                }
            }

            BattleState::Navigating { action } => {
                let menu_state = api.menu_state().map(|s| s.battle_menu_state()).flatten();
                if menu_state.is_none() {
                    // wait for menu state
                    return Ok(());
                }
                let menu_state = menu_state.unwrap();

                println!("{:?}, {:?}", api.menu_state().unwrap(), menu_state);

                let menu_target = BattleMenuState::from_action(action);

                if menu_state == menu_target {
                    api.release_all_buttons();
                    self.set_battle_state(BattleState::WaitingForMenu);
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
