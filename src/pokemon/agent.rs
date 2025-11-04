use std::collections::VecDeque;
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::pokemon::actions::{OverworldAction};
use crate::pokemon::{GameMode, MetaTile, PokemonApi};
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

#[derive(Debug, Clone, Copy, Default)]
enum State {
    #[default]
    Idle,
    OverworldMovement { destination: MetaTile, map: Map },
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
        match self.state {
            State::Idle => {},
            State::OverworldMovement { destination, map: expected_map } => {
                api.release_all_buttons();
                let game_state = api.game_mode();
                if game_state != GameMode::Overworld {
                    self.overworld_action_aborted(
                        destination,
                        format!("Game is currently in state: {}", game_state)
                    );
                    return Ok(());
                }

                let map_state = api.map_state()?;
                if map_state.map != expected_map {
                    self.overworld_action_aborted(
                        destination,
                        format!("Player is on map {:?}, expected {:?}", map_state.map, expected_map)
                    );
                    return Ok(());
                }

                if map_state.player_tile == destination {
                    self.event(AgentEvent::OverworldActionCompleted { destination });
                    self.state = State::Idle;
                    return Ok(());
                }

                let action = map_state.actions
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
        }
        Ok(())
    }
}