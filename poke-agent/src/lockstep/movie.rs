//! Power-on to the overworld, the cartridge booted cold beside `Game::power_on` with no save: a new
//! game. `HallOfFamePC` to THE END beside `Movie::hall_of_fame`. And an in-game trade's
//! `InternalClockTradeAnim` beside `Movie::trade`.
//!
//! The splash, the intro and the title are compared pixel for pixel on every frame. The cartridge
//! is late wherever the recreation leaves loading out, so the frames are walked with an offset that
//! may only grow, a few frames at a time; a frame that matches at no offset is the cartridge part
//! way through a transfer the recreation makes whole, and only a short run of those is allowed.

use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::movie::TITLE_VERSION_ROW;
use pokered::rng::GameRng;
use pokered::systems::hall_of_fame::{HallOfFameMon, HOF_TEAM_CAPACITY};
use pokered::{Game, Input, Pacing};
use gb::ram::ROM;
use crate::pokemon::symbols::pokered_local_labels as local;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::status_screen::{cartridge_until_polling, ours, screen};
use super::main_menu::menu_loading;
use super::{assert_late, breakpoint, joypad, ARROW, BOX, CURSOR, DELAY3};


/// `LoadTextBoxTilePatterns` through `CopyVideoData`, eight tiles a frame and a frame to finish.
fn text_box_tiles() -> u32 {
    let tiles = |start: crate::pokemon::symbols::DmgPointer, end: crate::pokemon::symbols::DmgPointer| {
        (end.address - start.address) as u32 / 16
    };
    tiles(sym::TextBoxGraphics, sym::TextBoxGraphicsEnd) / 8 + 1
}

/// The cartridge run alone first: its LCD at every `VBlank`, and every `Random` byte that was not
/// `VBlank`'s own.
struct Recording {
    frames: Vec<Vec<u8>>,
    tape: Vec<u8>,
}

fn lcd_shades(gb: &GameBoy) -> Vec<u8> {
    gb.core().mmu().ppu().screenshot().pixels().map(|p| match p.0[0] {
        0xFF => 0,
        0xAA => 1,
        0x55 => 2,
        _ => 3,
    }).collect()
}

fn record(gb: &mut GameBoy, frames: usize) -> Recording {
    let vblank = breakpoint(sym::VBlank);
    let random = breakpoint(sym::Random);
    let mut recording = Recording { frames: Vec::new(), tape: Vec::new() };
    while recording.frames.len() < frames {
        match gb.run_until(&[vblank, random], MachineCycles::PER_FRAME * 120).0 {
            Stop::Breakpoint(hit) if hit == vblank => {
                recording.frames.push(lcd_shades(gb));
            }
            Stop::Breakpoint(_) => {
                let caller = gb.return_address();
                let (stop, _) = gb.run_to_return(MachineCycles::PER_FRAME * 10);
                assert!(matches!(stop, Stop::Returned { .. }), "{stop:?}");
                if !(sym::VBlank.address..sym::VBlank.address + 0x80).contains(&caller) {
                    recording.tape.push(gb.core().registers().a);
                }
            }
            stop => panic!("{stop:?}"),
        }
    }
    recording
}

/// Four shades a byte, which is what a film of thousands of frames can afford to keep.
fn pack(shades: &[u8]) -> Vec<u8> {
    shades.chunks(4).map(|c| c.iter().enumerate().fold(0, |byte, (i, &s)| byte | s << (i * 2))).collect()
}

fn unpack(packed: &[u8]) -> Vec<u8> {
    packed.iter().flat_map(|&byte| (0..4).map(move |i| byte >> (i * 2) & 3)).collect()
}

fn is_white(picture: &[u8]) -> bool {
    picture.iter().all(|&b| b == 0)
}

fn hash(shades: &[u8]) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    shades.hash(&mut hasher);
    hasher.finish()
}

/// How a cartridge frame compares with the recreation frame it is lined up with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Match {
    Same,
    /// The picture the recreation showed a frame or two before: `AutoBgMapTransfer` carries a third
    /// of the tile map a frame, so a row the recreation draws at once can reach the LCD two frames on.
    Stale,
    Different,
}

/// How the recreation's frames line up with the cartridge's: the cartridge frame each one is shown
/// on, by the cheapest path whose lateness only grows, but for a frame at a time where a lag frame
/// inside a hold shows the held picture a frame short.
struct Alignment {
    frames: Vec<(usize, Match)>,
}

impl Alignment {
    fn new(cartridge: &[u64], ours: &[u64], start: usize) -> Self {
        const MAX_LATE: usize = 1500;
        const JUMP: usize = 16;
        let (miss, grow, shrink, stale) = (100u32, 1u32, 30u32, 2u32);
        let late = |r: usize, k: usize| match cartridge.get(r + k) {
            Some(&c) if c == ours[r] => Match::Same,
            Some(&c) if (r.saturating_sub(2)..r).any(|earlier| ours[earlier] == c) => Match::Stale,
            _ => Match::Different,
        };
        let price = |m: Match| match m {
            Match::Same => 0,
            Match::Stale => stale,
            Match::Different => miss,
        };
        let width = MAX_LATE + 1;
        let mut cost = vec![u32::MAX; width];
        let mut from = vec![0u16; ours.len() * width];
        cost[start] = price(late(0, start));
        for r in 1..ours.len() {
            let mut next = vec![u32::MAX; width];
            for k in 0..width {
                let mut best = (u32::MAX, 0usize);
                for previous in k.saturating_sub(JUMP)..=(k + 1).min(MAX_LATE) {
                    if cost[previous] == u32::MAX {
                        continue;
                    }
                    let step = match previous.cmp(&k) {
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Less => grow * (k - previous) as u32,
                        std::cmp::Ordering::Greater => shrink,
                    };
                    best = best.min((cost[previous] + step, previous));
                }
                if best.0 != u32::MAX {
                    next[k] = best.0 + price(late(r, k));
                    from[r * width + k] = best.1 as u16;
                }
            }
            cost = next;
        }
        let mut k = (0..width).min_by_key(|&k| cost[k]).expect("a path");
        let mut frames = vec![(0, Match::Different); ours.len()];
        for r in (0..ours.len()).rev() {
            frames[r] = (r + k, late(r, k));
            k = from[r * width + k] as usize;
        }
        Self { frames }
    }
}

