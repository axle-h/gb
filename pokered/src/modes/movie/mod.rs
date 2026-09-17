//! `engine/movie`: the game from power-on to the overworld, and the Hall of Fame into the credits.
//!
//! [`Movie::power_on`] is `Init` to `EnterMap`: the splash and the intro, the title screen, the main
//! menu, and for a new game Oak's speech. It ends by replacing itself with the overworld, so what
//! is under it is whatever was there before power-on, which for a host is nothing.
//!
//! [`Movie::hall_of_fame`] is the whole of the script that calls `HallOfFamePC`, so it saves the
//! game and restarts the console rather than returning to what pushed it. The save it leaves is
//! taken in the Hall of Fame, the one room CONTINUE does not resume in.

mod hall_of_fame;
mod intro;
mod oak_speech;
mod screen;
mod title;
mod wait;

use poke_core::map::Map;
use poke_core::map_objects::fly_warp;
use serde::{Deserialize, Serialize};
use crate::command::Decision;
use crate::gfx::sgb::PaletteCommand;
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::main_menu::{MainMenu, CONTINUE, NEW_GAME};
use crate::modes::overworld::Overworld;
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::world::World;
use hall_of_fame::HallOfFame;
use intro::Intro;
use oak_speech::OakSpeech;
use screen::MovieScreen;
use title::Title;
use wait::{Tick, Wait};

/// `StartNewGame`'s hold after Oak's speech, and `SpecialEnterMap`'s before `EnterMap`.
const AFTER_SPEECH: u16 = 20;
const BEFORE_ENTER_MAP: u16 = 20;
/// `HallOfFameResetEventsAndSaveScript`'s five `DelayFrames 120` after its save.
const SAVED_HOLD: u16 = 600;
/// `INDIGO_PLATEAU_EVENTS_START` to `INDIGO_PLATEAU_EVENTS_END`, both byte-aligned, so the script's
/// `ResetEventRange` clears the range whole.
const INDIGO_PLATEAU_EVENTS: std::ops::RangeInclusive<u16> = 0x8E0..=0x90F;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Movie {
    PowerOn(PowerOn),
    /// `HallOfFameResetEventsAndSaveScript`: `HallOfFamePC`'s ceremony and credits, then the save
    /// and the restart the script ends with.
    HallOfFame(Ceremony),
}

impl Movie {
    /// Power-on. `save_exists` is whether the world the game holds is a save to continue.
    pub fn power_on(save_exists: bool) -> Self {
        Self::PowerOn(PowerOn::new(save_exists))
    }

    /// The Hall of Fame script, which the League's script calls. It never returns: the cartridge
    /// ends it with `jp Init`, so the mode replaces itself with power-on.
    pub fn hall_of_fame() -> Self {
        Self::HallOfFame(Ceremony::default())
    }

    /// `HallOfFamePC` has returned and its script is saving, which is where a driver watching the
    /// credits end should stop, since the game itself goes on to restart.
    pub fn hall_of_fame_returned(&self) -> bool {
        matches!(self, Self::HallOfFame(ceremony) if ceremony.returned())
    }

    /// Presses the title screen has taken, and the one that ends the Hall of Fame, so a driver can
    /// see its own land.
    pub fn answered(&self) -> u32 {
        match self {
            Self::PowerOn(power_on) => power_on.title.answered(),
            Self::HallOfFame(ceremony) => ceremony.answered,
        }
    }

    /// The row the name list's cursor is on.
    pub fn selected(&self) -> u8 {
        match self {
            Self::PowerOn(PowerOn { stage: Stage::OakSpeech(speech), .. }) => speech.selected(),
            _ => 0,
        }
    }
}

impl ModeUpdate for Movie {
    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self {
            Self::PowerOn(power_on) => power_on.update(ctx),
            Self::HallOfFame(ceremony) => ceremony.update(ctx),
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match self {
            Self::PowerOn(power_on) => power_on.resume(outcome, ctx),
            Self::HallOfFame(ceremony) => ceremony.hall_of_fame.resume(ctx),
        }
    }

