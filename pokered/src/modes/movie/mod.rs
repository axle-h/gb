//! `engine/movie`: the game from power-on to the overworld, and the Hall of Fame into the credits.
//!
//! [`Movie::power_on`] is `Init` to `EnterMap`: the splash and the intro, the title screen, the main
//! menu, and for a new game Oak's speech. It ends by replacing itself with the overworld, so what
//! is under it is whatever was there before power-on, which for a host is nothing.
//!
//! [`Movie::hall_of_fame`] is the whole of the script that calls `HallOfFamePC`, so it saves the
//! game and restarts the console rather than returning to the overworld it took the place of. Its
//! save is the autosave slot, holding the game as the cartridge's CONTINUE resumed it: in Pallet
//! Town, since the room the ceremony is in has no way out.

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
use crate::modes::main_menu::{MainMenu, NEW_GAME};
use crate::modes::overworld::Overworld;
use crate::rng::GameRng;
use crate::save_slots::SlotAction;
use crate::world::World;
use crate::{Game, Pacing};
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
    pub fn power_on() -> Self {
        Self::PowerOn(PowerOn::new())
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
        ctx.slot = Some(SlotAction::Autosave);
        self.wait = Wait::frames(SAVED_HOLD);
        self.tail = Some(Tail::Held);
        Transition::Stay
    }

    /// `jp Init`: the console restarts, and the clock it cleared with the rest of WRAM only counts
    /// again once a map is entered.
    fn reset(&mut self, ctx: &mut Ctx) -> Transition {
        MovieScreen::release(ctx);
        ctx.world.play_time.counting = false;
        Transition::Replace(Mode::Movie(Movie::power_on()))
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
    /// `StartNewGame`'s hold, then `SpecialEnterMap`.
    EnterMap { after_speech: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerOn {
    stage: Stage,
    title: Title,
    screen: MovieScreen,
    wait: Wait,
}

impl PowerOn {
    fn new() -> Self {
        Self {
            stage: Stage::Intro(Intro::default()),
            title: Title::default(),
            screen: MovieScreen::default(),
            wait: Wait::default(),
        }
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
                self.stage = Stage::MainMenu;
                Transition::Push(Mode::MainMenu(MainMenu::new()))
            }
            Stage::MainMenu => Transition::Stay,
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
                _ => Transition::Replace(Mode::Overworld(Overworld::reset_player_sprite_data(&mut ctx.world.location))),
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
            _ => Transition::Stay,
        }
    }

    fn status(&self) -> Status {
        match &self.stage {
            Stage::Title if self.title.is_waiting() => Status::Waiting(Decision::TitleScreen),
            Stage::OakSpeech(speech) if speech.is_choosing_name() => Status::Waiting(Decision::IntroNameMenu),
            _ => Status::Busy,
        }
    }
}

