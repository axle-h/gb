//! Evolution, which no committed fixture stands in front of either. As for `LearnMove`, the Celadon
//! fixture's call of `DisplayStartMenu` becomes a call of `TryEvolvingMon`, with the first party mon
//! made a level 16 Charmander named after its species. Nothing in the evolution waits on a button
//! unless it is stopped, so each side runs on its own and every tile map either one shows is
//! compared, in order, with the frame it first appeared on.

use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::{RAM, ROM};
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::evolution::Evolution;
use pokered::rng::GameRng;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::pokered_symbols;
use super::learn_move::{hijack, with_the_tile_map};
use super::status_screen::{ours, screen, the_world};
use super::{breakpoint, joypad, open_the_start_menu, DELAY3};

const MON_MOVES: u16 = 8;
const MON_LEVEL: u16 = 33;
const NAME_LENGTH: usize = 11;
const TERMINATOR: u8 = 0x50;

type Screens = Vec<(u32, Vec<Vec<u8>>)>;

/// The picture's first tile, `vFrontPic`'s tile 0, which is its top right.
const PICTURE: (usize, usize) = (13, 2);

/// The Charmander, the hijacked call, and where that call returns to.
fn start() -> (GameBoy, Game, Breakpoint) {
    let mut gb = open_the_start_menu();
    let species = PokemonSpecies::Charmander as u8;
    let party_mon = pokered_symbols::wPartyMon1.address;
    let mmu = gb.core_mut().mmu_mut();
    mmu.write(pokered_symbols::wPartySpecies.address, species);
    mmu.write(party_mon, species);
    mmu.write(party_mon + MON_LEVEL, 16);
    for (i, known) in [PokemonMoveName::Scratch as u8, PokemonMoveName::Growl as u8, 0, 0].into_iter().enumerate() {
        mmu.write(party_mon + MON_MOVES + i as u16, known);
    }
    let mut nick = PokemonSpecies::Charmander.name();
    nick.resize(NAME_LENGTH, TERMINATOR);
    mmu.write_slice(pokered_symbols::wPartyMonNicks.address, &nick);
    mmu.write(pokered_symbols::wWhichPokemon.address, 0);
    mmu.write(pokered_symbols::wCurPartySpecies.address, species);
    mmu.write(pokered_symbols::wForceEvolution.address, 0);
    mmu.write(pokered_symbols::wLinkState.address, 0);
    mmu.write(pokered_symbols::wIsInBattle.address, 0);
    let sp = gb.core().registers().sp;
    let mmu = gb.core().mmu();
    let back = u16::from_le_bytes([mmu.read(sp), mmu.read(sp + 1)]);
    assert!(back < 0x4000, "DisplayStartMenu is called from the home bank");
    hijack(&mut gb, pokered_symbols::TryEvolvingMon);

    let mut game = Game::new(the_world(&gb), GameRng::seeded(0), Pacing::Faithful);
    with_the_tile_map(&gb, &mut game);
    (gb, game, Breakpoint::new(0, back))
}

/// Every tile map the cartridge shows at a VBlank until it reaches `stop`, with its frame.
fn cartridge_screens(gb: &mut GameBoy, stop: Breakpoint) -> Screens {
    let vblank = breakpoint(pokered_symbols::VBlank);
    let mut screens: Screens = vec![(0, screen(gb))];
    for frame in 1..3000 {
        match gb.run_until(&[vblank, stop], MachineCycles::PER_FRAME * 600).0 {
            Stop::Breakpoint(hit) if hit == stop => return screens,
            Stop::Breakpoint(_) => {}
            other => panic!("the cartridge stopped: {other:?}"),
        }
        let now = screen(gb);
        if screens.last().is_none_or(|(_, last)| *last != now) {
            screens.push((frame, now));
        }
    }
    panic!("the cartridge never reached {stop:?}");
}

/// The same for the recreation, pushing `mode` first when there is one so that the screen before it
/// is the first compared.
fn recreation_screens(game: &mut Game, mode: Option<Mode>, done: impl Fn(&Game) -> bool) -> Screens {
    let mut screens: Screens = vec![(0, ours(game))];
    if let Some(mode) = mode {
        game.push(mode);
        let now = ours(game);
        if screens[0].1 != now {
            screens.push((0, now));
        }
    }
    for frame in 1..3000 {
        game.frame(Input::None);
        let now = ours(game);
        if screens.last().is_none_or(|(_, last)| *last != now) {
            screens.push((frame, now));
        }
        if done(game) {
            return screens;
        }
    }
    panic!("the recreation never finished");
}