impl Film {
    /// Holds the recreation's pictures to the cartridge's: every one matched in order, but for runs
    /// of `transfer` frames the cartridge spends drawing what the recreation draws at once.
    fn check(&self, start: usize, transfer: usize, what: &str) -> Alignment {
        let alignment = Alignment::new(&self.cartridge, &self.recreation, start);
        let mut run = 0;
        let mut late = start;
        let stale = alignment.frames.iter().filter(|(_, m)| *m == Match::Stale).count();
        println!("{what}: {stale} frames of {} a transfer behind", alignment.frames.len());
        for (r, &(c, matched)) in alignment.frames.iter().enumerate() {
            let same = matched != Match::Different;
            if c - r != late {
                println!("{what}: from recreation frame {r} the cartridge is {} frames late", c - r);
                late = c - r;
            }
            run = if same { 0 } else { run + 1 };
            if !same {
                let (ours, theirs) = (unpack(&self.recreation_pictures[r]), unpack(&self.cartridge_pictures[c]));
                let wrong: Vec<(usize, usize)> = (0..ours.len()).filter(|&i| theirs[i] != ours[i]).map(|i| (i % 160, i / 160)).collect();
                println!("{what}: recreation frame {r} is not cartridge frame {c}: {} pixels, first {:?}", wrong.len(), &wrong[..wrong.len().min(6)]);
                if run > transfer {
                    if let Ok(dir) = std::env::var("GB_MOVIE_DUMP") {
                        let strip = |pictures: &[Vec<u8>], from: usize| {
                            let mut pgm = "P2 160 1440 3\n".to_string().into_bytes();
                            for frame in from..from + 10 {
                                for shade in unpack(&pictures[frame.min(pictures.len() - 1)]) {
                                    pgm.extend(format!("{} ", 3 - shade).bytes());
                                }
                            }
                            pgm
                        };
                        std::fs::write(format!("{dir}/ours.pgm"), strip(&self.recreation_pictures, r - transfer)).unwrap();
                        std::fs::write(format!("{dir}/theirs.pgm"), strip(&self.cartridge_pictures, c - transfer)).unwrap();
                    }
                }
                assert!(run <= transfer, "{what}: {run} frames running match nothing, the last recreation frame {r}");
            }
        }
        alignment
    }
}

#[test]
fn the_splash_the_intro_and_the_title_match_the_lcd_frame_for_frame() {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
    let recording = record(&mut gb, 3600);
    let mut game = Game::power_on(GameRng::tape(recording.tape.clone()), Pacing::Faithful);
    let mut film = Film::default();
    for frame in &recording.frames {
        film.cartridge.push(hash(frame));
        film.cartridge_pictures.push(pack(frame));
    }
    for _ in 0..3000 {
        game.frame(Input::None);
        film.recreation_frame(&game);
    }
    film.without_version(|pictures| pictures.iter().rposition(|p| is_white(p)).expect("the intro's fade to white") + 1..pictures.len());
    film.check(60, 3, "power-on");
}

/// Runs the cartridge to `label`, however long that takes.
fn cartridge_to(gb: &mut GameBoy, label: crate::pokemon::symbols::DmgPointer) {
    let at = breakpoint(label);
    let (stop, _) = gb.run_until(&[at], MachineCycles::PER_FRAME * 20_000);
    assert_eq!(stop, Stop::Breakpoint(at), "the cartridge never reached {label}");
}

/// How late the cartridge may reach a poll, from the press before it.
#[derive(Debug, Clone, Copy)]
enum Late {
    /// By exactly this much loading, give or take `assert_late`'s allowance.
    Loading(u32),
}

/// A text opened by the press: its box and its `▼`, less the letter the held press hurries.
const TEXT: Late = Late::Loading(BOX + ARROW - 2);
/// A text some frames after the press, which no longer hurries it.
const TEXT_UNHURRIED: u32 = BOX + ARROW;
const MENU: Late = Late::Loading(CURSOR);
/// `.finishedWaiting` after the cry: `GBPalWhiteOutWithDelay3`, `ClearScreen`'s `Delay3`, the two
/// `TitleScreenCopyTileMapToVRAM`'s and the one before `LoadGBPal`.
const TITLE_LEAVING: u32 = 5 * DELAY3;

/// A picture in Oak's speech and the text over it: `ClearScreen`'s `Delay3`, the picture
/// decompressed and copied, measured, and the text's box and `▼`.
const fn picture(decompressed: u32) -> Late {
    Late::Loading(DELAY3 + decompressed + TEXT_UNHURRIED)
}
/// Oak's, after `OakSpeech`'s own `ClearScreen`, `LoadTextBoxTilePatterns` and the new game's WRAM.
const OAK: Late = picture(48);
const NIDORINO: Late = picture(37);
const RED_PICTURE: u32 = 39;
const RED: Late = picture(RED_PICTURE);
const RIVAL: Late = picture(40);
/// `DisplayNamingScreen` to its first poll, measured: its white-out and `ClearScreen`, and the HP
/// bar, `ED` and party icon tiles copied.
const NAMING_SCREEN: Late = Late::Loading(44);
/// `.submitNickname`'s `GBPalWhiteOutWithDelay3`, `ClearScreen` and `LoadTextBoxTilePatterns`, then
/// `ChoosePlayerName`'s `ClearScreen`, `Delay3` and Red's picture again.
fn red_named() -> Late {
    Late::Loading(2 * DELAY3 + text_box_tiles() + DELAY3 + DELAY3 + RED_PICTURE + TEXT_UNHURRIED)
}
/// Into the overworld, measured: both shrinking pictures decompressed, and the text box tiles
/// copied with the LCD on.
const INTO_THE_MAP: u32 = 79;

