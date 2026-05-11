use std::collections::{VecDeque};
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::{BattleAction, BattleStateReader};
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use crate::pokemon::encoding::{GameMode, MetaTile};
use crate::pokemon::map::Map;
use crate::pokemon::policy::{Policy, RandomPolicy};
use crate::pokemon::symbols::{DmgPointerRead, pokered_symbols};
use crate::pokemon::text::PokemonTextReader;

const RESOLUTION: MachineCycles = MachineCycles::from_hz(60);

/// `wTextBoxID` values indicating a 2×2 menu is on screen waiting for input.
/// Both the main battle menu (FIGHT/PKMN/ITEM/RUN) and every sub-menu (moves, bag,
/// party) share this same ID, so we track which step we're on via the nav queue.
const BATTLE_MENU_TEXT_BOX_IDS: [u8; 2] = [0x0B, 0x1B];

/// Frames to wait after pressing A before attempting the next navigation step.
/// This lets the game finish its transition animation.
const POST_A_DELAY: u8 = 30;

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
enum AgentState {
    #[default]
    Idle,
    OverworldMovement { destination: MetaTile, map: Map },
    ReadingTextBox { reader: PokemonTextReader },

    /// Waiting for the battle menu (TextBoxID 0x0B/0x1B) to appear.
    BattleWaitingForMenu,

    /// Queue-based navigation: each `u8` in `targets` is a `wCurrentMenuItem`
    /// value to navigate to; the agent moves the cursor there then presses A,
    /// then waits `delay` frames before popping the next target.
    BattleNavigating {
        targets:       VecDeque<u8>,
        at_target_for: u8,   // frames cursor has been on the right slot
        delay_after_a: u8,   // frames remaining after an A press
    },

    /// Waiting for the next menu prompt or battle end.
    BattleWaitingForResult,
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
        self.cycles += delta_cycles;
        if self.cycles < RESOLUTION { return Ok(()); }
        while self.cycles >= RESOLUTION { self.cycles -= RESOLUTION; }

        let game_mode = PokemonApi::new(gb).game_mode()
            .ok_or_else(|| "Not in game".to_string())?;
        let in_battle = matches!(game_mode, GameMode::WildBattle | GameMode::TrainerBattle);

        if in_battle {
            // Switch any non-battle state to BattleWaitingForMenu.
            match self.state {
                AgentState::BattleWaitingForMenu
                | AgentState::BattleNavigating { .. }
                | AgentState::BattleWaitingForResult => {}
                AgentState::OverworldMovement { destination, .. } => {
                    let d = destination;
                    self.abort_overworld(d, "battle started".into());
                    self.event(AgentEvent::BattleStarted);
                    self.state = AgentState::BattleWaitingForMenu;
                }
                _ => {
                    self.event(AgentEvent::BattleStarted);
                    self.state = AgentState::BattleWaitingForMenu;
                }
            }
            return self.update_battle(gb);
        }

        // Leaving battle.
        match self.state {
            AgentState::BattleWaitingForMenu
            | AgentState::BattleNavigating { .. }
            | AgentState::BattleWaitingForResult => {
                self.event(AgentEvent::BattleEnded);
                self.state = AgentState::Idle;
            }
            _ => {}
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
                        if let Some(action) = self.policy.pick_action(&game_state) {
                            self.take_overworld_action(action);
                        }
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
                    if matches!(destination, MetaTile::Warp(_)) {
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

    fn update_battle(&mut self, gb: &mut GameBoy) -> Result<(), String> {
        let text_box_id  = gb.core().mmu().read_pointer(&pokered_symbols::wTextBoxID);
        let current_menu = gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem);
        let menu_ready   = BATTLE_MENU_TEXT_BOX_IDS.contains(&text_box_id);

        // Release all buttons by default; we'll press specific ones below.
        let joypad = gb.core_mut().mmu_mut().joypad_mut();
        joypad.release_button(JoypadButton::A);
        joypad.release_button(JoypadButton::Up);
        joypad.release_button(JoypadButton::Down);
        joypad.release_button(JoypadButton::Left);
        joypad.release_button(JoypadButton::Right);

        match self.state.clone() {

            AgentState::BattleWaitingForMenu => {
                if menu_ready {
                    let battle = gb.core().mmu().read_battle_state()
                        .ok_or("battle state unavailable")?;
                    let action = self.policy.pick_battle_action(&battle);
                    self.event(AgentEvent::BattleActionStarted { action });
                    self.state = AgentState::BattleNavigating {
                        targets:       action.navigation_targets().into(),
                        at_target_for: 0,
                        delay_after_a: 0,
                    };
                }
            }

            AgentState::BattleNavigating { mut targets, mut at_target_for, mut delay_after_a } => {
                if !menu_ready {
                    // Menu closed (still animating) — update state and wait.
                    self.state = AgentState::BattleNavigating { targets, at_target_for, delay_after_a };
                    return Ok(());
                }

                // Cooling down after an A press.
                if delay_after_a > 0 {
                    self.state = AgentState::BattleNavigating {
                        targets,
                        at_target_for,
                        delay_after_a: delay_after_a - 1,
                    };
                    return Ok(());
                }

                let target = match targets.front().copied() {
                    Some(t) => t,
                    None => {
                        self.state = AgentState::BattleWaitingForResult;
                        return Ok(());
                    }
                };

                if current_menu == target {
                    // Cursor is on the right slot.
                    if at_target_for >= 1 {
                        // Confirmed: press A.
                        gb.core_mut().mmu_mut().joypad_mut().press_button(JoypadButton::A);
                        targets.pop_front();
                        self.state = AgentState::BattleNavigating {
                            targets,
                            at_target_for: 0,
                            delay_after_a: POST_A_DELAY,
                        };
                    } else {
                        self.state = AgentState::BattleNavigating {
                            targets,
                            at_target_for: at_target_for + 1,
                            delay_after_a: 0,
                        };
                    }
                } else {
                    // Navigate towards target using the 2D grid layout.
                    let cur_row = current_menu / 2;
                    let cur_col = current_menu % 2;
                    let tgt_row = target / 2;
                    let tgt_col = target % 2;

                    let btn = if tgt_col > cur_col      { JoypadButton::Right }
                              else if tgt_col < cur_col  { JoypadButton::Left  }
                              else if tgt_row > cur_row  { JoypadButton::Down  }
                              else                        { JoypadButton::Up    };

                    gb.core_mut().mmu_mut().joypad_mut().press_button(btn);
                    self.state = AgentState::BattleNavigating {
                        targets,
                        at_target_for: 0,
                        delay_after_a: 0,
                    };
                }
            }

            AgentState::BattleWaitingForResult => {
                if menu_ready {
                    self.state = AgentState::BattleWaitingForMenu;
                }
            }

            _ => {}
        }
        Ok(())
    }
}
