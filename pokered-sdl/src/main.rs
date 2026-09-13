//! Until the game boots on its own, this sets a scene by hand: Pallet Town, then a text box, the
//! bag, the start menu, a yes/no, the party, the naming screen and a mon's field moves, over and
//! over.

use std::time::{Duration, Instant};
use poke_core::charmap::encode;
use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::map_header::MapHeader;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::EVENT_GOT_POKEDEX;
use pokered::gfx::colour::ColourMode;
use pokered::gfx::compose::{HEIGHT, WIDTH};
use pokered::gfx::layers::MapLayer;
use pokered::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::list_menu::ListMenu;
use pokered::modes::start_menu::StartMenu;
use pokered::modes::text_box::TextBox;
use pokered::modes::field_move_menu::FieldMoveMenu;
use pokered::modes::naming_screen::{NamingScreen, NamingScreenType};
use pokered::modes::party_menu::{PartyMenu, PartyMenuType};
use pokered::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use pokered::party::Named;
use pokered::systems::add_mon::{new_party_mon, Origin};
use pokered::systems::field_moves::field_moves;
use pokered::rng::GameRng;
use pokered::systems::map_data::{camera, tile_block_map, MAP_BORDER};
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Scancode};
use sdl2::pixels::PixelFormatEnum;

const SCALE: u32 = 4;
const FRAME: Duration = Duration::from_nanos(16_742_706);

fn scene() -> Game {
    let mut world = World { player_name: encode("RED").unwrap(), ..World::default() };
    // Without it the start menu is a row shorter.
    world.events.set(EVENT_GOT_POKEDEX as u16);
    world.party = [(PokemonSpecies::Ivysaur, 22, "IVY"), (PokemonSpecies::Pidgey, 9, "PIDGE"),
                   (PokemonSpecies::Rattata, 100, "RAT")]
        .into_iter()
        .map(|(species, level, nick)| {
            let mon = new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
            Named { mon, ot: encode("RED").unwrap(), nick: encode(nick).unwrap() }
        })
        .collect();
    let mut game = Game::new(world, GameRng::from_entropy(), Pacing::Faithful);
    let header = MapHeader::read(Map::PalletTown).expect("Pallet Town has a header");
    let screen = game.screen_mut();
    screen.tiles.load_tileset(header.tileset);
    screen.tiles.load_text_box_tiles();
    screen.tiles.load_font();
    screen.tiles.animation.kind = 2;
    screen.map = MapLayer {
        tileset: Some(header.tileset),
        blocks_wide: header.width as usize + 2 * MAP_BORDER,
        blocks: tile_block_map(Map::PalletTown).unwrap(),
        camera: camera(5, 6),
    };
    game
}

fn next_mode(shown: usize) -> Mode {
    match shown % 7 {
        0 => Mode::TextBox(TextBox::new(encode("Hello, <PLAYER>!<LINE>This is Pallet<CONT>Town, recreated.<PROMPT>").unwrap())),
        1 => Mode::ListMenu(ListMenu::items(vec![
            (ItemId::Potion, 5), (ItemId::PokeBall, 10), (ItemId::Antidote, 2),
            (ItemId::TownMap, 1), (ItemId::Hm01Cut, 1), (ItemId::Repel, 3),
        ], 0, 0)),
        // OPTION is the one row that opens a screen; the other five are still to be recreated.
        2 => Mode::StartMenu(StartMenu::new()),
        // Drawn over the town and taken back down again, which is the part worth watching.
        3 => Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, (14, 7), false)),
        4 => Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal)),
        // START or A on ED hands the name back; B rubs a letter out.
        5 => Mode::NamingScreen(NamingScreen::new(NamingScreenType::Player, None)),
        // Two field moves, so the box is both taller and wider than its empty form.
        _ => Mode::FieldMoveMenu(FieldMoveMenu::new(field_moves([
            PokemonMoveName::Cut as u8, PokemonMoveName::Strength as u8, 0, 0,
        ]))),
    }
}

fn buttons(keys: &sdl2::keyboard::KeyboardState) -> Joypad {
    [
        (Scancode::Z, Joypad::A), (Scancode::X, Joypad::B), (Scancode::Return, Joypad::START),
        (Scancode::RShift, Joypad::SELECT), (Scancode::Up, Joypad::UP), (Scancode::Down, Joypad::DOWN),
        (Scancode::Left, Joypad::LEFT), (Scancode::Right, Joypad::RIGHT),
    ].into_iter().filter(|(key, _)| keys.is_scancode_pressed(*key)).fold(Joypad::empty(), |held, (_, b)| held | b)
}

fn main() -> Result<(), String> {
    let sdl = sdl2::init()?;
    let window = sdl.video()?
        .window("pokered", WIDTH as u32 * SCALE, HEIGHT as u32 * SCALE)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
    let creator = canvas.texture_creator();
    let mut texture = creator
        .create_texture_streaming(PixelFormatEnum::RGB24, WIDTH as u32, HEIGHT as u32)
        .map_err(|e| e.to_string())?;
    let mut events = sdl.event_pump()?;

    let mut game = scene();
    let mut shown = 0;
    let mut saved: Option<Vec<u8>> = None;
    let mut next = Instant::now();
    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } | Event::KeyDown { keycode: Some(Keycode::Escape), .. } => break 'running,
                Event::KeyDown { keycode: Some(Keycode::F5), .. } => saved = Some(game.save()),
                Event::KeyDown { keycode: Some(Keycode::F9), .. } => if let Some(save) = &saved {
                    game = Game::load(save, Pacing::Faithful)?;
                },
                _ => {}
            }
        }
        if game.modes().is_empty() {
            game.screen_mut().ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
            game.push(next_mode(shown));
            shown += 1;
        }
        game.frame(Input::Buttons(buttons(&events.keyboard_state())));

        let rgb = ColourMode::Dmg.rgb(&game.screen().frame());
        texture.update(None, &rgb, WIDTH * 3).map_err(|e| e.to_string())?;
        canvas.copy(&texture, None, None)?;
        canvas.present();

        next += FRAME;
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            None => next = Instant::now(),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes raw RGB for a look; `POKERED_DUMP` names the directory.
    #[test]
    #[ignore = "a tool: dumps frames of the scene"]
    fn dump_scene() {
        let dir = std::env::var("POKERED_DUMP").unwrap();
        let mut game = scene();
        for shown in 0..7 {
            game.screen_mut().ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
            game.push(next_mode(shown));
            for _ in 0..120 {
                game.frame(Input::None);
            }
            std::fs::write(format!("{dir}/scene-{shown}.rgb"), ColourMode::Dmg.rgb(&game.screen().frame())).unwrap();
            while !game.modes().is_empty() {
                game.frame(Input::Buttons(Joypad::B));
                game.frame(Input::None);
            }
        }
    }
}
