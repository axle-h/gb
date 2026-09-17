//! The trainer card the way a player reaches it: START, the player's own row, the card, and back.
//! Opening it turns the LCD off to load its tiles and so does leaving, so neither way is timed;
//! what is compared is the whole screen, as tiles and as the picture on the LCD.

use gb::game_boy::GameBoy;
use gb::ram::RAM;
use pokered::command::Decision;
use pokered::gfx::compose::WIDTH;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::menu_input::CursorMemory;
use pokered::modes::start_menu::StartMenu;
use pokered::rng::GameRng;
use pokered::systems::play_time::PlayTime;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::status_screen::{cartridge_until_polling, ours, press, recreation_until, screen, the_world};
use super::{assert_late, open_the_start_menu, CURSOR, DELAY3};

/// The player's row, below `POKéDEX`, `POKéMON` and `ITEM`.
const TRAINER_INFO_ROW: u8 = 3;

fn lcd(gb: &GameBoy) -> Vec<u8> {
    gb.core().mmu().ppu().screenshot().pixels().map(|p| match p.0[0] {
        0xFF => 0,
        0xAA => 1,
        0x55 => 2,
        _ => 3,
    }).collect()
}

fn assert_same_picture(gb: &GameBoy, game: &Game, what: &str) {
    let (cartridge, recreation) = (lcd(gb), game.screen().frame().shades);
    let wrong: Vec<(usize, usize)> = (0..cartridge.len())
        .filter(|&i| cartridge[i] != recreation[i])
        .map(|i| (i % WIDTH, i / WIDTH))
        .collect();
    assert!(wrong.is_empty(), "{what}: {} pixels differ, first {:?}", wrong.len(), &wrong[..wrong.len().min(8)]);
}

/// The fixture's world, with what only this screen reads: the money, the badges and the clock.
fn the_game(gb: &GameBoy) -> Game {
    let mmu = gb.core().mmu();
    let mut world = the_world(gb);
    world.money.copy_from_slice(&mmu.read_pointer_vec(&pokered_symbols::wPlayerMoney, 3));
    world.badges = mmu.read_pointer(&pokered_symbols::wObtainedBadges);
    let time = mmu.read_pointer_vec(&pokered_symbols::wPlayTimeHours, 5);
    world.play_time = PlayTime { hours: time[0], maxed: time[1] != 0, minutes: time[2], seconds: time[3], frames: time[4], counting: false };
    let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
    *game.menu_mut() = CursorMemory {
        battle_and_start: mmu.read_pointer(&pokered_symbols::wBattleAndStartSavedMenuItem),
        ..CursorMemory::default()
    };
    // What the overworld leaves loaded, which the card draws over only in part.
    game.screen_mut().tiles.load_font();
    game.screen_mut().tiles.load_text_box_tiles();
    game.push(Mode::StartMenu(StartMenu::new()));
    game
}

fn the_card(badges: Option<u8>) {
    let mut gb = open_the_start_menu();
    if let Some(badges) = badges {
        gb.core_mut().mmu_mut().write(pokered_symbols::wObtainedBadges.address, badges);
    }
    let mut game = the_game(&gb);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::StartMenu);
    while gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem) != TRAINER_INFO_ROW {
        let current = gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem);
        press(&mut gb, &mut game, if current < TRAINER_INFO_ROW { Joypad::DOWN } else { Joypad::UP });
        let cartridge = cartridge_until_polling(&mut gb);
        let recreation = recreation_until(&mut game, Decision::StartMenu);
        assert_late(cartridge, recreation, CURSOR, "the start menu");
    }

    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::TrainerCard);
    assert_eq!(screen(&gb), ours(&game), "the card's tiles");
    let picture = gb.core().mmu().read_vram_slice(0x9000, 0x1C * 16).unwrap().to_vec();
    let mut tiles = pokered::gfx::tiles::TileData::default();
    tiles.load(0, &picture);
    for id in 0..0x1C {
        assert_eq!(game.screen().tiles.bg(id), tiles.obj(id), "the picture's tile ${id:02X}");
    }
    // The card polls in the frame it finishes drawing, before `AutoBgMapTransfer` has moved the
    // screen to VRAM a third at a time; the picture is compared once it has.
    for _ in 0..DELAY3 {
        assert_eq!(cartridge_until_polling(&mut gb), 1, "the card polls once a frame");
        game.frame(Input::None);
    }
    assert_same_picture(&gb, &game, "the card");

    press(&mut gb, &mut game, Joypad::A);
    cartridge_until_polling(&mut gb);
    recreation_until(&mut game, Decision::StartMenu);
    let menu = |rows: Vec<Vec<u8>>| rows.into_iter().take(16).map(|row| row[10..].to_vec()).collect::<Vec<_>>();
    assert_eq!(menu(screen(&gb)), menu(ours(&game)), "the start menu, back on the player's row");
}

#[test]
fn the_trainer_card_matches_the_cartridge() {
    the_card(None);
}

/// Won and not won side by side, and the Rainbow Badge's three palette blocks among them.
#[test]
fn a_badge_won_shows_its_badge_and_one_not_won_its_leader() {
    the_card(Some(0b1010_1001));
}
