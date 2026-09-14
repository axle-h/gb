//! The status screen the way a player reaches it: START, `POKéMON`, the first mon, `STATS`, both
//! pages, and back through the party menu to the start menu. The recreation runs the same presses
//! through its own start menu, so `PokemonMenu` is compared as well as the two pages.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::menu_input::CursorMemory;
use pokered::modes::start_menu::StartMenu;
use pokered::party::{BoxMon, Named, PartyMon};
use pokered::rng::GameRng;
use pokered::systems::pokedex::front_pic_tiles;
use pokered::systems::stats::Dvs;
use pokered::world::{BattleStyle, Options, TextSpeed, World, NUM_EVENTS};
use pokered::{Game, Input, Pacing};
use crate::pokemon::options::{self, GameOptionsReader};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{assert_late, breakpoint, joypad, open_the_start_menu, tile_row, to_vblank, CURSOR, DELAY3};

const PARTY_STRUCT: u16 = 0x2C;
const NAME_LENGTH: u16 = 11;
const TERMINATOR: u8 = 0x50;
/// `vFrontPic`, which the picture is 49 tiles of.
const FRONT_PIC: u16 = 0x9000;
/// The lag frames page two costs beyond a menu's two, in `GetMaxPP`.
const GET_MAX_PP_LAG: u32 = 4;

fn name_at(gb: &GameBoy, at: u16) -> Vec<u8> {
    let mmu = gb.core().mmu();
    (0..NAME_LENGTH).map(|i| mmu.read(at + i)).take_while(|&byte| byte != TERMINATOR).collect()
}

/// `party_struct` as WRAM lays it out, with the nickname and OT beside it.
pub(super) fn the_party(gb: &GameBoy) -> Vec<Named<PartyMon>> {
    let mmu = gb.core().mmu();
    let count = mmu.read_pointer(&pokered_symbols::wPartyCount);
    (0..count as u16).map(|slot| {
        let at = pokered_symbols::wPartyMon1.address + PARTY_STRUCT * slot;
        let b: Vec<u8> = (0..PARTY_STRUCT).map(|i| mmu.read(at + i)).collect();
        let word = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        let mon = BoxMon {
            species: PokemonSpecies::from_repr(b[0]).expect("a species"),
            hp: word(1),
            box_level: b[3],
            status: b[4],
            types: [b[5], b[6]],
            catch_rate: b[7],
            moves: [0, 1, 2, 3].map(|i| PokemonMoveName::from_repr(b[8 + i])),
            ot_id: word(12),
            exp: u32::from_be_bytes([0, b[14], b[15], b[16]]),
            stat_exp: [0, 1, 2, 3, 4].map(|i| word(17 + 2 * i)),
            dvs: Dvs([b[27], b[28]]),
            pp: [b[29], b[30], b[31], b[32]],
        };
        let mon = PartyMon { mon, level: b[33], stats: [0, 1, 2, 3, 4].map(|i| word(34 + 2 * i)) };
        Named {
            mon,
            ot: name_at(gb, pokered_symbols::wPartyMonOT.address + NAME_LENGTH * slot),
            nick: name_at(gb, pokered_symbols::wPartyMonNicks.address + NAME_LENGTH * slot),
        }
    }).collect()
}

/// The world as far as these screens read it: the player's name, the events (one of which decides
/// whether the start menu has a `POKéDEX` row), the text speed and the party.
pub(super) fn the_world(gb: &GameBoy) -> World {
    let mmu = gb.core().mmu();
    let live = mmu.read_game_options().expect("the fixture's options");
    let options = Options {
        text_speed: match live.text_speed {
            options::TextSpeed::Fast => TextSpeed::Fast,
            options::TextSpeed::Medium => TextSpeed::Medium,
            options::TextSpeed::Slow => TextSpeed::Slow,
        },
        battle_animation: live.battle_animations_on,
        battle_style: match live.battle_style {
            options::BattleStyle::Set => BattleStyle::Set,
            options::BattleStyle::Shift => BattleStyle::Shift,
        },
    };
    let mut world = World {
        player_name: name_at(gb, pokered_symbols::wPlayerName.address),
        party: the_party(gb),
        options,
        ..World::default()
    };
    let events = pokered_symbols::wEventFlags.address;
    for event in 0..NUM_EVENTS as u16 {
        if mmu.read(events + event / 8) & 1 << (event % 8) != 0 {
            world.events.set(event);
        }
    }
    world
}

fn the_game(gb: &GameBoy) -> Game {
    let mmu = gb.core().mmu();
    let mut game = Game::new(the_world(gb), GameRng::seeded(0), Pacing::Faithful);
    *game.menu_mut() = CursorMemory {
        battle_and_start: mmu.read_pointer(&pokered_symbols::wBattleAndStartSavedMenuItem),
        party_and_bills: mmu.read_pointer(&pokered_symbols::wPartyAndBillsPCSavedMenuItem),
        ..CursorMemory::default()
    };
    game.push(Mode::StartMenu(StartMenu::new()));
    game
}

/// VBlanks until the cartridge next polls the pad, finishing that frame. The party menu turns the
/// LCD off while it loads, so a stretch with no VBlank at all is waited through rather than failed.
pub(super) fn cartridge_until_polling(gb: &mut GameBoy) -> u32 {
    let poll = breakpoint(pokered_symbols::JoypadLowSensitivity);
    let vblank = breakpoint(pokered_symbols::VBlank);
    let mut frames = 0;
    loop {
        match gb.run_until(&[poll, vblank], MachineCycles::PER_FRAME * 600).0 {
            Stop::Breakpoint(hit) if hit == poll => {
                to_vblank(gb);
                return frames + 1;
            }
            Stop::Breakpoint(_) => frames += 1,
            stop => panic!("the cartridge never polled: {stop:?}"),
        }
        assert!(frames < 2000, "the cartridge never polled");
    }
}