/// Every poll of a new game, what is pressed at it, and how late the cartridge reaches it: START at
/// the title, NEW GAME, "AB" typed for the player and Oak's second name for the rival.
fn new_game_script() -> Vec<(Decision, Joypad, Late)> {
    use Decision::*;
    let a = Joypad::A;
    let none = Late::Loading(0);
    vec![
        (TitleScreen, Joypad::START, none),
        (MainMenu, a, Late::Loading(TITLE_LEAVING + menu_loading())),
        (Text, a, OAK), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
        (Text, a, NIDORINO), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
        (Text, a, TEXT), (Text, a, TEXT),
        (Text, a, RED),
        (IntroNameMenu, a, MENU),
        (NamingScreen, a, NAMING_SCREEN), (NamingScreen, Joypad::RIGHT, none),
        (NamingScreen, a, none), (NamingScreen, Joypad::START, none),
        (Text, a, red_named()),
        (Text, a, RIVAL), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
        (IntroNameMenu, Joypad::DOWN, MENU), (IntroNameMenu, Joypad::DOWN, MENU), (IntroNameMenu, a, MENU),
        (Text, a, Late::Loading(TEXT_UNHURRIED)), (Text, a, TEXT),
        (Text, a, RED), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
    ]
}

fn boot() -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
    gb
}

/// The two bytes `InitPlayerData2` makes the player's ID from, by a run ahead: `hRandomSub` after
/// its first `Random` and `hRandomAdd` after its second. The same presses at the same polls replay
/// the cartridge exactly.
fn player_id_tape() -> Vec<u8> {
    let mut gb = boot();
    cartridge_to(&mut gb, local::DisplayTitleScreen::awaitUserInterruptionLoop);
    cartridge_until_polling(&mut gb);
    press_cartridge(&mut gb, Joypad::START);
    cartridge_until_polling(&mut gb);
    press_cartridge(&mut gb, Joypad::A);
    cartridge_to(&mut gb, sym::InitPlayerData2);
    let random = breakpoint(sym::Random);
    let mut tape = Vec::new();
    for read in [sym::hRandomSub, sym::hRandomAdd] {
        assert_eq!(gb.run_until(&[random], MachineCycles::PER_FRAME).0, Stop::Breakpoint(random));
        gb.run_to_return(MachineCycles::PER_FRAME);
        tape.push(gb.core().mmu().read(read.address));
    }
    tape
}

fn press_cartridge(gb: &mut GameBoy, button: Joypad) {
    gb.hold_buttons(joypad(button));
    super::to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
}

/// Both sides' pictures, every frame of a run, for [`Alignment`] once the run is over.
#[derive(Default)]
struct Film {
    cartridge: Vec<u64>,
    recreation: Vec<u64>,
    /// The pictures themselves, kept where a mismatch wants reporting.
    cartridge_pictures: Vec<Vec<u8>>,
    recreation_pictures: Vec<Vec<u8>>,
}

impl Film {
    fn cartridge_frame(&mut self, gb: &GameBoy) {
        let shades = lcd_shades(gb);
        self.cartridge.push(hash(&shades));
        self.cartridge_pictures.push(pack(&shades));
    }

    fn recreation_frame(&mut self, game: &Game) {
        let shades = game.screen().frame().shades;
        self.recreation.push(hash(&shades));
        self.recreation_pictures.push(pack(&shades));
    }

    /// The recreation's title names its own version: that row blanked on both sides, in the frames
    /// `title` picks out of each side's pictures as the title's.
    fn without_version(&mut self, title: impl Fn(&[Vec<u8>]) -> std::ops::Range<usize>) {
        let lines = TITLE_VERSION_ROW * 8 * 160..(TITLE_VERSION_ROW + 1) * 8 * 160;
        for (pictures, hashes) in [(&mut self.cartridge_pictures, &mut self.cartridge), (&mut self.recreation_pictures, &mut self.recreation)] {
            for frame in title(pictures) {
                let mut shades = unpack(&pictures[frame]);
                shades[lines.clone()].fill(0);
                hashes[frame] = hash(&shades);
                pictures[frame] = pack(&shades);
            }
        }
    }

    /// Both films without their first frames, up to the first all-white one on each side.
    fn from_white(mut self) -> Self {
        let white = |pictures: &[Vec<u8>]| pictures.iter().position(|p| is_white(p)).expect("a white frame");
        let (c, r) = (white(&self.cartridge_pictures), white(&self.recreation_pictures));
        self.cartridge.drain(..c);
        self.cartridge_pictures.drain(..c);
        self.recreation.drain(..r);
        self.recreation_pictures.drain(..r);
        self
    }
}

/// `cartridge_until_polling`, filming every frame on the way.
fn cartridge_until_polling_filmed(gb: &mut GameBoy, film: &mut Film) -> u32 {
    let poll = breakpoint(sym::JoypadLowSensitivity);
    let vblank = breakpoint(sym::VBlank);
    let mut frames = 0;
    loop {
        match gb.run_until(&[poll, vblank], MachineCycles::PER_FRAME * 600).0 {
            Stop::Breakpoint(hit) if hit == poll => {
                super::to_vblank(gb);
                film.cartridge_frame(gb);
                return frames + 1;
            }
            Stop::Breakpoint(_) => {
                film.cartridge_frame(gb);
                frames += 1;
            }
            stop => panic!("the cartridge never polled: {stop:?}"),
        }
        assert!(frames < 2000, "the cartridge never polled");
    }
}

fn recreation_until_asked(game: &mut Game, film: &mut Film) -> u32 {
    for frames in 1..5000 {
        game.frame(Input::None);
        film.recreation_frame(game);
        if matches!(game.status(), Status::Waiting(_)) || matches!(game.modes(), [Mode::Overworld(_)]) {
            return frames;
        }
    }
    panic!("the recreation never asked anything");
}