    fn status(&self) -> Status {
        match self {
            Self::PowerOn(power_on) => power_on.status(),
            Self::HallOfFame(ceremony) => ceremony.status(),
        }
    }
}

/// `HallOfFameResetEventsAndSaveScript`: `HallOfFamePC` and the tail it returns to, which saves the
/// game with the Elite Four beaten, stands THE END for a while, and restarts the console.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ceremony {
    hall_of_fame: HallOfFame,
    /// `None` while `HallOfFamePC` is still running.
    tail: Option<Tail>,
    wait: Wait,
    answered: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Tail {
    /// The hold after `SaveGameData`.
    Held,
    /// `WaitForTextScrollButtonPress`, and then `Init`.
    Press,
}

impl Ceremony {
    fn returned(&self) -> bool {
        self.tail.is_some()
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.tail {
            None => {
                let transition = self.hall_of_fame.update(ctx);
                if self.hall_of_fame.is_done() {
                    return self.save(ctx);
                }
                transition
            }
            Some(Tail::Held) => {
                if self.wait.tick(ctx) != Tick::Waiting {
                    self.tail = Some(Tail::Press);
                }
                Transition::Stay
            }
            Some(Tail::Press) => {
                if !ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
                    return Transition::Stay;
                }
                self.answered += 1;
                self.reset(ctx)
            }
        }
    }

    /// The script after `HallOfFamePC`: the Elite Four's events cleared so the gauntlet can be
    /// fought again, a blackout now returning to Pallet Town, and the game written out.
    fn save(&mut self, ctx: &mut Ctx) -> Transition {
        for event in INDIGO_PLATEAU_EVENTS {
            ctx.world.events.clear(event);
        }
        ctx.world.location.last_blackout_map = Map::PalletTown;
        ctx.save_game = true;
        self.wait = Wait::frames(SAVED_HOLD);
        self.tail = Some(Tail::Held);
        Transition::Stay
    }

    /// `jp Init`: the console restarts on the save the script has just written, and the clock it
    /// cleared with the rest of WRAM only counts again once a map is entered.
    fn reset(&mut self, ctx: &mut Ctx) -> Transition {
        MovieScreen::release(ctx);
        ctx.world.play_time.counting = false;
        Transition::Replace(Mode::Movie(Movie::power_on(true)))
    }

    /// The press that ends the script is the one the title screen is waiting for a frame later.
    fn status(&self) -> Status {
        match self.tail {
            Some(Tail::Press) => Status::Waiting(Decision::TitleScreen),
            _ => Status::Busy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Stage {
    Intro(Intro),
    Title,
    /// The main menu is up.
    MainMenu,
    OakSpeech(OakSpeech),
    /// `SpecialEnterMap`, after `StartNewGame`'s own hold when it came from Oak's speech.
    EnterMap { after_speech: bool },
    /// `DoClearSaveDialogue`: its text is up, then its menu.
    ClearSave { asking: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerOn {
    save_exists: bool,
    stage: Stage,
    title: Title,
    screen: MovieScreen,
    wait: Wait,
}

impl PowerOn {
    fn new(save_exists: bool) -> Self {
        Self { save_exists, stage: Stage::Intro(Intro::default()), title: Title::default(), screen: MovieScreen::default(), wait: Wait::default() }
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match &mut self.stage {
            Stage::Intro(intro) => {
                self.screen.vblank(ctx);
                intro.update(ctx, &mut self.screen);
                if intro.is_done() {
                    self.stage = Stage::Title;
                    self.title = Title::default();
                    self.title.update(ctx, &mut self.screen);
                }
                self.screen.present(ctx);
                Transition::Stay
            }
            Stage::Title => {
                self.screen.vblank(ctx);
                self.title.update(ctx, &mut self.screen);
                self.screen.present(ctx);
                if !self.title.is_done() {
                    return Transition::Stay;
                }
                MovieScreen::release(ctx);
                if self.title.clears_save() {
                    return self.clear_save(ctx);
                }
                self.stage = Stage::MainMenu;
                Transition::Push(Mode::MainMenu(MainMenu::new(self.save_exists)))
            }
            Stage::MainMenu | Stage::ClearSave { .. } => Transition::Stay,
            Stage::OakSpeech(speech) => {
                let transition = speech.update(ctx);
                if speech.is_done() {
                    return self.enter_map(ctx, true);
                }
                transition
            }
            Stage::EnterMap { after_speech } => match self.wait.tick(ctx) {
                Tick::Waiting => Transition::Stay,
                _ if *after_speech => self.enter_map(ctx, false),
                _ => Transition::Replace(Mode::Overworld(Overworld::new())),
            },
        }
    }

    /// `StartNewGame`'s hold, then `SpecialEnterMap`, whose `BIT_GAME_TIMER_COUNTING` starts the clock.
    fn enter_map(&mut self, ctx: &mut Ctx, after_speech: bool) -> Transition {
        if after_speech {
            self.wait = Wait::frames(AFTER_SPEECH);
        } else {
            ctx.world.play_time.counting = true;
            self.wait = Wait::frames(BEFORE_ENTER_MAP);
        }
        self.stage = Stage::EnterMap { after_speech };
        Transition::Stay
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match &mut self.stage {
            Stage::MainMenu => match outcome {
                Outcome::Chosen(NEW_GAME) => {
                    let mut speech = OakSpeech::new();
                    let transition = speech.update(ctx);
                    self.stage = Stage::OakSpeech(speech);
                    transition
                }
                Outcome::Chosen(CONTINUE) => {
                    continue_from_hall_of_fame(ctx);
                    self.enter_map(ctx, false)
                }
                // B: back to `DisplayTitleScreen`.
                _ => {
                    self.stage = Stage::Title;
                    self.title = Title::default();
                    self.update(ctx)
                }
            },
            Stage::OakSpeech(speech) => {
                let transition = speech.resume(ctx);
                if speech.is_done() {
                    return self.enter_map(ctx, true);
                }
                transition
            }
            Stage::ClearSave { asking: false } => {
                self.stage = Stage::ClearSave { asking: true };
                Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::NoYes, (14, 7), false)))
            }
            Stage::ClearSave { asking: true } => {
                // `ClearAllSRAMBanks` on YES, then `Init` either way.
                if outcome == Outcome::Chosen(1) {
                    *ctx.world = World::default();
                    self.save_exists = false;
                }
                Transition::Replace(Mode::Movie(Movie::power_on(self.save_exists)))
            }
            _ => Transition::Stay,
        }
    }

    /// `DoClearSaveDialogue` up to its menu.
    fn clear_save(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.screen.sgb.run(&PaletteCommand::Default);
        ctx.screen.tiles.load_font();
        ctx.screen.tiles.load_text_box_tiles();
        self.stage = Stage::ClearSave { asking: false };
        let text = poke_core::text_script::decode(poke_core::symbols::pokered_symbols::ClearSaveDataText)
            .expect("the clear save text decodes");
        Transition::Push(Mode::TextBox(TextBox::script(text)))
    }

    fn status(&self) -> Status {
        match &self.stage {
            Stage::Title if self.title.is_waiting() => Status::Waiting(Decision::TitleScreen),
            Stage::OakSpeech(speech) if speech.is_choosing_name() => Status::Waiting(Decision::IntroNameMenu),
            _ => Status::Busy,
        }
    }
}

/// `.pressedA`: a game saved inside the Hall of Fame is continued from Pallet Town, on the square a
/// fly lands on, since the room the ceremony saved in has no way out. Every other save is continued
/// where it was left.
fn continue_from_hall_of_fame(ctx: &mut Ctx) {
    if ctx.world.hall_of_fame_teams == 0 || ctx.world.location.map != Map::HallOfFame {
        return;
    }
    let warp = fly_warp(Map::PalletTown).expect("Pallet Town has a fly warp");
    let location = &mut ctx.world.location;
    location.map = Map::PalletTown;
    location.last_map = Map::PalletTown;
    location.x = warp.x;
    location.y = warp.y;
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use poke_core::map::Map;
    use crate::command::{Command, Reply};
    use crate::rng::GameRng;
    use crate::{Game, Input, Pacing};
    use super::*;

    /// Runs until `done`, answering what is asked the way a new player would.
    fn play(game: &mut Game, limit: u32, answer: impl Fn(&Decision) -> Option<Command>, done: impl Fn(&Game) -> bool) -> u32 {
        for frame in 0..limit {
            if done(game) {
                return frame;
            }
            let input = match game.status() {
                Status::Waiting(decision) => answer(&decision).map_or(Input::None, Input::Command),
                _ => Input::None,
            };
            let reply = game.frame(input).reply;
            assert!(!matches!(reply, Some(Reply::Refused(_))), "{reply:?}");
        }
        panic!("not done after {limit} frames: {:?}", game.status());
    }

    #[test]
    fn power_on_reaches_the_title_and_waits() {
        let mut game = Game::power_on(None, GameRng::seeded(1), Pacing::Faithful);
        let frames = play(&mut game, 10_000, |_| None, |game| game.status() == Status::Waiting(Decision::TitleScreen));
        assert!(frames > 700, "the intro plays first: {frames}");
    }

    #[test]
    fn a_new_game_with_preset_names_ends_in_reds_room() {
        let mut game = Game::power_on(None, GameRng::seeded(2), Pacing::Faithful);
        play(&mut game, 30_000, |decision| match decision {
            Decision::TitleScreen | Decision::Text => Some(Command::Advance),
            Decision::MainMenu => Some(Command::ChooseOption(0)),
            Decision::IntroNameMenu => Some(Command::ChooseOption(1)),
            _ => None,
        }, |game| matches!(game.modes(), [Mode::Overworld(_)]));
        let world = game.world();
        assert_eq!(world.player_name, encode("RED").unwrap());
        assert_eq!(world.rival_name, encode("BLUE").unwrap());
        assert_eq!(world.money, [0x00, 0x30, 0x00]);
        assert_eq!(world.location.map, Map::RedsHouse2F);
        assert_eq!(world.pc_items.items.len(), 1);
        assert!(world.play_time.counting);
    }

    #[test]
    fn typed_names_go_through_the_naming_screen() {
        let mut game = Game::power_on(None, GameRng::seeded(3), Pacing::Faithful);
        play(&mut game, 40_000, |decision| match decision {
            Decision::TitleScreen | Decision::Text => Some(Command::Advance),
            Decision::MainMenu => Some(Command::ChooseOption(0)),
            Decision::IntroNameMenu => Some(Command::ChooseOption(0)),
            Decision::NamingScreen => Some(Command::EnterName(encode("AB").unwrap())),
            _ => None,
        }, |game| matches!(game.modes(), [Mode::Overworld(_)]));
        assert_eq!(game.world().player_name, encode("AB").unwrap());
        assert_eq!(game.world().rival_name, encode("AB").unwrap());
    }

    /// A champion standing in the Hall of Fame, as the ceremony's script leaves them.
    fn champion() -> World {
        use poke_core::species::PokemonSpecies;
        use crate::party::Named;
        use crate::systems::add_mon::{new_party_mon, Origin};
        let mut world = World { player_name: encode("RED").unwrap(), ..World::default() };
        world.party = [(PokemonSpecies::Squirtle, 85), (PokemonSpecies::Pidgey, 9)].into_iter().map(|(species, level)| Named {
            mon: new_party_mon(species, level, 1, &Origin::Trainer, &mut GameRng::tape(vec![])),
            ot: encode("RED").unwrap(),
            nick: encode("MON").unwrap(),
        }).collect();
        world.location.map = Map::HallOfFame;
        world
    }

    /// Runs until `HallOfFamePC` has returned and its script is saving.
    fn to_the_end(game: &mut Game) -> u32 {
        play(game, 20_000, |decision| (*decision == Decision::Text).then_some(Command::Advance),
             |game| matches!(game.modes().last(), Some(Mode::Movie(movie)) if movie.hall_of_fame_returned()))
    }

    #[test]
    fn the_hall_of_fame_records_the_team_and_the_credits_end() {
        use poke_core::species::PokemonSpecies;
        let mut game = Game::new(champion(), GameRng::seeded(4), Pacing::Faithful);
        game.push(Mode::Movie(Movie::hall_of_fame()));
        let frames = to_the_end(&mut game);
        let world = game.world();
        assert_eq!(world.hall_of_fame_teams, 1);
        assert_eq!(world.hall_of_fame.len(), 1);
        assert_eq!(world.hall_of_fame[0].iter().map(|mon| (mon.species, mon.level)).collect::<Vec<_>>(),
                   [(PokemonSpecies::Squirtle, 85), (PokemonSpecies::Pidgey, 9)]);
        assert!(!world.one_frame_letter_delay);
        assert!(frames > 5_000, "the credits roll: {frames}");
    }

    /// `HallOfFameResetEventsAndSaveScript` after the credits: the save, the hold, the press and
    /// `Init`.
    #[test]
    fn the_credits_save_the_game_and_restart_the_console() {
        use poke_core::symbols::pokered_events::EVENT_BEAT_LANCE;
        let mut world = champion();
        world.events.set(EVENT_BEAT_LANCE);
        world.location.last_blackout_map = Map::ViridianCity;
        world.play_time.counting = true;
        let mut game = Game::new(world, GameRng::seeded(6), Pacing::Faithful);
        game.push(Mode::Movie(Movie::hall_of_fame()));

        let mut saved = None;
        for _ in 0..20_000 {
            let input = match game.status() {
                Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
                _ => Input::None,
            };
            if let Some(bytes) = game.frame(input).save {
                assert!(saved.is_none(), "the script saves once");
                saved = Some(bytes);
            }
            if matches!(game.modes().last(), Some(Mode::Movie(movie)) if movie.hall_of_fame_returned()) {
                break;
            }
        }
        let saved = saved.expect("the script wrote the game out");
        assert!(!game.world().events.is_set(EVENT_BEAT_LANCE), "the Elite Four can be fought again");
        assert_eq!(game.world().location.last_blackout_map, Map::PalletTown);
        let reloaded = Game::load(&saved, Pacing::Faithful).unwrap();
        assert_eq!(reloaded.world().hall_of_fame_teams, 1);
        assert_eq!(reloaded.world().location.map, Map::HallOfFame, "the save is taken where the ceremony was");

        let held = play(&mut game, 1_000, |_| None, |game| game.status() == Status::Waiting(Decision::TitleScreen));
        assert_eq!(held, SAVED_HOLD as u32, "THE END stands before the press");
        game.frame(Input::Buttons(Joypad::A));
        assert!(matches!(game.modes(), [Mode::Movie(Movie::PowerOn(_))]), "{:?}", game.modes().last());
        assert!(!game.world().play_time.counting, "the clock only counts again on the map");
    }

    /// `.pressedA`'s `wNumHoFTeams` test: the only save continued anywhere but where it was left.
    #[test]
    fn continuing_from_the_hall_of_fame_starts_in_pallet_town() {
        let mut world = champion();
        world.hall_of_fame_teams = 1;
        let landing = fly_warp(Map::PalletTown).unwrap();
        let mut game = Game::power_on(Some(world.clone()), GameRng::seeded(7), Pacing::Faithful);
        play(&mut game, 20_000, |decision| match decision {
            Decision::TitleScreen | Decision::ContinueGame => Some(Command::Advance),
            Decision::MainMenu => Some(Command::ChooseOption(CONTINUE)),
            _ => None,
        }, |game| matches!(game.modes().last(), Some(Mode::Overworld(_))));
        let location = &game.world().location;
        assert_eq!((location.map, location.x, location.y), (Map::PalletTown, landing.x, landing.y));
        assert_eq!(location.last_map, Map::PalletTown);

        // A champion who saved anywhere else is continued where they left off.
        world.location.map = Map::ViridianCity;
        let mut game = Game::power_on(Some(world), GameRng::seeded(7), Pacing::Faithful);
        play(&mut game, 20_000, |decision| match decision {
            Decision::TitleScreen | Decision::ContinueGame => Some(Command::Advance),
            Decision::MainMenu => Some(Command::ChooseOption(CONTINUE)),
            _ => None,
        }, |game| matches!(game.modes().last(), Some(Mode::Overworld(_))));
        assert_eq!(game.world().location.map, Map::ViridianCity);
    }
}
