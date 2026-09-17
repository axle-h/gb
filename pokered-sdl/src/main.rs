//! Until the game boots on its own, this sets a scene by hand: the player outside Red's house in
//! Pallet Town, free to walk, read signs, talk and open the start menu.

use std::time::{Duration, Instant};
use poke_core::bag::BagItem;
use poke_core::charmap::encode;
use poke_core::item::ItemId;
use poke_core::map::Map;
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_events::EVENT_GOT_POKEDEX;
use pokered::gfx::colour::ColourMode;
use pokered::gfx::compose::{HEIGHT, WIDTH};
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::overworld::Overworld;
use pokered::party::Named;
use pokered::rng::GameRng;
use pokered::systems::add_mon::{new_party_mon, Origin};
use pokered::systems::inventory::Inventory;
use pokered::systems::overworld::Location;
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
    world.bag = Inventory::bag(vec![BagItem::new(ItemId::Potion, 5), BagItem::new(ItemId::PokeBall, 10)]);
    world.location = Location { map: Map::PalletTown, x: 5, y: 6, facing: SpriteFacing::Down, ..Location::default() };
    let mut game = Game::new(world, GameRng::from_entropy(), Pacing::Faithful);
    game.screen_mut().tiles.load_font();
    game.push(Mode::Overworld(Overworld::new()));
    game
}

fn buttons(keys: &sdl2::keyboard::KeyboardState) -> Joypad {
    [
        (Scancode::X, Joypad::A), (Scancode::Z, Joypad::B), (Scancode::Return, Joypad::START),
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
            game.push(Mode::Overworld(Overworld::new()));
        }
        game.frame(Input::Buttons(buttons(&events.keyboard_state())));

        let rgb = ColourMode::Dmg.rgb(game.screen());
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
        let walk = [Joypad::UP, Joypad::RIGHT, Joypad::START, Joypad::B];
        for (shown, &button) in walk.iter().enumerate() {
            for _ in 0..20 {
                game.frame(Input::Buttons(button));
            }
            for _ in 0..60 {
                game.frame(Input::None);
            }
            std::fs::write(format!("{dir}/scene-{shown}.rgb"), ColourMode::Dmg.rgb(game.screen())).unwrap();
        }
    }
}
