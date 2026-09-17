//! The game saved from the start menu, against the cartridge: the Celadon fixture standing in the
//! overworld, then START, SAVE, and YES to both the question and the older file it erases, with the
//! screen compared wherever either side settles and the frames "Now saving..." is held for counted
//! on both.
//!
//! What goes into SRAM is not compared, because the recreation's save is its own format. What a
//! player sees of it, and how long it takes, is.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::ram::ROM;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::start_menu::StartMenu;
use pokered::rng::GameRng;
use pokered::systems::play_time::PlayTime;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer, DmgPointerRead};
use super::item_menu::the_world;
use super::status_screen::{cartridge_until_polling, ours, press, recreation_until, screen};
use super::{assert_late, breakpoint, open_the_start_menu, tile_row, to_vblank, ARROW, BOX, CURSOR};

/// `SAVE` is the fifth row once the player has the Pokédex, which the Celadon fixture does.
const SAVE_ROW: u8 = 4;
/// `wSaveFileStatus`: 2 is a save file loaded and good, 1 is none.
const SAVE_FILE_FOUND: u8 = 2;
/// SRAM as `dump_sram` lays it out, a bank after another. "Save Data" is the second.
const SRAM_BANK: usize = 0x2000;
const S_PLAYER_NAME: usize = SRAM_BANK + (sym::sPlayerName.address - 0xA000) as usize;
const S_PLAYER_ID: usize = SRAM_BANK + (sym::sMainData.address - 0xA000) as usize
    + (sym::wPlayerID.address - sym::wMainDataStart.address) as usize;
/// `CheckPreviousSaveFile`'s `CalcCheckSum` over the whole of `sGameData`: lag frames.
const CHECK_PREVIOUS_SAVE_FILE: u32 = 4;
/// `SaveGameData`: three passes over `sGameData`, each copying it and then summing it, all lag.
const SAVE_GAME_DATA: u32 = 15;
/// The `DelayFrames 120` "Now saving..." is held for.
const NOW_SAVING: u32 = 120;

/// `LoadTextBoxTilePatterns` through `CopyVideoData`, eight tiles a frame and a frame to finish.
fn text_box_tiles() -> u32 {
    let tiles = |start: DmgPointer, end: DmgPointer| (end.address - start.address) as u32 / 16;
    tiles(sym::TextBoxGraphics, sym::TextBoxGraphicsEnd) / 8 + 1
}

/// The recreation where the cartridge is: its world as the save screen reads it, the screen it has
/// drawn, and the start menu over it.
fn the_game(gb: &GameBoy) -> Game {
    let mmu = gb.core().mmu();
    let mut world = the_world(gb);
    world.badges = mmu.read_pointer(&sym::wObtainedBadges);
    world.player_id = mmu.read_u16_be(sym::wPlayerID.address);
    world.pokedex.owned.copy_from_slice(&mmu.read_pointer_vec(&sym::wPokedexOwned, 19));
    let time = mmu.read_pointer_vec(&sym::wPlayTimeHours, 5);
    world.play_time = PlayTime { hours: time[0], maxed: time[1] != 0, minutes: time[2], seconds: time[3], frames: time[4], counting: true };
    let player_id = world.player_id;

    let saved = saved_player_id(gb);
    assert_ne!(saved, Some(player_id), "the fixture has never saved, so SRAM holds someone else's game");
    let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
    game.set_saved_player_id(saved);
    game.menu_mut().battle_and_start = mmu.read_pointer(&sym::wBattleAndStartSavedMenuItem);
    for y in 0..18 {
        for (x, tile) in tile_row(gb, y).into_iter().enumerate() {
            game.screen_mut().ui.set(x, y as usize, tile);
        }
    }
    game.push(Mode::StartMenu(StartMenu::new()));
    game
}

/// `CheckPreviousSaveFile` read off SRAM, which is what the host answers natively: an empty
/// `sPlayerName` is no save file, and otherwise the id is `sMainData`'s copy of `wPlayerID`.
fn saved_player_id(gb: &GameBoy) -> Option<u16> {
    if gb.core().mmu().read_pointer(&sym::wSaveFileStatus) != SAVE_FILE_FOUND {
        return None;
    }
    let sram = gb.dump_sram();
    (sram[S_PLAYER_NAME] != 0).then(|| u16::from_be_bytes([sram[S_PLAYER_ID], sram[S_PLAYER_ID + 1]]))
}

/// Frames until the cartridge's tile map says what `ready` is looking for.
fn cartridge_until_screen(gb: &mut GameBoy, what: &str, ready: impl Fn(&GameBoy) -> bool) -> u32 {
    for frames in 1..900 {
        to_vblank(gb);
        if ready(gb) {
            return frames;
        }
    }
    panic!("the cartridge never reached {what}");
}

fn recreation_until_screen(game: &mut Game, what: &str, ready: impl Fn(&Game) -> bool) -> u32 {
    for frames in 1..900 {
        game.frame(Input::None);
        if ready(game) {
            return frames;
        }
    }
    panic!("the recreation never reached {what}");
}