fn assert_in_time(cartridge: u32, recreation: u32, late: Late, what: &str) {
    match late {
        Late::Loading(loading) => assert_late(cartridge, recreation, loading, what),
    }
}

/// Both sides through `script` from where both stand, filming every frame: at each poll the same
/// decision and tile map, and the cartridge late by the loading named. `since` is the recreation's
/// frames since the press before the first poll, and `None` where both were started there; the
/// frames since the last press are returned.
fn play(gb: &mut GameBoy, game: &mut Game, film: &mut Film, since: Option<u32>, script: Vec<(Decision, Joypad, Late)>) -> u32 {
    let mut recreation = since;
    for (poll, (decision, button, late)) in script.into_iter().enumerate() {
        let cartridge = cartridge_until_polling_filmed(gb, film);
        assert_eq!(game.status(), Status::Waiting(decision.clone()), "poll {poll}");
        // Below these the recreation's option screen has the ruleset, which the cartridge has not.
        let rows = if decision == Decision::Options { pokered::modes::option_menu::CARTRIDGE_ROWS } else { 18 };
        // And the title names its own version.
        let compared = |mut map: Vec<Vec<u8>>| {
            map.truncate(rows);
            if decision == Decision::TitleScreen {
                map.remove(TITLE_VERSION_ROW);
            }
            map
        };
        assert_eq!(compared(screen(gb)), compared(ours(game)), "the tile map at poll {poll}, {decision:?}");
        if let Some(recreation) = recreation {
            assert_in_time(cartridge, recreation, late, &format!("poll {poll}, {decision:?}"));
        }
        gb.hold_buttons(joypad(button));
        super::to_vblank(gb);
        film.cartridge_frame(gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::Buttons(button));
        film.recreation_frame(game);
        recreation = Some(recreation_until_asked(game, film));
    }
    recreation.expect("a script of at least one poll")
}

#[test]
fn a_new_game_from_the_title_to_reds_room_matches_the_cartridge() {
    let tape = player_id_tape();
    let mut gb = boot();
    let mut game = Game::power_on(GameRng::tape(tape), Pacing::Faithful);
    cartridge_to(&mut gb, local::DisplayTitleScreen::awaitUserInterruptionLoop);
    let mut film = Film::default();
    // The intro is the other test's: both are filmed from the title screen's first wait.
    recreation_until_asked(&mut game, &mut Film::default());
    let recreation = play(&mut gb, &mut game, &mut film, None, new_game_script());
    assert!(matches!(game.modes(), [Mode::Overworld(_)]), "the new game ends in the overworld");
    let (vblank, enter) = (breakpoint(sym::VBlank), breakpoint(sym::EnterMap));
    let mut cartridge = 0;
    while gb.run_until(&[vblank, enter], MachineCycles::PER_FRAME * 600).0 == Stop::Breakpoint(vblank) {
        film.cartridge_frame(&gb);
        cartridge += 1;
    }
    assert_late(cartridge, recreation, INTO_THE_MAP, "into the overworld");
    film.without_version(|pictures| 0..pictures.iter().position(|p| is_white(p)).expect("the title's white-out"));
    film.check(0, 3, "a new game");

    let mmu = gb.core().mmu();
    let world = game.world();
    let name = |at: u16| (0..11).map(|i| mmu.read(at + i)).take_while(|&b| b != 0x50).collect::<Vec<u8>>();
    assert_eq!(world.player_name, name(sym::wPlayerName.address), "the player's name");
    assert_eq!(world.rival_name, name(sym::wRivalName.address), "the rival's name");
    assert_eq!(world.player_id, mmu.read_pointer_u16_be(&sym::wPlayerID), "the player's ID");
    assert_eq!(world.money.to_vec(), mmu.read_pointer_vec(&sym::wPlayerMoney, 3), "money");
    assert_eq!(mmu.read_pointer(&sym::wNumBagItems), 0, "an empty bag");
    let pc = mmu.read_pointer_vec(&sym::wNumBoxItems, 4);
    assert_eq!(pc, [1, ItemId::Potion as u8, 1, 0xFF], "the Potion in the PC");
    assert_eq!(world.pc_items.items.len(), 1);
    assert_eq!(world.location.map as u8, mmu.read_pointer(&sym::wCurMap), "the map");
    assert_eq!((world.location.x, world.location.y), (mmu.read_pointer(&sym::wXCoord), mmu.read_pointer(&sym::wYCoord)));
    assert_eq!(world.location.last_map as u8, mmu.read_pointer(&sym::wLastMap), "the last map");
    let expected = super::status_screen::the_world(&gb);
    assert_eq!(world.options, expected.options, "the options");
    assert_eq!(world.events, expected.events, "every event flag");
    assert_eq!(mmu.read_pointer(&sym::wStatusFlags6) & 1 << 0 != 0, world.play_time.counting, "the game timer counting");
    assert_eq!(mmu.read_pointer(&sym::wObtainedBadges), world.badges);
    assert_eq!(mmu.read_pointer(&sym::wPartyCount), 0);
    assert_eq!(mmu.read_pointer(&sym::wNumHoFTeams), world.hall_of_fame_teams);
}


/// `HOF_MON`, and `sHallOfFame`'s offset into the cartridge's first SRAM bank.
const HOF_MON: usize = 16;
const HOF_TEAM: usize = 6 * HOF_MON;
const S_HALL_OF_FAME: usize = 0x598;
/// Past this many frames on either side, the movie has stopped for something nobody answers.
const MOVIE_FRAMES: usize = 10_000;

/// `sHallOfFame`'s team `index`, as far as its `$FF`.
fn sram_team(sram: &[u8], index: usize) -> Vec<HallOfFameMon> {
    let team = &sram[S_HALL_OF_FAME + index * HOF_TEAM..S_HALL_OF_FAME + (index + 1) * HOF_TEAM];
    team.chunks_exact(HOF_MON).take_while(|entry| entry[0] != 0xFF).map(|entry| HallOfFameMon {
        species: PokemonSpecies::from_repr(entry[0]).expect("a species"),
        level: entry[1],
        nick: entry[2..13].iter().copied().take_while(|&b| b != 0x50).collect(),
    }).collect()
}

