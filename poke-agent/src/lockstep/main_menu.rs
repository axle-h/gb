//! The main menu, with no save and with one. With none it is the cartridge booted cold and START
//! pressed at the title; with one it is the Celadon fixture's call of the start menu turned into a
//! call of `MainMenu`, told a save was found without loading one, so the summary under `CONTINUE`
//! has the fixture's own player, badges, Pokédex and clock to show.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::{RAM, ROM};
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::main_menu::MainMenu;
use pokered::rng::GameRng;
use pokered::systems::play_time::PlayTime;
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer, DmgPointerRead};
use super::learn_move::hijack;
use super::status_screen::{cartridge_until_polling, ours, press, recreation_until, screen, the_world};
use super::{assert_late, breakpoint, open_the_start_menu, to_vblank, CURSOR, DELAY3};

/// `.mainMenuLoop` before it takes input: `ClearScreen`'s `Delay3`, the text box tiles and the font
/// copied through `CopyVideoData` eight tiles a frame and a frame to finish, and the cursor's.
fn menu_loading() -> u32 {
    let tiles = |start: DmgPointer, end: DmgPointer, bytes| (end.address - start.address) as u32 / bytes;
    let text_box = tiles(sym::TextBoxGraphics, sym::TextBoxGraphicsEnd, 16) / 8 + 1;
    let font = tiles(sym::FontGraphics, sym::FontGraphicsEnd, 8) / 8 + 1;
    DELAY3 + text_box + font + CURSOR
}

/// Frames until the cartridge reaches `label`, finishing that frame.
fn cartridge_until(gb: &mut GameBoy, label: DmgPointer) -> u32 {
    let (at, vblank) = (breakpoint(label), breakpoint(sym::VBlank));
    for frames in 1..2000 {
        match gb.run_until(&[at, vblank], MachineCycles::PER_FRAME * 600).0 {
            Stop::Breakpoint(hit) if hit == at => {
                to_vblank(gb);
                return frames;
            }
            Stop::Breakpoint(_) => {}
            stop => panic!("the cartridge never reached {label}: {stop:?}"),
        }
    }
    panic!("the cartridge never reached {label}");
}

fn recreation_until_gone(game: &mut Game) -> u32 {
    for frames in 1..2000 {
        game.frame(Input::None);
        if game.modes().is_empty() {
            return frames;
        }
    }
    panic!("the main menu never answered");
}

fn step(gb: &mut GameBoy, game: &mut Game, button: Joypad, then: Decision, loading: u32, what: &str) {
    press(gb, game, button);
    let cartridge = cartridge_until_polling(gb);
    let recreation = recreation_until(game, then);
    assert_eq!(screen(gb), ours(game), "{what}");
    assert_late(cartridge, recreation, loading, what);
}

#[test]
fn with_no_save_new_game_and_option_match_the_cartridge() {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    let main_menu = breakpoint(sym::MainMenu);
    let mut reached = false;
    for round in 0..2000 {
        gb.hold_buttons(JoypadButtonState { start: round % 2 == 0, ..Default::default() });
        if gb.run_until(&[main_menu], MachineCycles::PER_FRAME * 3).0 == Stop::Breakpoint(main_menu) {
            reached = true;
            break;
        }
    }
    assert!(reached, "the title never gave way to the main menu");
    gb.hold_buttons(JoypadButtonState::default());
    assert_eq!(gb.core().mmu().read_pointer(&sym::wNumBagItems), 0, "a cold boot, with no save loaded");

    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::MainMenu(MainMenu::new(false)));
    let cartridge = cartridge_until_polling(&mut gb);
    let recreation = recreation_until(&mut game, Decision::MainMenu);
    assert_eq!(screen(&gb), ours(&game), "NEW GAME and OPTION");
    assert_late(cartridge, recreation, menu_loading(), "the menu's first poll");

    step(&mut gb, &mut game, Joypad::DOWN, Decision::MainMenu, CURSOR, "down to OPTION");
    // The option screen draws straight over the menu, with no `ClearScreen` before it.
    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::Options);
    assert_eq!(screen(&gb), ours(&game), "the option screen over the menu");
    step(&mut gb, &mut game, Joypad::B, Decision::MainMenu, menu_loading(), "back to the menu from the top");
    assert_eq!(gb.core().mmu().read_pointer(&sym::wCurrentMenuItem), 0, "on NEW GAME again");

    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until(&mut gb, sym::OakSpeech);
    let recreation = recreation_until_gone(&mut game);
    assert_eq!(game.menu().chosen_item, 0, "nothing else was chosen on the way");
    assert_late(cartridge, recreation, 0, "NEW GAME, answered after its 20 frames");
}

#[test]
fn with_a_save_continue_shows_the_save_and_takes_it() {
    let mut gb = open_the_start_menu();
    hijack(&mut gb, sym::MainMenu);
    let check = breakpoint(sym::CheckForPlayerNameInSRAM);
    assert_eq!(gb.run_until(&[check], MachineCycles::PER_FRAME * 10).0, Stop::Breakpoint(check));
    // A save found and already in WRAM: the status says so, and no carry skips `TryLoadSaveFile`.
    gb.core_mut().mmu_mut().write(sym::wSaveFileStatus.address, 2);
    let sp = gb.core().registers().sp;
    let back = gb.core().mmu().read_u16_le(sp);
    let registers = gb.core_mut().registers_mut();
    (registers.sp, registers.pc, registers.flags.c) = (sp + 2, back, false);

    let mmu = gb.core().mmu();
    let mut world = the_world(&gb);
    world.badges = mmu.read_pointer(&sym::wObtainedBadges);
    world.pokedex.owned.copy_from_slice(&mmu.read_pointer_vec(&sym::wPokedexOwned, 19));
    let time = mmu.read_pointer_vec(&sym::wPlayTimeHours, 5);
    world.play_time = PlayTime { hours: time[0], maxed: time[1] != 0, minutes: time[2], seconds: time[3], frames: time[4], counting: false };
    let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::MainMenu(MainMenu::new(true)));

    let cartridge = cartridge_until_polling(&mut gb);
    let recreation = recreation_until(&mut game, Decision::MainMenu);
    assert_eq!(screen(&gb), ours(&game), "CONTINUE, NEW GAME and OPTION");
    assert_late(cartridge, recreation, menu_loading(), "the menu's first poll");

    // `.inputLoop` reads the pad with `Joypad` rather than `JoypadLowSensitivity`, and nothing else
    // calls it between here and there.
    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until(&mut gb, sym::Joypad);
    let recreation = recreation_until(&mut game, Decision::ContinueGame);
    assert_eq!(screen(&gb), ours(&game), "the save's summary");
    assert_late(cartridge, recreation, 0, "the summary, after 20 frames and its own 30");

    step(&mut gb, &mut game, Joypad::B, Decision::MainMenu, menu_loading(), "B back to the menu");

    press(&mut gb, &mut game, Joypad::A);
    cartridge_until(&mut gb, sym::Joypad);
    recreation_until(&mut game, Decision::ContinueGame);
    // `.pressedA`'s `GBPalWhiteOutWithDelay3` and `ClearScreen` are loading; its 10 frames are not.
    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until(&mut gb, sym::SpecialEnterMap);
    let recreation = recreation_until_gone(&mut game);
    assert_eq!(game.status(), Status::Idle);
    assert_late(cartridge, recreation, 2 * DELAY3, "CONTINUE taken");
}
