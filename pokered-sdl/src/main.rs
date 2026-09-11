//! Until the game boots on its own, this sets a scene by hand: Pallet Town, then a text box and
//! the bag, over and over.

use std::time::{Duration, Instant};
use poke_core::charmap::encode;
use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::map_header::MapHeader;
use pokered::gfx::colour::ColourMode;
use pokered::gfx::compose::{HEIGHT, WIDTH};
use pokered::gfx::layers::MapLayer;
use pokered::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::list_menu::ListMenu;
use pokered::modes::text_box::TextBox;
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
    let world = World { player_name: encode("RED").unwrap(), ..World::default() };
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
    if shown % 2 == 0 {
        Mode::TextBox(TextBox::new(encode("Hello, <PLAYER>!<LINE>This is Pallet<CONT>Town, recreated.<PROMPT>").unwrap()))
    } else {
        Mode::ListMenu(ListMenu::items(vec![
            (ItemId::Potion, 5), (ItemId::PokeBall, 10), (ItemId::Antidote, 2),
            (ItemId::TownMap, 1), (ItemId::Hm01Cut, 1), (ItemId::Repel, 3),
        ], 0, 0))
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
        for shown in 0..2 {
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
