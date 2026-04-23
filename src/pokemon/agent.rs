use std::collections::VecDeque;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use crate::pokemon::encoding::{GameMode, MetaTile};
use crate::pokemon::map::Map;
use crate::pokemon::text::PokemonTextReader;

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
    ReadingTextBox { reader: PokemonTextReader },
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
                        self.state = State::ReadingTextBox { reader: PokemonTextReader::default() };
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
            State::ReadingTextBox { ref reader } if game_mode != GameMode::TextBox => {
                // Check if text box is closed
                // print the collected text
                println!("TextBox: {}", reader);
                api.release_all_buttons();
                self.state = State::Idle;
            }
            State::ReadingTextBox { ref mut reader } => {
                reader.update(&mut api);
            }
        }
        Ok(())
    }
}