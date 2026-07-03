use std::fmt::{Display, Formatter};
use crate::joypad::JoypadButton;
use crate::pokemon::{PokemonApi, PokemonApiTrait};

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct PokemonTextReader {
    buffer: String,
    had_text: bool,
    page_cleared: bool,
    message_box_only: bool,
}

impl Display for PokemonTextReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.buffer.as_str())
    }
}

impl PokemonTextReader {
    pub fn message_box_only() -> Self {
        Self {
            message_box_only: true,
            ..Self::default()
        }
    }


    pub fn update<A: PokemonApiTrait>(&mut self, api: &mut A) {
        // mash the A button to advance the text
        api.toggle_button(JoypadButton::A);

        let buffer_len = self.buffer.len();
        if let Some(on_screen_text) = api.on_screen_text(self.message_box_only) {
            let on_screen_text_len = on_screen_text.len();
            if on_screen_text_len == 0 && self.had_text {
                self.page_cleared = true;
                self.had_text = false;
            } else if on_screen_text_len > 0 && buffer_len == 0 {
                self.buffer = on_screen_text;
                self.had_text = true;
                self.page_cleared = false;
            } else if on_screen_text_len > 0 {
                if self.page_cleared {
                    self.buffer.push(' ');
                    self.page_cleared = false;
                }
                // Try to find overlap at the end of buffer
                let buffer_chars: Vec<char> = self.buffer.chars().collect();
                let screen_chars: Vec<char> = on_screen_text.chars().collect();

                let mut best_overlap = 0;
                for overlap_len in (1..=buffer_chars.len().min(screen_chars.len())).rev() {
                    if buffer_chars[buffer_chars.len() - overlap_len..] == screen_chars[..overlap_len] {
                        best_overlap = overlap_len;
                        break;
                    }
                }

                // Skip the overlapping characters and append the rest
                self.buffer.push_str(&on_screen_text.chars().skip(best_overlap).collect::<String>());
                self.had_text = true;

            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::joypad::JoypadButtonState;
    use crate::pokemon::encoding::GameMode;
    use crate::pokemon::GameState;
    use crate::pokemon::menu::MenuState;
    use crate::pokemon::options::GameOptions;
    use super::*;

    #[derive(Default)]
    struct StubPokemonApi {
        joypad: JoypadButtonState,
        game_state: GameState,
        on_screen_text: Option<String>,
    }

    impl PokemonApiTrait for StubPokemonApi {
        fn release_all_buttons(&mut self) {
            self.joypad = JoypadButtonState::default();
        }

        fn press_button(&mut self, button: JoypadButton) {
            self.joypad.update_button(button, true)
        }

        fn release_button(&mut self, button: JoypadButton) {
            self.joypad.update_button(button, false)
        }

        fn toggle_button(&mut self, button: JoypadButton) {
            self.joypad.update_button(button, !self.joypad.is_button_pressed(button))
        }

        fn read_joypad_state(&self) -> JoypadButtonState {
            self.joypad
        }

        fn game_mode(&self) -> Option<GameMode> {
            Some(self.game_state.mode)
        }

        fn trainer_battle_pending(&self) -> bool {
            false
        }

        fn raw_player_coords(&self) -> crate::geometry::Point8 {
            self.game_state.map.player_position
        }

        fn game_state(&self) -> Result<GameState, String> {
            Ok(self.game_state.clone())
        }

        fn on_screen_text(&self, only_message_box: bool) -> Option<String> {
            self.on_screen_text.clone()
        }

        fn menu_state(&self) -> Option<MenuState> {
            None
        }

        fn naming_screen_species(&self) -> Result<crate::pokemon::species::PokemonSpecies, String> {
            Err("not available in stub".to_string())
        }

        fn move_to_learn(&self) -> Option<crate::pokemon::move_name::PokemonMoveName> { None }
        fn learning_pokemon_index(&self) -> usize { 0 }

        fn write_naming_screen_buffer(&mut self, _nickname: Option<&str>) -> Result<(), String> {
            Ok(())
        }

        fn mart_item_list(&self) -> Vec<crate::pokemon::item::ItemId> { vec![] }
        fn mart_item_quantity(&self) -> u8 { 0 }
        fn mart_in_quantity_selector(&self) -> bool { false }
        fn write_max_item_quantity(&mut self, _value: u8) {}

        fn read_game_options(&self) -> Result<GameOptions, String> {
            Err("not available in stub".to_string())
        }

        fn write_game_options(&mut self, options: &GameOptions) -> Result<(), String> {
            Err("not available in stub".to_string())
        }
    }

    #[test]
    fn test_reads_text() {
        const RAW_TEXT: &'static str = include_str!("data/text_box_stream_example.txt");

        let mut reader: PokemonTextReader = Default::default();
        let mut api: StubPokemonApi = Default::default();
        api.game_state.mode = GameMode::TextBox;
        for line in RAW_TEXT.split("\n") {
            api.on_screen_text = Some(String::from(line));
            reader.update(&mut api);
        }

        let result = format!("{}", reader);
        assert_eq!(
            result,
            "PROF.OAK is the authority on POKéMON! Many POKéMON trainers hold him in high regard!"
        );
    }
}