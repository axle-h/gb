use std::fmt::{Display, Formatter};
use gb::joypad::JoypadButton;
use crate::pokemon::PokemonApiTrait;

/// Reads what the game is saying, one frame at a time, out of the tile map.
/// ```text
/// Emb GEODUDE 10Ember GEODUDE 10Ember u GEODUDE 10Ember use GEODUDE 10Ember used E…
/// ```
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct PokemonTextReader {
    /// Pages that have already been replaced, joined by spaces.
    buffer: String,
    /// The page currently on screen, as last read.
    page: String,
    /// Consecutive reads that did not continue `page`.
    mismatches: u8,
    message_box_only: bool,
}

impl Display for PokemonTextReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.committed().as_str())
    }
}

impl PokemonTextReader {
    pub fn message_box_only() -> Self {
        Self {
            message_box_only: true,
            ..Self::default()
        }
    }

    pub fn take(&mut self) -> String {
        let out = self.committed();
        self.buffer.clear();
        self.page.clear();
        self.mismatches = 0;
        out
    }

    /// Everything read so far: the committed pages plus the one still on screen.
    fn committed(&self) -> String {
        match (self.buffer.is_empty(), self.page.is_empty()) {
            (_, true) => self.buffer.clone(),
            (true, false) => self.page.clone(),
            (false, false) => format!("{} {}", self.buffer, self.page),
        }
    }

    pub fn update<A: PokemonApiTrait>(&mut self, api: &mut A) {
        self.update_with(api, JoypadButton::A);
    }

    /// [`Self::update`], but advancing with `button` instead of A.
    pub fn update_with<A: PokemonApiTrait>(&mut self, api: &mut A, button: JoypadButton) {
        api.toggle_button(button);
        self.accumulate(api);
    }

    /// [`Self::update_with`] without the button: read this tick's screen and press nothing.
    pub fn accumulate<A: PokemonApiTrait>(&mut self, api: &A) {
        let Some(screen) = api.on_screen_text(self.message_box_only) else { return };

        // A blank frame is not a page break and must not commit anything.
        if screen.is_empty() {
            return;
        }
        if self.page.is_empty() {
            self.page = screen;
            return;
        }
        // Still the same page being typed.
        if screen.starts_with(self.page.as_str()) || self.page.starts_with(screen.as_str()) {
            if screen.len() > self.page.len() {
                self.page = screen;
            }
            self.mismatches = 0;
            return;
        }
        // One read of something else is not a page break, because a torn frame reads like one.
        self.mismatches += 1;
        if self.mismatches < MISMATCHES_BEFORE_PAGE_BREAK {
            return;
        }
        self.mismatches = 0;
        // A different page.
        let overlap = longest_overlap(&self.page, &screen);
        match overlap {
            0 => {
                self.commit_page();
                self.page = screen;
            }
            n => {
                let tail: String = screen.chars().skip(n).collect();
                if !tail.is_empty() {
                    self.page.push_str(&tail);
                }
            }
        }
    }

    /// Move the page on screen into the committed text, verbatim: deduplicating deletes real text.
    fn commit_page(&mut self) {
        if self.page.is_empty() {
            return;
        }
        if !self.buffer.is_empty() {
            self.buffer.push(' ');
        }
        let page = std::mem::take(&mut self.page);
        self.buffer.push_str(&page);
    }
}

/// How many consecutive reads must fail to continue the page before it is taken to have ended.
const MISMATCHES_BEFORE_PAGE_BREAK: u8 = 2;