/// The game the Hall of Fame's autosave holds: the cartridge's CONTINUE of the save the ceremony
/// wrote, which `.pressedA` moves to the square a fly lands on in Pallet Town, the main menu's
/// screen setup, and `SpecialEnterMap`. It is played until the overworld settles, so its picture
/// shows the player.
pub(crate) fn continued_after_hall_of_fame(mut world: World, rng: GameRng, pacing: Pacing) -> Game {
    let warp = fly_warp(Map::PalletTown).expect("Pallet Town has a fly warp");
    let location = &mut world.location;
    location.map = Map::PalletTown;
    location.last_map = Map::PalletTown;
    location.x = warp.x;
    location.y = warp.y;
    world.play_time.counting = true;
    world.one_frame_letter_delay = false;
    let mut game = Game::new(world, rng, pacing);
    game.screen.sgb.run(&PaletteCommand::Default);
    game.screen.tiles.load_text_box_tiles();
    game.screen.tiles.load_font();
    let overworld = Overworld::reset_player_sprite_data(&mut game.world.location);
    game.push(Mode::Overworld(overworld));
    for _ in 0..60 {
        if game.status() != Status::Busy {
            break;
        }
        game.frame(crate::Input::None);
    }
    game
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
        let mut game = Game::power_on(GameRng::seeded(1), Pacing::Faithful);
        let frames = play(&mut game, 10_000, |_| None, |game| game.status() == Status::Waiting(Decision::TitleScreen));
        assert!(frames > 700, "the intro plays first: {frames}");
    }

    #[test]
    fn a_new_game_with_preset_names_ends_in_reds_room() {
        let mut game = Game::power_on(GameRng::seeded(2), Pacing::Faithful);
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
        let mut game = Game::power_on(GameRng::seeded(3), Pacing::Faithful);
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
    /// `Init`. The save is the autosave slot, of the game continued in Pallet Town.
    #[test]
    fn the_credits_save_the_game_and_restart_the_console() {
        use poke_core::symbols::pokered_events::EVENT_BEAT_LANCE;
        use crate::save_slots::{SlotRequest, AUTOSAVE};
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
            if let Some(request) = game.frame(input).slot {
                assert!(saved.is_none(), "the script saves once");
                saved = Some(request);
            }
            if matches!(game.modes().last(), Some(Mode::Movie(movie)) if movie.hall_of_fame_returned()) {
                break;
            }
        }
        let Some(SlotRequest::Save { slot, bytes, summary }) = saved else { panic!("the script wrote no slot: {saved:?}") };
        assert_eq!(slot, AUTOSAVE);
        assert!(!game.world().events.is_set(EVENT_BEAT_LANCE), "the Elite Four can be fought again");
        assert_eq!(game.world().location.last_blackout_map, Map::PalletTown);
        let reloaded = Game::load(&bytes, Pacing::Faithful).unwrap();
        let landing = fly_warp(Map::PalletTown).unwrap();
        let location = &reloaded.world().location;
        assert_eq!((location.map, location.x, location.y, location.last_map), (Map::PalletTown, landing.x, landing.y, Map::PalletTown),
                   "the autosave is continued where a fly lands, not in the room the ceremony was in");
        assert_eq!(reloaded.world().hall_of_fame_teams, 1);
        assert!(!reloaded.world().events.is_set(EVENT_BEAT_LANCE));
        assert!(reloaded.world().play_time.counting, "SpecialEnterMap starts the clock");
        assert!(matches!(reloaded.modes(), [Mode::Overworld(_)]), "{:?}", reloaded.modes());
        assert_eq!(summary.map, Map::PalletTown);

        let held = play(&mut game, 1_000, |_| None, |game| game.status() == Status::Waiting(Decision::TitleScreen));
        assert_eq!(held, SAVED_HOLD as u32, "THE END stands before the press");
        // The press a driver answers the title with, which is what THE END waits on.
        let pressed = game.frame(Input::Command(Command::Advance));
        assert_eq!(pressed.reply, Some(Reply::Accepted));
        assert!(matches!(game.modes(), [Mode::Movie(Movie::PowerOn(_))]), "{:?}", game.modes().last());
        assert_eq!(game.frame(Input::None).events, [crate::Event::CommandDone(Command::Advance)]);
        assert!(!game.world().play_time.counting, "the clock only counts again on the map");
    }

    /// The cartridge continues a game finished in the Hall of Fame from Pallet Town: here CONTINUE
    /// after the credits opens the slots on the autosave, the newest, and A and LOAD play on there.
    #[test]
    fn continuing_after_the_credits_loads_the_autosave_in_pallet_town() {
        use crate::save_slots::{MemoryStore, SavedAt, SlotStore, AUTOSAVE};
        let mut store = MemoryStore::default();
        let mut game = Game::new(champion(), GameRng::seeded(7), Pacing::Faithful);
        game.set_slots(store.slots());
        game.push(Mode::Movie(Movie::hall_of_fame()));
        let (mut chose_continue, mut pressed) = (false, false);
        for frame in 0..40_000 {
            if chose_continue && matches!(game.modes(), [Mode::Overworld(_)]) {
                break;
            }
            let input = match game.status() {
                Status::Waiting(Decision::Text | Decision::TitleScreen) => Input::Command(Command::Advance),
                Status::Waiting(Decision::MainMenu) => {
                    chose_continue = true;
                    Input::Command(Command::ChooseOption(crate::modes::main_menu::CONTINUE))
                }
                Status::Waiting(Decision::SlotSelector) => {
                    let Some(Mode::SlotSelector(selector)) = game.modes().last() else { unreachable!() };
                    assert_eq!(selector.current(), AUTOSAVE, "the cursor on the newest slot");
                    Input::Buttons(Joypad::A)
                }
                Status::Waiting(Decision::CursorMenu) => Input::Buttons(Joypad::A),
                _ => Input::None,
            };
            // A press is new only after a frame without it.
            let input = if pressed && matches!(input, Input::Buttons(_)) { Input::None } else { input };
            pressed = matches!(input, Input::Buttons(_));
            if let Some(request) = game.frame(input).slot {
                let saved_at = SavedAt { unix_seconds: frame as i64, utc_offset_minutes: 0 };
                store.answer(&mut game, request, saved_at).unwrap();
            }
        }
        assert!(chose_continue && matches!(game.modes(), [Mode::Overworld(_)]), "{:?}", game.status());
        let landing = fly_warp(Map::PalletTown).unwrap();
        let location = &game.world().location;
        assert_eq!((location.map, location.x, location.y), (Map::PalletTown, landing.x, landing.y));
        assert_eq!(game.world().hall_of_fame_teams, 1);
        assert_eq!(game.slots().len(), crate::save_slots::SLOTS, "the host handed the slots back after the load");
    }
}