/// The Celadon fixture's call of the start menu turned into a call of `HallOfFamePC`, with the
/// fixture's party, Pokédex, clock, money and earlier teams to show, to its return after THE END.
#[test]
fn the_hall_of_fame_and_the_credits_match_the_cartridge_frame_for_frame() {
    use pokered::mode::Mode;
    use pokered::modes::movie::Movie;
    use pokered::systems::play_time::PlayTime;
    let mut gb = super::open_the_start_menu();
    gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
    let returned = gb.return_address();
    assert!(returned < 0x4000, "the start menu is called from the home bank");
    let back = gb::game_boy::Breakpoint::new(0, returned);
    super::learn_move::hijack(&mut gb, sym::HallOfFamePC);

    let mmu = gb.core().mmu();
    let mut world = super::status_screen::the_world(&gb);
    world.pokedex.seen.copy_from_slice(&mmu.read_pointer_vec(&sym::wPokedexSeen, 19));
    world.pokedex.owned.copy_from_slice(&mmu.read_pointer_vec(&sym::wPokedexOwned, 19));
    world.money.copy_from_slice(&mmu.read_pointer_vec(&sym::wPlayerMoney, 3));
    world.hall_of_fame_teams = mmu.read_pointer(&sym::wNumHoFTeams);
    let sram = gb.dump_sram();
    world.hall_of_fame = (0..(world.hall_of_fame_teams as usize).min(HOF_TEAM_CAPACITY)).map(|i| sram_team(&sram, i)).collect();
    let time = mmu.read_pointer_vec(&sym::wPlayTimeHours, 5);
    world.play_time = PlayTime { hours: time[0], maxed: time[1] != 0, minutes: time[2], seconds: time[3], frames: time[4], counting: mmu.read_pointer(&sym::wStatusFlags6) & 1 != 0 };
    let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::Movie(Movie::hall_of_fame()));
    // The cartridge's breakpoint is `HallOfFamePC` returning; the recreation goes on from there to
    // the save and the restart its script ends with.
    let hall_of_fame_returned = |game: &Game| matches!(game.modes().last(), Some(Mode::Movie(movie)) if movie.hall_of_fame_returned());

    // The rating's text runs past its box with a `cont` a line, each waiting for A; both sides are
    // pressed once both have polled.
    let mut film = Film::default();
    let (vblank, poll) = (breakpoint(sym::VBlank), breakpoint(sym::JoypadLowSensitivity));
    let mut presses = 0;
    loop {
        let cartridge_polled = loop {
            match gb.run_until(&[vblank, poll, back], MachineCycles::PER_FRAME * 600).0 {
                Stop::Breakpoint(hit) if hit == vblank => film.cartridge_frame(&gb),
                Stop::Breakpoint(hit) if hit == poll => {
                    super::to_vblank(&mut gb);
                    film.cartridge_frame(&gb);
                    break true;
                }
                Stop::Breakpoint(_) => break false,
                stop => panic!("{stop:?}"),
            }
            assert!(film.cartridge.len() < MOVIE_FRAMES, "the cartridge never returned");
        };
        loop {
            game.frame(Input::None);
            film.recreation_frame(&game);
            if hall_of_fame_returned(&game) || game.status() == Status::Waiting(Decision::Text) {
                break;
            }
            assert!(film.recreation.len() < MOVIE_FRAMES, "the credits never ended");
        }
        assert_eq!(cartridge_polled, !hall_of_fame_returned(&game), "both wait for the player at press {presses}");
        if !cartridge_polled {
            break;
        }
        presses += 1;
        assert!(presses < 10, "the movie waits for the player only in its texts");
        gb.hold_buttons(joypad(Joypad::A));
        super::to_vblank(&mut gb);
        film.cartridge_frame(&gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::Buttons(Joypad::A));
        film.recreation_frame(&game);
    }
    assert_eq!(presses, 2, "the fixture's rating, `_DexRatingText_Own10To19`, has two `cont`s");
    println!("the cartridge {} frames and the recreation {}", film.cartridge.len(), film.recreation.len());
    let film = film.from_white();
    film.check(0, 3, "the Hall of Fame");

    let mmu = gb.core().mmu();
    let world = game.world();
    let teams = mmu.read_pointer(&sym::wNumHoFTeams);
    assert_eq!(teams, world.hall_of_fame_teams, "wNumHoFTeams");
    let sram = gb.dump_sram();
    let recorded: Vec<_> = (0..(teams as usize).min(HOF_TEAM_CAPACITY)).map(|i| sram_team(&sram, i)).collect();
    assert_eq!(world.hall_of_fame, recorded, "sHallOfFame");
    let party: Vec<_> = world.party.iter().map(|named| (named.mon.mon.species, named.mon.level, named.nick.clone())).collect();
    let last: Vec<_> = recorded.last().expect("a team recorded").iter().map(|mon| (mon.species, mon.level, mon.nick.clone())).collect();
    assert_eq!(last, party, "the team recorded is the party");
}

/// What the trade movie waits on that the recreation leaves out, besides `CopyVideoData` and the
/// pictures' decompression: `DisableLCD`'s frame, the `Delay3` of `ClearScreen`, of
/// `Trade_CopyTileMapToVRAM` and of `PrintText`'s box, `CopyScreenTileBufferToVRAM`'s three frames,
/// and the `Delay3` after the cable's last copy.
const TRADE_LOADING: [(crate::pokemon::symbols::DmgPointer, u32); 6] = [
    (sym::DisableLCD, 1), (sym::ClearScreen, DELAY3), (sym::Trade_CopyTileMapToVRAM, DELAY3), (sym::PrintText, BOX),
    (sym::CopyScreenTileBufferToVRAM, DELAY3), (local::Trade_AnimateBallEnteringLinkCable::ballSpriteReachedEdgeOfScreen, DELAY3),
];

