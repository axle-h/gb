//! `SaveMenu`: the start menu's SAVE row, from the summary of what is about to be written to the
//! "saved the game!" it ends with.
//!
//! `PrintSaveScreenText` draws `DisplayContinueGameInfo`'s box seven rows higher, so the two share
//! it. Then "Would you like to SAVE the game?" and a yes/no. A yes over a save file belonging to
//! another playthrough asks once more, because writing erases it: `CheckPreviousSaveFile` compares
//! the player id in SRAM with the one in play, and here the host says whose save it holds.
//!
//! `SaveGameData` is one frame: `ctx.save_game` asks `Game::frame` for the bytes. What follows is
//! pacing: "Now saving..." held for 120 frames, the text, `SFX_SAVE`, and 30 frames more.

use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::gfx::ui::UiSurface;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::main_menu::save_screen_info;
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};

/// `PrintSaveScreenText`'s `hlcoord 4, 0`.
const INFO_AT: (usize, usize) = (4, 0);
/// `SaveTheGame_YesOrNo`'s `hlcoord 0, 7`, with the cursor at (1, 8) one in from it.
const YES_NO_AT: (usize, usize) = (0, 7);
/// `PrintSaveScreenText`'s `DelayFrames 30`, which stands the summary before the question.
const INFO_STANDS: u16 = 30;
/// The `DelayFrames 120` "Now saving..." is held for, and the `DelayFrames 30` after the sound.
const NOW_SAVING: u16 = 120;
const AFTER_SAVED: u16 = 30;
/// `ClearScreenArea` over the question's box, and where "Now saving..." goes in it.
const CLEARED: (usize, usize, usize, usize) = (1, 13, 18, 4);
const NOW_SAVING_AT: (usize, usize) = (1, 14);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// A `DelayFrames` counting down, and what runs when it ends.
    Hold(u16, After),
    /// A text or a menu is up, and this is what its answer feeds.
    Child(After),
    /// `WaitForSoundToFinish` after `SFX_SAVE`.
    Sound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    /// The summary has stood; ask.
    Ask,
    /// The question is printed; put its menu up.
    Question,
    /// The question is answered.
    Answered,
    /// The warning is printed; put its menu up.
    Warning,
    /// The warning is answered.
    Confirmed,
    /// "Now saving..." has stood; say the game is saved.
    Saved,
    /// The text is done; play the sound.
    Sound,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveMenu {
    phase: Phase,
}

impl SaveMenu {
    pub fn new() -> Self {
        Self { phase: Phase::Hold(INFO_STANDS, After::Ask) }
    }

    /// `PrintText` of one of the menu's own texts.
    fn print(&mut self, label: &str, after: After, ctx: &mut Ctx) -> Transition {
        let script = poke_core::text_script::far_text(label).expect("the save menu's texts are in the cartridge");
        self.phase = Phase::Child(after);
        ctx.update_sprites = true;
        Transition::Push(Mode::TextBox(TextBox::script(script)))
    }

    /// `SaveTheGame_YesOrNo`'s own menu, which leaves `wTwoOptionMenuID` at the `YES_NO_MENU` every
    /// route to the SAVE row has already left there.
    fn yes_no(&mut self, after: After, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(after);
        ctx.update_sprites = true;
        Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
    }

    /// `CheckPreviousSaveFile`: a save file from a playthrough that is not this one is about to be
    /// erased, and the cartridge asks before erasing it.
    fn erases_another_game(ctx: &Ctx) -> bool {
        ctx.saved_player_id.is_some_and(|id| id != ctx.world.player_id)
    }

    /// `.save`: the bytes, then "Now saving..." over the question's box.
    fn save(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.save_game = true;
        let (x, y, width, height) = CLEARED;
        ctx.screen.ui.fill(x, y, width, height, UiSurface::BLANK);
        let text = poke_core::charmap::encode("Now saving...").expect("the string encodes");
        ctx.screen.ui.place(NOW_SAVING_AT.0, NOW_SAVING_AT.1, &text);
        self.phase = Phase::Hold(NOW_SAVING, After::Saved);
        Transition::Stay
    }

    fn step(&mut self, after: After, ctx: &mut Ctx) -> Transition {
        match after {
            After::Ask => self.print("_WouldYouLikeToSaveText", After::Question, ctx),
            After::Question => self.yes_no(After::Answered, ctx),
            After::Warning => self.yes_no(After::Confirmed, ctx),
            After::Saved => self.print("_GameSavedText", After::Sound, ctx),
            After::Sound => {
                ctx.audio.play_sound(sounds::SFX_SAVE);
                self.phase = Phase::Sound;
                Transition::Stay
            }
            After::Done => Transition::Pop(Outcome::Done),
            // Both answers are the pushed menu's, which `resume` takes.
            After::Answered | After::Confirmed => Transition::Stay,
        }
    }
}

