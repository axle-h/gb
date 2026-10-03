//! What a game says, as one line per message for a host to show, whoever is playing: a person at
//! the keyboard as well as an agent, which reports its own reading as `AgentEvent::TextBox`.

use pokered::Game;
use pokered::gfx::ui::UiSurface;
use pokered::mode::Mode;
use crate::pokemon::PokemonApi;
use crate::pokemon::font::FontAware;
use crate::pokemon::text::PokemonTextReader;

pub struct Speech {
    reader: PokemonTextReader,
    /// Frames in a row with no message up.
    quiet: u8,
    /// How many of those end a message.
    quiet_frames: u8,
}

impl Speech {
    /// The cartridge's, read from `wTileMap` a frame at a time. A box blank for a frame or two is
    /// being redrawn, so only a longer silence ends a line; a paragraph break is longer, and so a
    /// line of its own.
    pub fn emulated() -> Self {
        Self { reader: PokemonTextReader::untorn(), quiet: 0, quiet_frames: 3 }
    }

    /// The recreation's, read from the box while a text box is up and from each box as it was
    /// left, since a line can come and go within a frame. One box keeps a message up to its end.
    pub fn native() -> Self {
        Self { reader: PokemonTextReader::untorn(), quiet: 0, quiet_frames: 1 }
    }

    /// One emulated frame, answering a message that has just ended. The battle menu's box shares
    /// the message box's rows, so its labels are read as a message too.
    pub fn hear_emulated(&mut self, api: &PokemonApi) -> Option<String> {
        let page = api.mmu().pokemon_font_loaded().then(|| api.tile_map_glyphs(true))
            .filter(|page| !page.trim().is_empty());
        let up = page.is_some();
        self.reader.read(page);
        self.heard(up)
    }

    /// One native frame: what its text boxes printed, then the game after it.
    pub fn hear_native(&mut self, printed: &[UiSurface], game: &Game) -> Option<String> {
        for page in printed {
            self.reader.read(Some(crate::pokemon::native::message_box_text(page)));
        }
        let up = matches!(game.modes().last(), Some(Mode::TextBox(_) | Mode::Evolution(_)));
        if up {
            self.reader.read(Some(crate::pokemon::native::message_box_text(game.ui())));
        }
        self.heard(up || !printed.is_empty())
    }

    fn heard(&mut self, up: bool) -> Option<String> {
        if up {
            self.quiet = 0;
            return None;
        }
        self.quiet = self.quiet.saturating_add(1);
        if self.quiet != self.quiet_frames {
            return None;
        }
        Some(self.reader.take()).filter(|message| !message.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pokered::command::{Command, Decision};
    use pokered::mode::Status;
    use pokered::rng::GameRng;
    use pokered::{Input, Pacing};

    /// Oak's welcome, each message once and whole, from a new game played by commands.
    #[test]
    fn the_native_intro_is_heard_a_message_at_a_time() {
        let mut game = Game::power_on(GameRng::seeded(1), Pacing::Instant);
        let mut speech = Speech::native();
        let mut heard = Vec::new();
        while heard.len() < 3 {
            assert!(game.frames() < 100_000, "{heard:?}");
            let input = match game.status() {
                Status::Waiting(Decision::TitleScreen | Decision::Text) => Input::Command(Command::Advance),
                Status::Waiting(Decision::MainMenu) => Input::Command(Command::ChooseOption(0)),
                _ => Input::None,
            };
            let frame = game.frame(input);
            heard.extend(speech.hear_native(&frame.printed, &game));
        }
        assert!(heard[0].starts_with("Hello there! Welcome to the world of POKéMON!"), "{heard:?}");
        assert_eq!(heard.iter().collect::<std::collections::HashSet<_>>().len(), heard.len(), "{heard:?}");
    }
}