const TRADES: &[u8] = include_bytes!("../pokemon/data/postgame-trades.bin");

/// The Underground Path girl's trade as `events.rs` makes it, a Nidoran♂ written into the lead slot:
/// the cartridge on `gb`, with `TRADES` loaded, walked in, talked to her and A pressed until
/// `InternalClockTradeAnim` begins.
fn cartridge_at_the_trade_movie(mut gb: GameBoy) -> GameBoy {
    use gb::ram::RAM;
    use poke_core::map::Map;
    gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
    let mmu = gb.core_mut().mmu_mut();
    let at = sym::wCompletedInGameTradeFlags.address + 1;
    mmu.write(at, mmu.read(at) & !(1 << 1));
    let nidoran = PokemonSpecies::NidoranMale as u8;
    mmu.write(sym::wPartySpecies.address, nidoran);
    mmu.write(sym::wPartyMon1.address, nidoran);
    mmu.write(sym::wPartyAndBillsPCSavedMenuItem.address, 0);
    let (vblank, movie) = (breakpoint(sym::VBlank), breakpoint(sym::InternalClockTradeAnim));
    for frame in 0..6000u32 {
        let mmu = gb.core().mmu();
        let (map, x, y) = (mmu.read_pointer(&sym::wCurMap), mmu.read_pointer(&sym::wXCoord), mmu.read_pointer(&sym::wYCoord));
        let button = if map != Map::UndergroundPathRoute5 as u8 || y > 4 {
            Joypad::UP
        } else if x > 2 {
            Joypad::LEFT
        } else if frame % 4 < 2 {
            // Up turns to face her, then A talks and answers everything after.
            if mmu.read_pointer(&sym::wSpritePlayerStateData1FacingDirection) != 4 { Joypad::UP } else { Joypad::A }
        } else {
            Joypad::empty()
        };
        gb.hold_buttons(joypad(button));
        match gb.run_until(&[vblank, movie], MachineCycles::PER_FRAME * 120).0 {
            Stop::Breakpoint(hit) if hit == movie => {
                gb.hold_buttons(JoypadButtonState::default());
                return gb;
            }
            Stop::Breakpoint(_) => {}
            stop => panic!("{stop:?}"),
        }
    }
    panic!("the trade movie never began");
}

/// What `InternalClockTradeAnim` reads, as `InGameTrade_PrepareTradeData` left it.
fn trade_data(gb: &GameBoy) -> pokered::modes::movie::trade::TradeData {
    use pokered::modes::movie::trade::{TradeData, TradedMon};
    let mmu = gb.core().mmu();
    let name = |at: u16| (0..11).map(|i| mmu.read(at + i)).take_while(|&b| b != 0x50).collect::<Vec<u8>>();
    let species = |at: u16| PokemonSpecies::from_repr(mmu.read(at)).expect("a species");
    let id = |at: u16| u16::from_be_bytes([mmu.read(at), mmu.read(at + 1)]);
    TradeData {
        player: TradedMon { species: species(sym::wTradedPlayerMonSpecies.address), ot: name(sym::wTradedPlayerMonOT.address),
                            ot_id: id(sym::wTradedPlayerMonOTID.address) },
        enemy: TradedMon { species: species(sym::wTradedEnemyMonSpecies.address), ot: name(sym::wTradedEnemyMonOT.address),
                           ot_id: id(sym::wTradedEnemyMonOTID.address) },
        enemy_trainer: name(sym::wLinkEnemyTrainerName.address),
        // `FadePal4`, as `LoadGBPal` finds it on a map that is not dark.
        palettes: [0b1110_0100, 0b1101_0000, 0b1110_0000],
    }
}

/// The recreation where the cartridge stands at `InternalClockTradeAnim`, with what the movie reads.
fn recreation_at_the_trade_movie(gb: &GameBoy) -> (Game, pokered::modes::movie::trade::TradeData) {
    let data = trade_data(gb);
    assert_eq!((data.player.species, data.enemy.species), (PokemonSpecies::NidoranMale, PokemonSpecies::NidoranFemale));
    let mut world = super::status_screen::the_world(gb);
    world.text.strings.insert(poke_core::text_script::TextBuffer::LinkEnemyTrainerName, data.enemy_trainer.clone());
    let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
    let mmu = gb.core().mmu();
    let screen = game.screen_mut();
    screen.tiles.load(0, &mmu.read_vram_slice(0x8000, 384 * 16).unwrap().to_vec());
    screen.effects.bgp = mmu.read(0xFF47);
    screen.effects.obp0 = mmu.read(0xFF48);
    screen.effects.obp1 = mmu.read(0xFF49);
    (game, data)
}

/// Writes recreation and cartridge frames side by side as PNGs into `dir`, every `every` frames of
/// the alignment.
fn dump_side_by_side(film: &Film, alignment: &Alignment, dir: &str, every: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let shade = |s: u8| [0xFF, 0xAA, 0x55, 0x00][s as usize];
    for (r, &(c, _)) in alignment.frames.iter().enumerate().step_by(every) {
        let (ours, theirs) = (unpack(&film.recreation_pictures[r]), unpack(&film.cartridge_pictures[c]));
        let picture = image::GrayImage::from_fn(2 * 160 + 4, 144, |x, y| {
            let x = x as usize;
            image::Luma([match x {
                0..160 => shade(ours[y as usize * 160 + x]),
                160..164 => 0x80,
                _ => shade(theirs[y as usize * 160 + x - 164]),
            }])
        });
        picture.save(format!("{dir}/trade-{r:04}-vs-{c:04}.png")).unwrap();
    }
}