impl Default for SaveMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeUpdate for SaveMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        save_screen_info(ctx, INFO_AT);
        ctx.screen.tiles.load_text_box_tiles();
        ctx.update_sprites = true;
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Hold(frames, after) if frames > 1 => {
                self.phase = Phase::Hold(frames - 1, after);
                Transition::Stay
            }
            Phase::Hold(_, after) => self.step(after, ctx),
            Phase::Child(_) => Transition::Stay,
            Phase::Sound if ctx.audio.sound_finished() => {
                self.phase = Phase::Hold(AFTER_SAVED, After::Done);
                Transition::Stay
            }
            Phase::Sound => Transition::Stay,
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        let yes = outcome == Outcome::Chosen(0);
        match after {
            After::Answered if !yes => Transition::Pop(Outcome::Done),
            After::Answered if Self::erases_another_game(ctx) => {
                self.print("_OlderFileWillBeErasedText", After::Warning, ctx)
            }
            After::Answered | After::Confirmed if yes => self.save(ctx),
            After::Confirmed => Transition::Pop(Outcome::Done),
            other => self.step(other, ctx),
        }
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::symbols::pokered_events::EVENT_GOT_POKEDEX;
    use crate::command::{Command, Decision, Reply};
    use crate::input::Joypad;
    use crate::mode::{Mode, Status};
    use crate::modes::pc::bills_pc::{BillsPc, CHANGE_BOX};
    use crate::modes::start_menu::{StartMenu, StartMenuEntry};
    use crate::rng::GameRng;
    use crate::systems::play_time::PlayTime;
    use crate::world::World;
    use crate::{Event, Frame, Game, Input, Pacing};
    use super::*;

    /// The player id in play, and another playthrough's.
    const MINE: u16 = 0x1234;
    const ANOTHER: u16 = 0x4321;
    /// The `:` `PrintPlayTime` writes between the hours and the minutes.
    const TIME_COLON: u8 = 0x6D;

    fn world() -> World {
        let mut world = World {
            player_name: encode("RED").unwrap(),
            badges: 0b0000_0111,
            player_id: MINE,
            play_time: PlayTime { hours: 3, minutes: 7, ..PlayTime::default() },
            boxes: vec![Vec::new(); 3],
            ..World::default()
        };
        world.pokedex.owned[0] = 0b0001_1111;
        world.events.set(EVENT_GOT_POKEDEX as u16);
        world
    }

    /// A game, and the one save it is allowed to write.
    struct Saving {
        game: Game,
        saved: Option<Vec<u8>>,
    }

    impl Saving {
        fn new(mode: Mode) -> Self {
            let mut game = Game::new(world(), GameRng::seeded(0), Pacing::Faithful);
            game.push(mode);
            Self { game, saved: None }
        }

        fn frame(&mut self, input: Input) -> Frame {
            let frame = self.game.frame(input);
            if let Some(bytes) = &frame.save {
                assert!(self.saved.is_none(), "the game wrote itself twice");
                self.saved = Some(bytes.clone());
            }
            frame
        }

        fn settle(&mut self) -> Decision {
            for _ in 0..3000 {
                if let Status::Waiting(decision) = self.game.status() {
                    return decision;
                }
                self.frame(Input::None);
            }
            panic!("nothing ever waited: {:?}", self.game.modes().last());
        }

        fn until(&mut self, decision: Decision) -> u32 {
            for frames in 1..600 {
                self.frame(Input::None);
                if self.game.status() == Status::Waiting(decision.clone()) {
                    return frames;
                }
            }
            panic!("never waited for {decision:?}");
        }

        fn press(&mut self, button: Joypad) {
            self.frame(Input::Buttons(button));
            self.frame(Input::None);
        }

        fn answer(&mut self, decision: Decision, command: Command) {
            assert_eq!(self.settle(), decision, "before {command:?}");
            assert_eq!(self.game.frame(Input::Command(command.clone())).reply, Some(Reply::Accepted));
            for _ in 0..600 {
                if self.frame(Input::None).events.contains(&Event::CommandDone(command.clone())) {
                    return;
                }
            }
            panic!("{command:?} never finished");
        }

        /// A `▼` or a `<CONT>` answered until something that is not a text waits.
        fn read_on(&mut self) -> Decision {
            while self.settle() == Decision::Text {
                self.answer(Decision::Text, Command::Advance);
            }
            self.settle()
        }

        /// Runs until the mode that was pushed has gone, answering how many frames that took.
        fn until_gone(&mut self) -> u32 {
            for frames in 1..600 {
                self.frame(Input::None);
                if self.game.modes().is_empty() {
                    return frames;
                }
            }
            panic!("the save menu never closed");
        }

        fn row(&self, y: usize, from: usize, text: &str) {
            let bytes = encode(text).unwrap();
            assert_eq!(self.game.ui().row(y)[from..from + bytes.len()], bytes[..], "row {y} from {from}");
        }

        fn now_saving(&self) -> bool {
            let text = encode("Now saving...").unwrap();
            self.game.ui().row(NOW_SAVING_AT.1)[NOW_SAVING_AT.0..NOW_SAVING_AT.0 + text.len()] == text[..]
        }
    }

    #[test]
    fn the_screen_shows_the_player_badges_dex_and_time_before_it_asks() {
        let mut save = Saving::new(Mode::SaveMenu(SaveMenu::new()));
        save.row(2, 5, "PLAYER");
        save.row(2, 12, "RED");
        save.row(4, 5, "BADGES");
        save.row(4, 18, "3");
        save.row(6, 5, "POKéDEX");
        save.row(6, 18, "5");
        save.row(8, 5, "TIME");
        save.row(8, 15, "3");
        assert_eq!(save.game.ui().get(16, 8), TIME_COLON);
        save.row(8, 17, "07");

        assert!(save.until(Decision::TwoOption) > INFO_STANDS as u32, "the summary stands first");
        save.row(14, 1, "Would you like to");
        save.row(16, 1, "SAVE the game?");
    }

    #[test]
    fn yes_writes_the_game_and_holds_now_saving_for_a_hundred_and_twenty_frames() {
        let mut save = Saving::new(Mode::SaveMenu(SaveMenu::new()));
        save.until(Decision::TwoOption);
        save.press(Joypad::A);

        let mut appeared = None;
        let mut held = None;
        for frame in 1..600u32 {
            save.frame(Input::None);
            match (appeared, save.now_saving()) {
                (None, true) => appeared = Some(frame),
                (Some(at), false) => {
                    held = Some(frame - at);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(held, Some(NOW_SAVING as u32), "the frames it is held for");
        assert!(save.saved.is_some(), "the bytes are the host's to keep");
        let closed = save.until_gone();
        save.row(14, 1, "RED saved");
        save.row(16, 1, "the game!");
        assert!(closed > AFTER_SAVED as u32, "the text stands while the sound plays out");
    }

    #[test]
    fn no_leaves_without_writing_anything() {
        let mut save = Saving::new(Mode::SaveMenu(SaveMenu::new()));
        save.until(Decision::TwoOption);
        // B answers the second option, which is NO.
        save.press(Joypad::B);
        save.until_gone();
        assert!(save.saved.is_none());
    }

    /// `CheckPreviousSaveFile`: only a save file belonging to someone else is asked about.
    #[test]
    fn a_save_over_another_playthrough_asks_once_more() {
        let mut save = Saving::new(Mode::SaveMenu(SaveMenu::new()));
        save.game.set_saved_player_id(Some(ANOTHER));
        save.until(Decision::TwoOption);
        save.press(Joypad::A);
        // `_OlderFileWillBeErasedText`'s `cont` is a `▼` that scrolls before the menu goes up.
        assert_eq!(save.settle(), Decision::Text);
        save.row(14, 1, "The older file");
        save.row(16, 1, "will be erased to");
        assert_eq!(save.read_on(), Decision::TwoOption);
        save.row(16, 1, "save. Okay?");
        save.press(Joypad::B);
        save.until_gone();
        assert!(save.saved.is_none(), "the older file is still the host's");
    }

    #[test]
    fn the_same_playthrough_is_never_asked_twice() {
        let mut save = Saving::new(Mode::SaveMenu(SaveMenu::new()));
        save.game.set_saved_player_id(Some(MINE));
        save.until(Decision::TwoOption);
        save.press(Joypad::A);
        save.until_gone();
        assert!(save.saved.is_some());
    }

    #[test]
    fn a_save_taken_and_reloaded_is_the_same_game() {
        let mut save = Saving::new(Mode::SaveMenu(SaveMenu::new()));
        save.until(Decision::TwoOption);
        save.press(Joypad::A);
        while save.saved.is_none() {
            save.frame(Input::None);
        }
        let bytes = save.saved.clone().unwrap();
        let loaded = Game::load(&bytes, Pacing::Faithful).unwrap();
        assert_eq!(loaded.save(), bytes, "the bytes are of the game they came from");
        assert_eq!(loaded.world(), save.game.world());
        // The file is now this playthrough's, so the next save asks nothing.
        assert_eq!(loaded.saved_player_id, Some(MINE));
    }

    #[test]
    fn the_start_menus_save_row_opens_it_and_saving_closes_the_menu() {
        let mut save = Saving::new(Mode::StartMenu(StartMenu::new()));
        save.answer(Decision::StartMenu, Command::ChooseStartMenuEntry(StartMenuEntry::SaveReset));
        save.until(Decision::TwoOption);
        save.press(Joypad::A);
        save.until_gone();
        assert!(save.saved.is_some());
    }

    /// `ChangeBox`'s `SaveGameData`, which is the save menu's without the screen around it.
    #[test]
    fn changing_a_box_writes_the_same_save() {
        let mut save = Saving::new(Mode::BillsPc(BillsPc::direct()));
        assert_eq!(save.read_on(), Decision::CursorMenu);
        save.answer(Decision::CursorMenu, Command::ChooseOption(CHANGE_BOX));
        assert_eq!(save.read_on(), Decision::TwoOption);
        save.answer(Decision::TwoOption, Command::ChooseOption(0));
        assert_eq!(save.read_on(), Decision::CursorMenu);
        save.answer(Decision::CursorMenu, Command::ChooseOption(2));
        assert_eq!(save.game.world().current_box, 2);
        let bytes = save.saved.clone().expect("the box change wrote the game");
        assert_eq!(Game::load(&bytes, Pacing::Faithful).unwrap().world().current_box, 2);
    }
}
