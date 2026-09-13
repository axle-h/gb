use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::menu_input::CursorMemory;
use pokered::modes::start_menu::StartMenu;
use pokered::rng::GameRng;
use pokered::world::{BattleStyle, Options, TextSpeed, World, NUM_EVENTS};
use pokered::{Game, Input, Pacing};
use crate::pokemon::options::{self, GameOptionsReader};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{breakpoint, cartridge_until_polling, joypad, open_the_start_menu, recreation_until_polling, tile_row, to_vblank};

/// The world the cartridge is in, as far as these two screens read it: the player's name, the
/// events, one of which decides whether there is a `POKéDEX` row, and the options the option
/// screen is a view of.
fn the_world(gb: &GameBoy) -> World {
    let mmu = gb.core().mmu();
    let base = pokered_symbols::wPlayerName.address;
    let player_name = (0..11)
        .map(|i| mmu.read(base + i))
        .take_while(|&byte| byte != 0x50)
        .collect();
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
    let mut world = World { player_name, options, ..World::default() };
    let events = pokered_symbols::wEventFlags.address;
    for event in 0..NUM_EVENTS as u16 {
        if mmu.read(events + event / 8) & 1 << (event % 8) != 0 {
            world.events.set(event);
        }
    }
    world
}

fn the_menu(gb: &GameBoy) -> Game {
    let mut game = Game::new(the_world(gb), GameRng::seeded(0), Pacing::Faithful);
    *game.menu_mut() = CursorMemory {
        battle_and_start: gb.core().mmu().read_pointer(&pokered_symbols::wBattleAndStartSavedMenuItem),
        ..CursorMemory::default()
    };
    game.push(Mode::StartMenu(StartMenu::new()));
    game
}

/// The menu's own columns. Everything left of them is the overworld the recreation is not drawing.
fn cartridge_menu(gb: &GameBoy) -> Vec<Vec<u8>> {
    (0..16).map(|y| tile_row(gb, y)[10..].to_vec()).collect()
}

fn recreation_menu(game: &Game) -> Vec<Vec<u8>> {
    (0..16).map(|y| game.ui().row(y)[10..].to_vec()).collect()
}

/// The same menu wherever both poll. The cartridge may arrive late, never early.
fn compare(presses: &[Joypad]) {
    let mut gb = open_the_start_menu();
    let mut game = the_menu(&gb);
    for (step, &press) in presses.iter().enumerate() {
        let cartridge = cartridge_until_polling(&mut gb);
        let recreation = recreation_until_polling(&mut game, Decision::StartMenu);
        assert_eq!(cartridge_menu(&gb), recreation_menu(&game), "polling before press {step}");
        assert!((0..=2).contains(&(cartridge as i64 - recreation as i64)),
            "before press {step} the cartridge took {cartridge} frames and the recreation {recreation}");
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::None);
    }
}

#[test]
fn moving_the_start_menu_shows_what_the_cartridge_shows_at_every_poll() {
    use Joypad as J;
    compare(&[J::DOWN, J::DOWN, J::DOWN, J::UP, J::UP, J::UP]);
}

/// Up at the top reaches `EXIT` and down from there comes back, which the start menu does itself
/// because `wMaxMenuItem` is one past the last row.
#[test]
fn the_start_menu_wraps_where_the_cartridge_wraps() {
    compare(&[Joypad::UP, Joypad::DOWN, Joypad::DOWN]);
}

/// The option screen covers the whole width, so this compares every column of it.
#[test]
fn the_option_screen_matches_the_cartridge() {
    let mut gb = open_the_start_menu();
    let mut game = the_menu(&gb);
    let option_row = 5;
    for _ in 0..12 {
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::StartMenu);
        let current = gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem);
        if current == option_row {
            break;
        }
        let press = if current < option_row { Joypad::DOWN } else { Joypad::UP };
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::None);
    }
    assert_eq!(gb.core().mmu().read_pointer(&pokered_symbols::wCurrentMenuItem), option_row);

    gb.hold_buttons(joypad(Joypad::A));
    game.frame(Input::Buttons(Joypad::A));
    let options = breakpoint(pokered_symbols::DisplayOptionMenu);
    let (stop, _) = gb.run_until(&[options], MachineCycles::PER_FRAME * 120);
    assert_eq!(stop, Stop::Breakpoint(options), "A never opened the option screen");
    gb.hold_buttons(JoypadButtonState::default());
    game.frame(Input::None);

    let cartridge = cartridge_until_polling(&mut gb);
    let recreation = recreation_until_polling(&mut game, Decision::Options);
    assert!(matches!(game.modes().last(), Some(Mode::OptionMenu(_))), "the recreation opened it too");
    let whole = |rows: &dyn Fn(usize) -> Vec<u8>| (0..18).map(rows).collect::<Vec<_>>();
    assert_eq!(whole(&|y| tile_row(&gb, y as u16)), whole(&|y| game.ui().row(y).to_vec()),
        "the option screen as drawn, cartridge {cartridge} frames against {recreation}");
    assert_eq!(game.status(), Status::Waiting(Decision::Options));
}