pub(super) fn recreation_until(game: &mut Game, decision: Decision) -> u32 {
    for frames in 1..2000 {
        game.frame(Input::None);
        if game.status() == Status::Waiting(decision.clone()) {
            return frames;
        }
    }
    panic!("the recreation never waited for {decision:?}");
}

pub(super) fn press(gb: &mut GameBoy, game: &mut Game, button: Joypad) {
    gb.hold_buttons(joypad(button));
    game.frame(Input::Buttons(button));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    game.frame(Input::None);
}

pub(super) fn screen(gb: &GameBoy) -> Vec<Vec<u8>> {
    (0..18).map(|y| tile_row(gb, y)).collect()
}

pub(super) fn ours(game: &Game) -> Vec<Vec<u8>> {
    (0..18).map(|y| game.ui().row(y).to_vec()).collect()
}

/// The start menu's own columns: left of them is the overworld, which the recreation does not draw.
fn menu_columns(rows: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    rows.into_iter().take(16).map(|row| row[10..].to_vec()).collect()
}

/// One press on both sides, then both run to their next poll; every one of these is a menu moving or
/// opening, so the cartridge is late by `HandleMenuInput`'s `Delay3`.
fn step(gb: &mut GameBoy, game: &mut Game, button: Joypad, decision: Decision, what: &str) {
    press(gb, game, button);
    let cartridge = cartridge_until_polling(gb);
    let recreation = recreation_until(game, decision);
    println!("{what}: cartridge {cartridge} frames, recreation {recreation}");
    assert_late(cartridge, recreation, CURSOR, what);
}

#[test]
fn the_status_screen_and_the_way_to_it_match_the_cartridge() {
    let mut gb = open_the_start_menu();
    let mut game = the_game(&gb);
    assert!(game.world().party.len() > 1, "the fixture has a party to look at");
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::StartMenu);

    // `POKéMON` is the second row once the player has the Pokédex.
    while gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem) != 1 {
        let button = if gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem) < 1 { Joypad::DOWN } else { Joypad::UP };
        step(&mut gb, &mut game, button, Decision::StartMenu, "the start menu");
    }
    assert_eq!(menu_columns(screen(&gb)), menu_columns(ours(&game)), "the start menu on POKéMON");

    // The party menu's first poll is not timed: the cartridge loads its icons with the LCD off.
    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::PartyMenu);
    assert_eq!(screen(&gb)[..12], ours(&game)[..12], "the party menu");
    while gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem) != 0 {
        step(&mut gb, &mut game, Joypad::UP, Decision::PartyMenu, "the party menu");
    }

    step(&mut gb, &mut game, Joypad::A, Decision::FieldMoveMenu, "the submenu");
    assert_eq!(screen(&gb)[..12], ours(&game)[..12], "the party behind the submenu");
    assert_eq!(screen(&gb)[12..], ours(&game)[12..], "the submenu");

    // `STATS` is the row after the field moves, and the menu opens on the first row.
    let field_moves = (0..4)
        .map(|i| gb.core().mmu().read(pokered_symbols::wFieldMoves.address + i))
        .take_while(|&name| name != 0)
        .count();
    for _ in 0..field_moves {
        step(&mut gb, &mut game, Joypad::DOWN, Decision::FieldMoveMenu, "down the submenu");
    }

    // Loading is instant here where the cartridge copies tiles through VBlank a few at a time, so
    // what is timed is the cry: from the frame the picture goes up to the page taking a press. The
    // recreation puts it up in the frame A lands, one before `press` lets go.
    press(&mut gb, &mut game, Joypad::A);
    assert_eq!(game.ui().get(7, 0), 0, "the recreation's picture is up");
    let cry = breakpoint(pokered_symbols::PlayCry);
    let (stop, _) = gb.run_until(&[cry], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(cry), "the status screen never cried");
    let cartridge = cartridge_until_polling(&mut gb) - 1;
    let recreation = 1 + recreation_until(&mut game, Decision::StatusScreen);
    println!("page one: cartridge {cartridge} frames, recreation {recreation}");
    assert_eq!(screen(&gb), ours(&game), "page one");
    let species = game.world().party[0].mon.mon.species;
    let picture = gb.core().mmu().read_vram_slice(FRONT_PIC, 49 * 16).unwrap().to_vec();
    assert_eq!(front_pic_tiles(species, true).concat(), picture, "{species}'s picture");
    assert_late(cartridge, recreation, 0, "the cry");

    // Page two is late by its `Delay3` and by more than a menu's lag: `GetMaxPP` runs `AddBonusPP`'s
    // 256-pass loop for every move with no PP Up, and the PP arrive on screen two moves a frame.
    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until_polling(&mut gb);
    let recreation = recreation_until(&mut game, Decision::StatusScreen);
    println!("page two: cartridge {cartridge} frames, recreation {recreation}");
    assert_late(cartridge, recreation, DELAY3 + GET_MAX_PP_LAG, "page two");
    assert_eq!(screen(&gb), ours(&game), "page two");

    // Back to the party menu from the top, cleared and redrawn, with the LCD off again.
    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::PartyMenu);
    assert_eq!(screen(&gb)[..12], ours(&game)[..12], "the party menu again");

    // Not timed either: `RestoreScreenTilesAndReloadTilePatterns` reloads every sprite's tiles
    // through VBlank, where loading is instant here.
    press(&mut gb, &mut game, Joypad::B);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::StartMenu);
    assert_eq!(menu_columns(screen(&gb)), menu_columns(ours(&game)), "the start menu again");
}