/// `InternalClockTradeAnim` beside `Movie::trade`, from the predef's entry to `RemovePokemon` after
/// it, frame for frame, and each routine of the sequence timed against the loading the cartridge
/// entered in it. `GB_TRADE_FRAMES` names a directory for every `GB_TRADE_EVERY`th (40th) pair as
/// PNGs, the recreation on the left.
#[test]
fn the_trade_movie_matches_the_cartridge_frame_for_frame() {
    use pokered::modes::movie::Movie;
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(TRADES).unwrap();
    let mut gb = cartridge_at_the_trade_movie(gb);
    let (mut game, data) = recreation_at_the_trade_movie(&gb);

    let mut film = Film::default();
    let (vblank, done, routine) = (breakpoint(sym::VBlank), breakpoint(sym::RemovePokemon), breakpoint(local::TradeAnimCommon::r#loop));
    let (copy_video_data, picture) = (breakpoint(sym::CopyVideoData), breakpoint(sym::LoadFrontSpriteByMonIndex));
    let named: Vec<(Breakpoint, u32)> = TRADE_LOADING.iter().map(|&(at, frames)| (breakpoint(at), frames)).collect();
    let (jump, clear_screen) = (breakpoint(sym::TradeJumpPokeball), breakpoint(sym::ClearScreen));
    let mut points = vec![vblank, done, routine, copy_video_data, picture, jump];
    points.extend(named.iter().map(|&(at, _)| at));
    // Each routine's start on the cartridge, and the loading it entered.
    let (mut starts, mut loading) = (Vec::new(), Vec::new());
    let mut decompressing: Option<Breakpoint> = None;
    // `TradeJumpPokeball` ends in `ClearScreen`, whose `Delay3` passes while `MoveAnimation` waits
    // for the ball's last `SFX_SWAP` anyway.
    let mut jumping = false;
    loop {
        let hit = match gb.run_until(&points, MachineCycles::PER_FRAME * 600).0 {
            Stop::Breakpoint(hit) => hit,
            stop => panic!("{stop:?}"),
        };
        if hit == vblank {
            film.cartridge_frame(&gb);
            if decompressing.is_some() {
                *loading.last_mut().unwrap() += 1;
            }
        } else if hit == done {
            break;
        } else if hit == routine {
            starts.push(film.cartridge.len());
            loading.push(0);
        } else if hit == copy_video_data {
            // The picture's own copy is inside its span.
            if decompressing.is_none() {
                *loading.last_mut().unwrap() += gb.core().registers().c as u32 / 8 + 1;
            }
        } else if hit == picture {
            let back = gb.return_address();
            let back = Breakpoint::new(gb.core().mmu().rom_bank() as u8, back);
            decompressing = Some(back);
            points.push(back);
        } else if decompressing == Some(hit) {
            decompressing = None;
            points.pop();
        } else if hit == jump {
            jumping = true;
        } else if hit == clear_screen && jumping {
            jumping = false;
        } else {
            *loading.last_mut().unwrap() += named.iter().find(|&&(at, _)| at == hit).unwrap().1;
        }
        assert!(film.cartridge.len() < MOVIE_FRAMES, "the cartridge never finished the movie");
    }
    // The last start is `TradeAnimCommon` reading the sequence's end.
    starts.pop();
    loading.pop();
    starts.push(film.cartridge.len());

    let begun = |game: &Game| game.modes().iter().find_map(|mode| match mode {
        Mode::Movie(Movie::Trade(trade)) => Some(trade.routines_begun()),
        _ => None,
    });
    let mut recreation_starts = Vec::new();
    game.push(Mode::Movie(Movie::trade(data)));
    loop {
        film.recreation_frame(&game);
        let Some(now) = begun(&game) else { break };
        while recreation_starts.len() < now {
            recreation_starts.push(film.recreation.len() - 1);
        }
        game.frame(Input::None);
        assert!(film.recreation.len() < MOVIE_FRAMES, "the movie never ended");
    }
    let last = film.recreation.len() - 1;
    recreation_starts.resize(loading.len(), last);
    recreation_starts.push(last);
    println!("the cartridge {} frames and the recreation {}", film.cartridge.len(), film.recreation.len());
    if let Ok(dir) = std::env::var("GB_TRADE_FRAMES") {
        let every = std::env::var("GB_TRADE_EVERY").map_or(40, |n| n.parse().unwrap());
        dump_side_by_side(&film, &Alignment::new(&film.cartridge, &film.recreation, 0), &dir, every);
    }
    for (i, loading) in loading.iter().enumerate() {
        let (cartridge, recreation) = (starts[i + 1] - starts[i], recreation_starts[i + 1] - recreation_starts[i]);
        assert_late(cartridge as u32, recreation as u32, *loading, &format!("routine {i} of the sequence"));
    }
    film.check(0, 3, "the trade");
}

/// Both sides' pictures in colour, every frame, as `R, G, B` a pixel. Each distinct picture is kept
/// once, deflated, where a mismatch or a dump wants it.
#[derive(Default)]
struct ColourFilm {
    cartridge: Vec<u64>,
    recreation: Vec<u64>,
    pictures: std::collections::HashMap<u64, Vec<u8>>,
}

impl ColourFilm {
    fn keep(&mut self, rgb: Vec<u8>) -> u64 {
        use std::io::Write;
        let key = hash(&rgb);
        self.pictures.entry(key).or_insert_with(|| {
            let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&rgb).unwrap();
            encoder.finish().unwrap()
        });
        key
    }

    fn picture(&self, key: u64) -> Vec<u8> {
        use std::io::Read;
        let mut rgb = Vec::new();
        flate2::read::DeflateDecoder::new(&self.pictures[&key][..]).read_to_end(&mut rgb).unwrap();
        rgb
    }

    fn cartridge_frame(&mut self, gb: &GameBoy) {
        let key = self.keep(super::display_rgb(gb));
        self.cartridge.push(key);
    }

    fn recreation_frame(&mut self, game: &Game, colours: pokered::gfx::colour::ColourMode) {
        let rgb = colours.rgba(game.screen()).chunks_exact(4).flat_map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect();
        let key = self.keep(rgb);
        self.recreation.push(key);
    }

    /// `Film::check` in colour: every recreation picture matched in order, but for runs of
    /// `transfer` frames the cartridge spends getting there.
    fn check(&self, transfer: usize, what: &str) -> Alignment {
        let alignment = Alignment::new(&self.cartridge, &self.recreation, 0);
        let mut run = 0;
        for (r, &(c, matched)) in alignment.frames.iter().enumerate() {
            run = if matched == Match::Different { run + 1 } else { 0 };
            if matched == Match::Different {
                let (ours, theirs) = (self.picture(self.recreation[r]), self.picture(self.cartridge[c]));
                let wrong: Vec<(usize, usize)> = (0..ours.len() / 3).filter(|&i| ours[i * 3..i * 3 + 3] != theirs[i * 3..i * 3 + 3])
                    .map(|i| (i % 160, i / 160)).collect();
                println!("{what}: recreation frame {r} is not cartridge frame {c}: {} pixels, first {:?}", wrong.len(), &wrong[..wrong.len().min(6)]);
            }
            assert!(run <= transfer, "{what}: {run} frames running match nothing, the last recreation frame {r}");
        }
        alignment
    }

    /// Recreation and cartridge side by side as PNGs into `dir`, the frames `pick` names.
    fn dump(&self, alignment: &Alignment, dir: &str, prefix: &str, pick: impl Iterator<Item = usize>) {
        std::fs::create_dir_all(dir).unwrap();
        for r in pick {
            let c = alignment.frames[r].0;
            let (ours, theirs) = (self.picture(self.recreation[r]), self.picture(self.cartridge[c]));
            let picture = image::RgbImage::from_fn(2 * 160 + 4, 144, |x, y| {
                let (x, y) = (x as usize, y as usize);
                let at = |rgb: &[u8], x: usize| image::Rgb([rgb[(y * 160 + x) * 3], rgb[(y * 160 + x) * 3 + 1], rgb[(y * 160 + x) * 3 + 2]]);
                match x {
                    0..160 => at(&ours, x),
                    160..164 => image::Rgb([0x80; 3]),
                    _ => at(&theirs, x - 164),
                }
            });
            picture.save(format!("{dir}/{prefix}-{r:04}-vs-{c:04}.png")).unwrap();
        }
    }
}

/// The trade movie in `colours` beside the cartridge on `gb`, `TRADES` loaded, from the predef's
/// entry to `RemovePokemon`, frame for frame. `GB_TRADE_FRAMES` names a directory for every
/// `GB_TRADE_EVERY`th (40th) pair as PNGs, the recreation on the left.
fn the_trade_movie_in_colour(gb: GameBoy, colours: pokered::gfx::colour::ColourMode, what: &str) {
    use pokered::modes::movie::Movie;
    let mut gb = cartridge_at_the_trade_movie(gb);
    let (mut game, data) = recreation_at_the_trade_movie(&gb);
    if colours == pokered::gfx::colour::ColourMode::Sgb {
        // What `InGameTrade_RestoreScreen`'s `SET_PAL_DEFAULT` left after the party menu.
        use pokered::gfx::sgb::{OverworldPalette, PaletteCommand};
        let mmu = gb.core().mmu();
        let palette = OverworldPalette {
            map: poke_core::map::Map::from_repr(mmu.read_pointer(&sym::wCurMap)).unwrap(),
            tileset: poke_core::map_header::TileSetId::from_repr(mmu.read_pointer(&sym::wCurMapTileset)).unwrap(),
            last_map: poke_core::map::Map::from_repr(mmu.read_pointer(&sym::wLastMap)).unwrap(),
        };
        game.screen_mut().sgb.run(&PaletteCommand::Overworld(palette));
    }
    let mut film = ColourFilm::default();
    let (vblank, done) = (breakpoint(sym::VBlank), breakpoint(sym::RemovePokemon));
    loop {
        match gb.run_until(&[vblank, done], MachineCycles::PER_FRAME * 600).0 {
            Stop::Breakpoint(hit) if hit == vblank => film.cartridge_frame(&gb),
            Stop::Breakpoint(_) => break,
            stop => panic!("{stop:?}"),
        }
        assert!(film.cartridge.len() < MOVIE_FRAMES, "the cartridge never finished the movie");
    }
    game.push(Mode::Movie(Movie::trade(data)));
    loop {
        film.recreation_frame(&game, colours);
        if !game.modes().iter().any(|mode| matches!(mode, Mode::Movie(Movie::Trade(_)))) {
            break;
        }
        game.frame(Input::None);
        assert!(film.recreation.len() < MOVIE_FRAMES, "the movie never ended");
    }
    println!("{what}: the cartridge {} frames and the recreation {}", film.cartridge.len(), film.recreation.len());
    let alignment = film.check(3, what);
    if let Ok(dir) = std::env::var("GB_TRADE_FRAMES") {
        let every = std::env::var("GB_TRADE_EVERY").map_or(40, |n| n.parse().unwrap());
        film.dump(&alignment, &dir, what, (0..film.recreation.len()).step_by(every));
    }
}

/// On a Super Game Boy: the palette commands sent where the cartridge sends them, and each picture
/// painted as the SNES paints it.
#[test]
fn the_trade_movie_matches_the_cartridge_on_a_super_game_boy() {
    the_trade_movie_in_colour(super::on_sgb(TRADES), pokered::gfx::colour::ColourMode::Sgb, "sgb");
}

/// On a Game Boy Color, in compatibility mode: each pixel through the palette register it was
/// drawn with, into the boot ROM's colours.
#[test]
fn the_trade_movie_matches_the_cartridge_on_a_game_boy_color() {
    let mut gb = GameBoy::cgb(crate::pokemon::roms::POKERED);
    gb.load_state(TRADES).unwrap();
    the_trade_movie_in_colour(gb, pokered::gfx::colour::ColourMode::Gbc, "gbc");
}