/// The same screens in the same order, each no more than `lag` frames later on the cartridge than
/// the one before it was, plus the loading the recreation leaves out between them. That loading is
/// a screen the cartridge holds for a `Delay3` and the recreation never draws, an empty box before
/// its first letter or a cleared screen, and `skipped` says how many there are. The exception is
/// the picture: `EvolveMon` decompresses both species' with the tile map already showing the first,
/// which costs the cartridge two stretches of lag frames the recreation does not have, so the
/// picture going up, the cleared screen held before it and the screen after it may be any amount
/// later.
fn compare(cartridge: &Screens, recreation: &Screens, lag: i64, skipped: usize, what: &str) {
    let picture = recreation.iter().position(|(_, rows)| rows[PICTURE.1][PICTURE.0] == 0);
    let (mut theirs, mut late, mut skips) = (0, 0, 0);
    for (i, (ours_at, ours)) in recreation.iter().enumerate() {
        let lagged = picture.is_some_and(|picture| i == picture || i == picture + 1);
        let mut loading = 0;
        while theirs < cartridge.len() && cartridge[theirs].1 != *ours {
            let held = cartridge.get(theirs + 1).map_or(0, |next| next.0 - cartridge[theirs].0) as i64;
            assert!(lagged || held <= DELAY3 as i64 + lag,
                "{what}: cartridge screen {theirs} at frame {} is held {held} frames and the recreation never shows it",
                cartridge[theirs].0);
            loading += DELAY3 as i64;
            skips += 1;
            theirs += 1;
        }
        assert!(theirs < cartridge.len(), "{what}: recreation screen {i} at frame {ours_at} never shows on the cartridge");
        let now = cartridge[theirs].0 as i64 - *ours_at as i64;
        let allowed = if lagged { late..=i64::MAX } else { late..=late + loading + lag };
        assert!(allowed.contains(&now), "{what}: screen {i} is {now} frames late on the cartridge, after {late} before it");
        late = now;
        theirs += 1;
    }
    assert_eq!(theirs, cartridge.len(), "{what}: the cartridge shows more once the recreation is done");
    assert_eq!(skips, skipped, "{what}: screens only the cartridge's loading shows");
    println!("{what}: {} screens, the cartridge {late} frames late by the end", recreation.len());
}

#[test]
fn an_evolution_draws_what_the_cartridge_draws_when_it_draws_it() {
    let (mut gb, mut game, back) = start();
    let cartridge = cartridge_screens(&mut gb, back);
    let evolution = Mode::Evolution(Evolution::try_evolving(0, PokemonSpecies::Charmander as u8, false));
    let recreation = recreation_screens(&mut game, Some(evolution), |game| game.modes().is_empty());
    compare(&cartridge, &recreation, 2, 3, "Charmander into Charmeleon");

    let party = &game.world().party[0];
    let mmu = gb.core().mmu();
    assert_eq!(mmu.read(pokered_symbols::wPartySpecies.address), party.mon.mon.species as u8);
    assert_eq!(party.nick, PokemonSpecies::Charmeleon.name(), "renamed");
    let theirs: Vec<u8> = (0..11).map(|i| mmu.read(pokered_symbols::wPartyMon1.address + MON_LEVEL + i)).collect();
    let ours: Vec<u8> = std::iter::once(party.mon.level)
        .chain(party.mon.stats.iter().flat_map(|stat| stat.to_be_bytes()))
        .collect();
    assert_eq!(theirs, ours, "the level and the stats");
}

#[test]
fn b_stops_an_evolution_where_the_cartridge_stops() {
    let (mut gb, mut game, _) = start();
    let watching = breakpoint(pokered_symbols::Evolution_CheckForCancel);
    let before = cartridge_screens(&mut gb, watching);
    let evolution = Mode::Evolution(Evolution::try_evolving(0, PokemonSpecies::Charmander as u8, false));
    let ours = recreation_screens(&mut game, Some(evolution), |game| {
        matches!(game.modes().last(), Some(Mode::Evolution(evolution)) if evolution.is_watching())
    });
    compare(&before, &ours, 2, 2, "up to the first watch");

    // The cartridge is stopped mid-frame, so B is held through the whole of the next VBlank, which
    // is where the pad is read, and the recreation is given the same two frames.
    gb.hold_buttons(joypad(Joypad::B));
    game.frame(Input::Buttons(Joypad::B));
    super::to_vblank(&mut gb);
    super::to_vblank(&mut gb);
    gb.hold_buttons(JoypadButtonState::default());
    game.frame(Input::None);
    let prompt = breakpoint(pokered_symbols::WaitForTextScrollButtonPress);
    let after = cartridge_screens(&mut gb, prompt);
    let ours = recreation_screens(&mut game, None, |game| game.status() == Status::Waiting(Decision::Text));
    compare(&after, &ours, 2, 1, "stopped");
    assert_eq!(game.world().party[0].mon.mon.species, PokemonSpecies::Charmander);
}
