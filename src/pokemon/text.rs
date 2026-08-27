use std::fmt::{Display, Formatter};
use crate::joypad::JoypadButton;
use crate::pokemon::{PokemonApi, PokemonApiTrait};

/// Reads what the game is saying, one frame at a time, out of the tile map.
///
/// ⚠️ **The screen is a *page being typed*, not a stream to be spliced, and treating it as a stream
/// is what produced 5 KB text boxes.** A Gen 1 box types its page out a character at a time, so
/// consecutive frames are prefixes of one another; the page then clears and the next one starts.
/// The old accumulator instead looked for the longest suffix of everything read so far that was a
/// prefix of this frame, and appended the remainder. That works while the frames arrive in order
/// and fails permanently the moment one does not: `AutoBgMapTransfer` copies a third of the screen
/// per V-blank, so a frame can carry the message without the row above it, and once one such frame
/// has been appended the tail no longer matches anything. The whole screen is then re-appended,
/// which makes the tail match again next frame, which makes it *not* match the frame after — a
/// sawtooth that grows quadratically. Reproduced from the deployed run's own save state
/// (`issues/turn-440/state.gbst`, `RandomPolicy::seeded(3)`) at **1404 bytes** in ten emulated
/// minutes:
///
/// ```text
/// Emb GEODUDE 10Ember GEODUDE 10Ember u GEODUDE 10Ember use GEODUDE 10Ember used E…
/// ```
///
/// ⚠️ **`page` and `buffer` are two different things and merging them is the bug.** `page` is what
/// is on screen *now* and is replaced wholesale; `buffer` is everything committed by a page that
/// has already gone. Nothing is ever appended to `buffer` character by character, so the most any
/// misread frame can cost is one duplicated *page* rather than one per frame for the rest of the
/// box.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct PokemonTextReader {
    /// Pages that have already been replaced, joined by spaces.
    buffer: String,
    /// The page currently on screen, as last read. Committed to `buffer` when it goes.
    page: String,
    /// Consecutive reads that did not continue `page`. See [`Self::update_with`]'s rule 3.
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


    /// Everything read so far, leaving the reader empty and still configured the way it was.
    ///
    /// ⚠️ **Reading a box and *reporting* it are two different moments, and they used to be the
    /// same one.** The agent emitted the buffer only on the `TextBox → not a text box` edge, from
    /// inside [`AgentState::ReadingTextBox`] — so anything that took the state away first threw the
    /// words on the floor. This is what [`PokemonAgent::flush_text_reader`] hands out, and it clears
    /// rather than replaces so the battle reader's `message_box_only` survives being drained.
    ///
    /// [`AgentState::ReadingTextBox`]: crate::pokemon::agent::AgentState
    /// [`PokemonAgent::flush_text_reader`]: crate::pokemon::agent::PokemonAgent
    pub fn take(&mut self) -> String {
        let out = self.committed();
        self.buffer.clear();
        self.page.clear();
        self.mismatches = 0;
        out
    }

    /// Everything read so far: the committed pages plus the one still on screen.
    ///
    /// ⚠️ **The open page counts.** The reader is drained wherever it stops being the thing in
    /// charge ([`crate::pokemon::agent::PokemonAgent::flush_text_reader`]) and that is usually
    /// mid-page, so a version that reported only `buffer` would throw the last page of every box
    /// away — which is the bug `flush_text_reader` exists to fix, one level down.
    fn committed(&self) -> String {
        match (self.buffer.is_empty(), self.page.is_empty()) {
            (_, true) => self.buffer.clone(),
            (true, false) => self.page.clone(),
            (false, false) => format!("{} {}", self.buffer, self.page),
        }
    }

    pub fn update<A: PokemonApiTrait>(&mut self, api: &mut A) {
        // mash the A button to advance the text
        self.update_with(api, JoypadButton::A);
    }

    /// [`Self::update`], but advancing with `button` instead of A.
    ///
    /// ⚠️ **The caller picks the button; the reader still reads.** Both A and B advance a Gen 1 text
    /// box (`ManualTextScroll` waits on either), so B is a drop-in for reading purposes — but where
    /// the two differ is on a *menu*, and this reader cannot tell a menu from a message
    /// (`GameMode::TextBox` comes from `wFontLoaded` alone; see `encoding.rs`'s `TODO menu vs
    /// dialogue`). The places that matter today are the PC menus, which A-mashing cannot leave;
    /// see [`PokemonApiTrait::in_pc_menu`]. Accumulation is unconditional either way, so the
    /// text that scrolled past on the way out is still reported when the box closes.
    pub fn update_with<A: PokemonApiTrait>(&mut self, api: &mut A, button: JoypadButton) {
        api.toggle_button(button);

        let Some(screen) = api.on_screen_text(self.message_box_only) else { return };

        // ⚠️ **A blank frame is not a page break and must not commit anything.** It is far more
        // often the screen mid-redraw: a battle animation blanks the tile map for a frame or two at
        // a time, and the deployed sawtooth committed the half-typed page on every one of them. A
        // page break is recognised by the *text that comes back* not continuing the page, which
        // rules 2 and 3 below do whether or not a blank came between.
        if screen.is_empty() {
            return;
        }
        if self.page.is_empty() {
            self.page = screen;
            return;
        }
        // Still the same page being typed. ⚠️ **Both directions**, because a frame can arrive
        // *shorter* than the last one: the tile map is transferred a third of a screen at a time,
        // so a row can blank a beat before it is redrawn.
        if screen.starts_with(self.page.as_str()) || self.page.starts_with(screen.as_str()) {
            if screen.len() > self.page.len() {
                self.page = screen;
            }
            self.mismatches = 0;
            return;
        }
        // ⚠️ **One read of something else is not a page break, because a *torn* frame reads like
        // one.** `AutoBgMapTransfer` copies a third of the screen per V-blank, so a two-line box
        // being rewritten shows one line of the new text above one line of the old for a frame:
        // `"Our POKéMON's an outsider, outsider, so it's"` — neither a prefix of the page nor a
        // continuation of it, and committed straight into the middle of the sentence. A real page
        // break persists; a tear is gone by the next read.
        self.mismatches += 1;
        if self.mismatches < MISMATCHES_BEFORE_PAGE_BREAK {
            return;
        }
        self.mismatches = 0;
        // A different page. A Gen 1 box *scrolling* looks like this — the second line becomes the
        // first and a new second line is typed under it — so splice on the overlap where there is
        // one, and commit outright where there is none.
        //
        // ⚠️ **The overlap is searched against the page, never against `buffer`.** Against the whole
        // accumulated text it is the old algorithm again: a chance match deep in the history splices
        // a page into the middle of a sentence, and a miss re-appends everything.
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

    /// Move the page on screen into the committed text.
    ///
    /// ⚠️ **Verbatim, joined with a space, and both cleverer versions tried here deleted real
    /// text.** Splicing the page onto the buffer's tail on their overlap, and dropping a page
    /// already contained in it, are the obvious way to tidy up a torn frame committed mid-scroll —
    /// and the cartridge repeats itself constantly: `"Ember used RAGE!"` once per turn of a
    /// five-turn battle, `"Critical hit!"`, `"Got away safely!"`. Any lookback long enough to catch
    /// a tear is long enough to catch those. Measured on the deployed states, it took the worst box
    /// from 654 bytes to 398 by **deleting four turns of a battle**. Deduplication belongs where a
    /// frame is compared with the page it is redrawing, in [`Self::update_with`], where the two are
    /// known to be the same page. Nowhere else.
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
///
/// Two, which is the smallest number that outlives a torn frame. The cost of being wrong is that a
/// page break is noticed one tick late, and the frame it starts the new page from is a superset of
/// the one it skipped, so nothing is lost by it.
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

        fn in_pc_menu(&self) -> bool {
            false
        }

        fn raw_player_coords(&self) -> crate::geometry::Point8 {
            self.game_state.map.player_position
        }

        fn game_state(&self) -> Result<GameState, String> {
            Ok(self.game_state.clone())
        }

        fn bag_item_quantity(&self, _item: crate::pokemon::item::ItemId) -> u8 { 0 }
        fn pc_box_item_position(&self, _item: crate::pokemon::item::ItemId) -> Option<u8> { None }
        fn pc_box_item_quantity(&self, _item: crate::pokemon::item::ItemId) -> u8 { 0 }
        fn pc_stored_items(&self) -> crate::pokemon::bag::Bag { crate::pokemon::bag::Bag::default() }

        fn on_screen_text(&self, only_message_box: bool) -> Option<String> {
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

        fn write_game_options(&mut self, options: &GameOptions) -> Result<(), String> {
            Err("not available in stub".to_string())
        }
    }

    /// ⚠️ **A frame that arrives out of order must not duplicate the page.**
    ///
    /// This is the deployed sawtooth, reduced: `AutoBgMapTransfer` copies a third of the screen per
    /// V-blank, so the battle HUD row and the message row can disagree for a beat, and the screen
    /// reads empty in between while a move animation plays. Against the old accumulator this exact
    /// sequence produced
    /// `"Emb ONIX 14Ember us ONIX 14Ember use ONIX 14Ember used EMBER!"`; the real thing reached
    /// 5320 bytes.
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
    /// with no blank frame between the two. The shared line must be said once.
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

    /// ⚠️ **`take` includes the page still on screen.** The agent drains the reader wherever it
    /// stops being the thing in charge, which is usually mid-page — a version that reported only
    /// the committed pages would throw away the last thing every blocker in the game says.
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