/// The length, in `char`s, of the longest suffix of `left` that is a prefix of `right`.
fn longest_overlap(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    (1..=left.len().min(right.len()))
        .rev()
        .find(|&n| left[left.len() - n..] == right[..n])
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use gb::joypad::JoypadButtonState;
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

        fn toggle_button(&mut self, button: JoypadButton) {
            self.joypad.update_button(button, !self.joypad.is_button_pressed(button))
        }

        fn read_joypad_state(&self) -> JoypadButtonState {
            self.joypad
        }

        fn game_mode(&self) -> Option<GameMode> {
            Some(self.game_state.mode)
        }

        fn a_game_is_loaded(&self) -> bool {
            true
        }

        fn trainer_battle_pending(&self) -> bool {
            false
        }

        fn in_pc_menu(&self) -> bool {
            false
        }

        fn raw_player_coords(&self) -> gb::geometry::Point8 {
            self.game_state.map.player_position
        }

        fn game_state(&self) -> Result<GameState, String> {
            Ok(self.game_state.clone())
        }

        fn bag_item_quantity(&self, _item: crate::pokemon::item::ItemId) -> u8 { 0 }
        fn pc_box_item_position(&self, _item: crate::pokemon::item::ItemId) -> Option<u8> { None }
        fn pc_box_item_quantity(&self, _item: crate::pokemon::item::ItemId) -> u8 { 0 }
        fn pc_stored_items(&self) -> crate::pokemon::bag::Bag { crate::pokemon::bag::Bag::default() }

        fn on_screen_text(&self, _only_message_box: bool) -> Option<String> {
            self.on_screen_text.clone()
        }

        fn menu_state(&self) -> Option<MenuState> {
            None
        }

        fn list_menu_id(&self) -> u8 {
            0
        }

        fn menu_geometry(&self) -> (u8, u8, u8, u8) { (0, 0, 0, 0) }
        fn bag_item_position(&self, _item: crate::pokemon::item::ItemId) -> Option<u8> { None }

        fn item_price(&self, _item: crate::pokemon::item::ItemId) -> Option<u32> { None }

        fn naming_screen_species(&self) -> Result<crate::pokemon::species::PokemonSpecies, String> {
            Err("not available in stub".to_string())
        }

        fn move_to_learn(&self) -> Option<crate::pokemon::move_name::PokemonMoveName> { None }
        fn learning_pokemon_index(&self) -> usize { 0 }

        fn write_naming_screen_buffer(&mut self, _nickname: Option<&str>) -> Result<(), String> {
            Ok(())
        }

        fn write_player_name(&mut self, _name: &str) -> Result<(), String> {
            Ok(())
        }

        fn mart_item_list(&self) -> Vec<crate::pokemon::item::ItemId> { vec![] }
        fn mart_item_quantity(&self) -> u8 { 0 }
        fn mart_in_quantity_selector(&self) -> bool { false }
        fn write_max_item_quantity(&mut self, _value: u8) {}

        fn read_game_options(&self) -> Result<GameOptions, String> {
            Err("not available in stub".to_string())
        }

    }

    /// A frame that arrives out of order must not duplicate the page.
    #[test]
    fn a_frame_out_of_order_does_not_duplicate_the_page() {
        let frames = [
            "Emb", "", "ONIX 14Ember", "", "ONIX 14Ember us", "", "ONIX 14Ember use", "",
            "ONIX 14Ember used", "", "ONIX 14Ember used EMBER!",
        ];
        let mut reader: PokemonTextReader = Default::default();
        let mut api: StubPokemonApi = Default::default();
        api.game_state.mode = GameMode::TextBox;
        for frame in frames {
            api.on_screen_text = Some(frame.to_string());
            reader.update(&mut api);
        }
        let read = reader.to_string();
        assert_eq!(
            read.matches("Ember used").count(),
            1,
            "the sentence is read once, not once per frame: {read:?}",
        );
        assert!(read.ends_with("Ember used EMBER!"), "and it is the whole of it: {read:?}");
    }

    /// A box that scrolls replaces its first line with its second and types a new one underneath,
    /// with no blank frame between the two.
    #[test]
    fn a_box_that_scrolls_says_the_shared_line_once() {
        let frames = [
            "PROF.OAK is the", "PROF.OAK is the authority", "PROF.OAK is the authority on POKéMON!",
            "authority on POKéMON! Many", "authority on POKéMON! Many trainers",
        ];
        let mut reader: PokemonTextReader = Default::default();
        let mut api: StubPokemonApi = Default::default();
        api.game_state.mode = GameMode::TextBox;
        for frame in frames {
            api.on_screen_text = Some(frame.to_string());
            reader.update(&mut api);
        }
        assert_eq!(
            reader.to_string(),
            "PROF.OAK is the authority on POKéMON! Many trainers",
        );
    }

    /// `take` includes the page still on screen.
    #[test]
    fn taking_the_reader_mid_page_keeps_what_is_on_screen() {
        let mut reader: PokemonTextReader = Default::default();
        let mut api: StubPokemonApi = Default::default();
        api.game_state.mode = GameMode::TextBox;
        // The last frame twice: a real page persists for many ticks, and one read of something
        // that does not continue the page is a torn frame rather than a page break.
        for frame in ["You don't have the", "", "BOULDERBADGE yet!", "BOULDERBADGE yet!"] {
            api.on_screen_text = Some(frame.to_string());
            reader.update(&mut api);
        }
        assert_eq!(reader.take(), "You don't have the BOULDERBADGE yet!");
        assert_eq!(reader.take(), "", "and it is emptied by the drain");
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