/// Frames until the cartridge reaches `label`, finishing that frame.
fn cartridge_until(gb: &mut GameBoy, label: DmgPointer) -> u32 {
    let (at, vblank) = (breakpoint(label), breakpoint(sym::VBlank));
    for frames in 1..900 {
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
    for frames in 1..900 {
        game.frame(Input::None);
        if game.modes().is_empty() {
            return frames;
        }
    }
    panic!("the start menu never closed");
}

fn says(row: &[u8], from: usize, text: &str) -> bool {
    let bytes = poke_core::charmap::encode(text).expect("the text encodes");
    row[from..from + bytes.len()] == bytes[..]
}

fn cartridge_now_saving(gb: &GameBoy) -> bool {
    says(&tile_row(gb, 14), 1, "Now saving...")
}

fn recreation_now_saving(game: &Game) -> bool {
    says(game.ui().row(14), 1, "Now saving...")
}

#[test]
fn saving_from_the_start_menu_matches_the_cartridge() {
    let mut gb = open_the_start_menu();
    let mut game = the_game(&gb);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::StartMenu);
    // The cursor walks down on both sides rather than being placed, so the menu compares too.
    for _ in 0..SAVE_ROW {
        assert!(gb.core().mmu().read_pointer(&sym::wCurrentMenuItem) < SAVE_ROW, "past SAVE");
        press(&mut gb, &mut game, Joypad::DOWN);
        let cartridge = cartridge_until_polling(&mut gb);
        let recreation = recreation_until(&mut game, Decision::StartMenu);
        assert_late(cartridge, recreation, CURSOR, "down a row");
    }
    assert_eq!(gb.core().mmu().read_pointer(&sym::wCurrentMenuItem), SAVE_ROW);
    assert_eq!(screen(&gb), ours(&game), "the start menu on SAVE");

    // `PrintSaveScreenText`'s summary, the question, and the yes/no under it.
    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until_polling(&mut gb);
    let recreation = recreation_until(&mut game, Decision::TwoOption);
    assert_eq!(screen(&gb), ours(&game), "the save screen and its question");
    assert_late(cartridge, recreation, text_box_tiles() + BOX + CURSOR, "the yes/no");

    // YES. The fixture's SRAM holds someone else's game, so both ask again before erasing it.
    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until_polling(&mut gb);
    let recreation = recreation_until(&mut game, Decision::Text);
    assert_eq!(screen(&gb), ours(&game), "the older file will be erased");
    assert_late(cartridge, recreation, CHECK_PREVIOUS_SAVE_FILE + BOX + ARROW, "the warning");

    // Its `cont` scrolls the box before the second yes/no goes up.
    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until_polling(&mut gb);
    let recreation = recreation_until(&mut game, Decision::TwoOption);
    assert_eq!(screen(&gb), ours(&game), "\"save. Okay?\" and its yes/no");
    assert_late(cartridge, recreation, CURSOR, "the second yes/no");

    // YES again. The cartridge writes SRAM here, which is lag; both then hold "Now saving...".
    press(&mut gb, &mut game, Joypad::A);
    let cartridge = cartridge_until_screen(&mut gb, "\"Now saving...\"", cartridge_now_saving);
    let recreation = recreation_until_screen(&mut game, "\"Now saving...\"", recreation_now_saving);
    assert_eq!(screen(&gb), ours(&game), "the game being written");
    assert_late(cartridge, recreation, SAVE_GAME_DATA, "\"Now saving...\"");

    let cartridge = cartridge_until_screen(&mut gb, "the box coming down", |gb| !cartridge_now_saving(gb));
    let recreation = recreation_until_screen(&mut game, "the box coming down", |game| !recreation_now_saving(game));
    assert_eq!(recreation, NOW_SAVING, "the game's own `DelayFrames 120`");
    assert_late(cartridge, recreation, 0, "the frames \"Now saving...\" is held for");

    // "<PLAYER> saved the game!".
    let mut said = game.world().player_name.clone();
    said.extend(poke_core::charmap::encode(" saved").expect("the text encodes"));
    let saved = |row: &[u8]| row[1..1 + said.len()] == said[..];
    let cartridge = cartridge_until_screen(&mut gb, "\"saved the game!\"", |gb| saved(&tile_row(gb, 14)));
    let recreation = recreation_until_screen(&mut game, "\"saved the game!\"", |game| saved(game.ui().row(14)));
    assert_eq!(screen(&gb), ours(&game), "the game saved");
    assert_late(cartridge, recreation, BOX, "\"saved the game!\"");

    // `SFX_SAVE` played out, the last 30 frames, and `HoldTextDisplayOpen` into `CloseTextDisplay`:
    // the SAVE row closes the start menu rather than coming back to it.
    let cartridge = cartridge_until(&mut gb, sym::CloseTextDisplay);
    let recreation = recreation_until_gone(&mut game);
    assert_late(cartridge, recreation, 0, "the start menu closed");
}
