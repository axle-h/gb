//! Power-on to the overworld, the cartridge booted cold beside `Game::power_on`, and `HallOfFamePC`
//! to THE END beside `Movie::hall_of_fame`.
//!
//! The splash, the intro and the title are compared pixel for pixel on every frame. The cartridge
//! is late wherever the recreation leaves loading out, so the frames are walked with an offset that
//! may only grow, a few frames at a time; a frame that matches at no offset is the cartridge part
//! way through a transfer the recreation makes whole, and only a short run of those is allowed.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::rng::GameRng;
use pokered::systems::hall_of_fame::{HallOfFameMon, HOF_TEAM_CAPACITY};
use pokered::{Game, Input, Pacing};
use gb::ram::ROM;
use poke_core::symbols::pokered_local_labels as local;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::status_screen::{cartridge_until_polling, ours, screen};
use super::{assert_late, breakpoint, joypad, ARROW, BOX, CURSOR};

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
    let mut game = Game::power_on(None, GameRng::tape(recording.tape.clone()), Pacing::Faithful);
    let mut film = Film::default();
    for frame in &recording.frames {
        film.cartridge.push(hash(frame));
        film.cartridge_pictures.push(pack(frame));
    }
    for _ in 0..3000 {
        game.frame(Input::None);
        film.recreation_frame(&game);
    }
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
    /// A picture decompressed, the naming screen loaded or the title's cry and copies: all loading,
    /// and too uneven to name, so held only to never early and at most this late.
    Untimed(u32),
}

/// A text opened by the press: its box and its `▼`, less the letter the held press hurries.
const TEXT: Late = Late::Loading(BOX + ARROW - 2);
/// A text some frames after the press, which no longer hurries it.
const TEXT_UNHURRIED: Late = Late::Loading(BOX + ARROW);
const PICTURE: Late = Late::Untimed(70);
const MENU: Late = Late::Loading(CURSOR);

/// Every poll of a new game, what is pressed at it, and how late the cartridge reaches it: START at
/// the title, NEW GAME, "AB" typed for the player and Oak's second name for the rival.
fn new_game_script() -> Vec<(Decision, Joypad, Late)> {
    use Decision::*;
    let a = Joypad::A;
    vec![
        (TitleScreen, Joypad::START, Late::Loading(0)),
        (MainMenu, a, Late::Untimed(50)),
        (Text, a, PICTURE), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
        (Text, a, PICTURE), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
        (Text, a, TEXT), (Text, a, TEXT),
        (Text, a, PICTURE),
        (IntroNameMenu, a, MENU),
        (NamingScreen, a, Late::Untimed(50)), (NamingScreen, Joypad::RIGHT, Late::Loading(0)),
        (NamingScreen, a, Late::Loading(0)), (NamingScreen, Joypad::START, Late::Loading(0)),
        (Text, a, PICTURE),
        (Text, a, PICTURE), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
        (IntroNameMenu, Joypad::DOWN, MENU), (IntroNameMenu, Joypad::DOWN, MENU), (IntroNameMenu, a, MENU),
        (Text, a, TEXT_UNHURRIED), (Text, a, TEXT),
        (Text, a, PICTURE), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT), (Text, a, TEXT),
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

    /// Both films without their first frames, up to the first all-white one on each side.
    fn from_white(mut self) -> Self {
        let white = |pictures: &[Vec<u8>]| pictures.iter().position(|p| p.iter().all(|&b| b == 0)).expect("a white frame");
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

#[test]
fn a_new_game_from_the_title_to_reds_room_matches_the_cartridge() {
    let tape = player_id_tape();
    let mut gb = boot();
    let mut game = Game::power_on(None, GameRng::tape(tape), Pacing::Faithful);
    cartridge_to(&mut gb, local::DisplayTitleScreen::awaitUserInterruptionLoop);
    let script = new_game_script();
    let mut film = Film::default();
    // The intro is the other test's: both are filmed from the title screen's first wait.
    recreation_until_asked(&mut game, &mut Film::default());
    let mut recreation = 0;
    for (poll, (decision, button, late)) in script.into_iter().enumerate() {
        let cartridge = cartridge_until_polling_filmed(&mut gb, &mut film);
        assert_eq!(game.status(), Status::Waiting(decision.clone()), "poll {poll}");
        assert_eq!(screen(&gb), ours(&game), "the tile map at poll {poll}, {decision:?}");
        let what = format!("poll {poll}, {decision:?}");
        match late {
            Late::Loading(loading) if poll > 0 => assert_late(cartridge, recreation, loading, &what),
            Late::Untimed(most) => assert!(cartridge >= recreation && cartridge <= recreation + most,
                "{what}: the cartridge took {cartridge} frames and the recreation {recreation}"),
            Late::Loading(_) => {}
        }
        gb.hold_buttons(joypad(button));
        super::to_vblank(&mut gb);
        film.cartridge_frame(&gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::Buttons(button));
        film.recreation_frame(&game);
        recreation = recreation_until_asked(&mut game, &mut film);
    }
    assert!(matches!(game.modes(), [Mode::Overworld(_)]), "the new game ends in the overworld");
    let (vblank, enter) = (breakpoint(sym::VBlank), breakpoint(sym::EnterMap));
    let mut cartridge = 0;
    while gb.run_until(&[vblank, enter], MachineCycles::PER_FRAME * 600).0 == Stop::Breakpoint(vblank) {
        film.cartridge_frame(&gb);
        cartridge += 1;
    }
    // Both shrinking pictures are decompressed, and the text box tiles copied with the LCD on.
    assert!(cartridge >= recreation && cartridge <= recreation + 90,
        "into the overworld: the cartridge took {cartridge} frames and the recreation {recreation}");
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
