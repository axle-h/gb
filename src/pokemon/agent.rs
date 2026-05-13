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
use crate::pokemon::policy::{Policy, RandomPolicy};
use crate::pokemon::text::PokemonTextReader;

const RESOLUTION: MachineCycles = MachineCycles::from_hz(60);

/// Time to wait between agent actions e.g. between the cursor landing on a menu item and pressing A
const ACTION_DELAY: MachineCycles = MachineCycles::from_duration(Duration::from_millis(50));

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

#[derive(Debug, Clone, Default)]
enum BattleState {
    /// Waiting for the battle menu (TextBoxID 0x0B/0x1B) to appear.
    #[default]
    WaitingForMenu,
    /// Battle menu is up but policy hasn't returned an action yet.
    AwaitingPolicy,

    /// Queue-based navigation: each `u8` in `targets` is a `wCurrentMenuItem`
    /// value to navigate to; the agent moves the cursor there then presses A,
    /// then waits `delay` frames before popping the next target.
    Navigating {
        targets:       VecDeque<u8>,
        /// Accumulated time the cursor has been sitting on the target slot.
        at_target_for: MachineCycles,
        /// Remaining cooldown after the last A press.
        delay_after_a: MachineCycles,
        /// Remaining cooldown after a DPAD press (game uses edge detection).
        dpad_cooldown: MachineCycles,
    },

    /// Waiting for the next menu prompt or battle end.
    WaitingForResult,
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
                let menu_state = api.menu_state().unwrap_or_default();
                if menu_state.is_battle_menu() {
                    api.release_button(JoypadButton::A);
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
                    self.set_battle_state(BattleState::Navigating {
                        targets:       action.navigation_targets().into(),
                        at_target_for: MachineCycles::ZERO,
                        delay_after_a: MachineCycles::ZERO,
                        dpad_cooldown: MachineCycles::ZERO,
                    });
                }
            }

            BattleState::Navigating { mut targets, at_target_for, delay_after_a, dpad_cooldown } => {
                api.release_all_buttons();
                let menu_state = api.menu_state().unwrap_or_default();

                // All targets confirmed — hand off to BattleWaitingForResult so A-pulse
                // can advance the battle text (the pulse doesn't run in this state).
                if targets.is_empty() {
                    self.set_battle_state(BattleState::WaitingForResult);
                    return Ok(());
                }

                if !menu_state.is_battle_menu() {
                    // Menu closed (animation playing) — keep waiting.
                    self.set_battle_state(BattleState::Navigating { targets, at_target_for, delay_after_a, dpad_cooldown });
                    return Ok(());
                }

                // Drain the post-A cooldown before doing anything else.
                if delay_after_a > MachineCycles::ZERO {
                    self.set_battle_state(BattleState::Navigating {
                        targets,
                        at_target_for,
                        delay_after_a: delay_after_a - delta,
                        dpad_cooldown: MachineCycles::ZERO,
                    });
                    return Ok(());
                }

                let target = match targets.front().copied() {
                    Some(t) => t,
                    None => {
                        self.set_battle_state(BattleState::WaitingForResult);
                        return Ok(());
                    }
                };

                if menu_state.current_menu == target {
                    // Cursor is on the correct slot — wait AT_TARGET_MIN before confirming.
                    let new_at_target = at_target_for + delta;
                    if new_at_target >= ACTION_DELAY {
                        api.press_button(JoypadButton::A);
                        targets.pop_front();
                        self.set_battle_state(BattleState::Navigating {
                            targets,
                            at_target_for: MachineCycles::ZERO,
                            delay_after_a: ACTION_DELAY,
                            dpad_cooldown: MachineCycles::ZERO,
                        });
                    } else {
                        self.set_battle_state(BattleState::Navigating {
                            targets,
                            at_target_for: new_at_target,
                            delay_after_a: MachineCycles::ZERO,
                            dpad_cooldown: MachineCycles::ZERO,
                        });
                    }
                } else if dpad_cooldown > MachineCycles::ZERO {
                    // Cooldown after last DPAD press — DPAD already blanket-released above,
                    // so the game will see an edge (0→1) when we press again after cooldown.
                    self.set_battle_state(BattleState::Navigating {
                        targets,
                        at_target_for: MachineCycles::ZERO,
                        delay_after_a: MachineCycles::ZERO,
                        dpad_cooldown: dpad_cooldown - delta,
                    });
                } else {
                    // Navigate towards target using the 2D grid layout.
                    let cur_row = menu_state.current_menu / 2;
                    let cur_col = menu_state.current_menu % 2;
                    let tgt_row = target / 2;
                    let tgt_col = target % 2;

                    let btn = if tgt_col > cur_col      { JoypadButton::Right }
                              else if tgt_col < cur_col  { JoypadButton::Left  }
                              else if tgt_row > cur_row  { JoypadButton::Down  }
                              else                        { JoypadButton::Up    };

                    api.press_button(btn);
                    self.set_battle_state(BattleState::Navigating {
                        targets,
                        at_target_for: MachineCycles::ZERO,
                        delay_after_a: MachineCycles::ZERO,
                        dpad_cooldown: ACTION_DELAY,
                    });
                }
            }

            BattleState::WaitingForResult => {
                let menu_state = api.menu_state().unwrap_or_default();
                if menu_state.is_battle_menu() {
                    api.release_button(JoypadButton::A);
                    self.set_battle_state(BattleState::WaitingForMenu);
                } else {
                    api.toggle_button(JoypadButton::B);
                }
                // else: A-pulse already handles advance in the unthrottled section above.
            }

            _ => {}
        }
        Ok(())
    }
}
