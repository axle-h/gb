use std::collections::VecDeque;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::PokemonApi;
use crate::pokemon::encoding::{GameMode, MetaTile};
use crate::pokemon::map::Map;

const RESOLUTION: MachineCycles = MachineCycles::from_hz(60);

#[derive(Debug, Default)]
pub struct PokemonAgent {
    state: State,
    event_buffer: VecDeque<AgentEvent>,
    cycles: MachineCycles,
}

#[derive(Debug)]
pub enum AgentEvent {
    StartedOverworldAction { destination: MetaTile },
    OverworldActionAborted { destination: MetaTile, reason: String },
    OverworldActionCompleted { destination: MetaTile },
}

#[derive(Debug, Clone, Default)]
enum State {
    #[default]
    Idle,
    OverworldMovement { destination: MetaTile, map: Map },
    ReadingTextBox { buffer: String },
}

impl PokemonAgent {
    fn event(&mut self, event: AgentEvent) {
        println!("{:?}", event);
        self.event_buffer.push_back(event);
        while self.event_buffer.len() > 100 {
            self.event_buffer.pop_front();
        }
    }

    fn overworld_action_aborted(&mut self, destination: MetaTile, reason: String) {
        self.event(AgentEvent::OverworldActionAborted { destination, reason });
        self.state = State::Idle;
    }

    pub fn take_overworld_action(&mut self, action: OverworldAction) {
        self.event(AgentEvent::StartedOverworldAction { destination: action.tile.clone() });
        self.state = State::OverworldMovement { destination: action.tile, map: action.map };
    }

    pub fn update(&mut self, gb: &mut GameBoy, delta_cycles: MachineCycles) -> Result<(), String> {
        self.cycles += delta_cycles;
        if self.cycles < RESOLUTION {
            return Ok(());
        }
        while self.cycles >= RESOLUTION {
            // skip unused cycles
            self.cycles -= RESOLUTION;
        }

        let mut api = PokemonApi::new(gb);
        let game_mode = api.game_mode();
        match self.state {
            State::Idle => {
                match game_mode {
                    GameMode::TextBox => {
                        self.state = State::ReadingTextBox { buffer: "".to_string() };
                    }
                    _ => {
                        // TODO wait for llm action
                    }
                }
            },
            State::OverworldMovement { destination, map: expected_map } => {
                api.release_all_buttons();
                let game_state = api.game_state()?;
                if game_state.mode != GameMode::Overworld {
                    self.overworld_action_aborted(
                        destination,
                        format!("Game is currently in state: {}", game_state.mode)
                    );
                    return Ok(());
                }

                if game_state.map != expected_map {
                    self.overworld_action_aborted(
                        destination,
                        format!("Player is on map {:?}, expected {:?}", game_state.map, expected_map)
                    );
                    return Ok(());
                }

                if game_state.player_tile == destination {
                    self.event(AgentEvent::OverworldActionCompleted { destination });
                    self.state = State::Idle;
                    return Ok(());
                }

                let action = game_state.actions
                    .into_iter()
                    .find(|action| action.tile == destination);
                if action.is_none() {
                    self.overworld_action_aborted(
                        destination,
                        format!("No available route to {:?}", destination)
                    );
                    return Ok(());
                }

                let action = action.unwrap();
                let button = action.route.first();
                if button.is_none() {
                    self.event(AgentEvent::OverworldActionCompleted { destination });
                    self.state = State::Idle;
                } else {
                    api.press_button(*button.unwrap());
                }
            }
            State::ReadingTextBox { ref buffer } if game_mode != GameMode::TextBox => {
                // Check if text box is closed
                // print the collected text
                println!("TextBox: {:?}", buffer);
                api.release_all_buttons();
                self.state = State::Idle;
            }
            State::ReadingTextBox { ref buffer } => {
                // mash the A button to advance the text
                let joypad_state = api.read_joypad_state();
                if joypad_state.a {
                    api.release_all_buttons();
                } else {
                    api.press_button(JoypadButton::A);
                }

                let mut next_buffer = buffer.clone();
                let buffer_len = next_buffer.len();
                if let Some(on_screen_text) = api.on_screen_text() {
                    let on_screen_text_len = on_screen_text.len();
                    if on_screen_text_len > 0 && buffer_len == 0 {
                        next_buffer = on_screen_text;
                    } else if on_screen_text_len > 0 {
                        // Try to find overlap at the end of buffer
                        let buffer_chars: Vec<char> = next_buffer.chars().collect();
                        let screen_chars: Vec<char> = on_screen_text.chars().collect();

                        let mut best_overlap = 0;
                        for overlap_len in (1..=buffer_chars.len().min(screen_chars.len())).rev() {
                            if buffer_chars[buffer_chars.len() - overlap_len..] == screen_chars[..overlap_len] {
                                best_overlap = overlap_len;
                                break;
                            }
                        }

                        // Skip the overlapping characters and append the rest
                        next_buffer.push_str(&on_screen_text.chars().skip(best_overlap).collect::<String>());

                    }
                }

                self.state = State::ReadingTextBox { buffer: next_buffer };
            }




        }
        Ok(())
    }
}