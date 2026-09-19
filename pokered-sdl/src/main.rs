//! Pokémon Red, recreated, in a desktop window: the game from power-on, played from the keyboard.
//!
//! The window is the only place the recreation is judged by eye and ear rather than against the
//! emulator, so it carries the two switches nothing else can settle: `C` cycles the four colour
//! modes, and `M` mutes.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use pokered::audio::synth::{Synth, SAMPLE_RATE};
use pokered::audio::{Voices, Write};
use pokered::gfx::colour::ColourMode;
use pokered::input::Joypad;
use pokered::rng::GameRng;
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use sdl2::audio::{AudioQueue, AudioSpecDesired};
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Scancode};
use sdl2::pixels::PixelFormatEnum;

const SCALE: u32 = 4;
const FRAME: Duration = Duration::from_nanos(16_742_706);

/// Cycled by `C`. The border is the only one that paints more than the screen.
const COLOUR_MODES: [ColourMode; 4] =
    [ColourMode::Dmg, ColourMode::Gbc, ColourMode::Sgb, ColourMode::SgbBorder];

/// The synth reaches ±1 with four channels at full amplitude, and its output filters ring past it:
/// the title theme peaks at 1.19. The sink is fed through this so those samples are heard rather
/// than squared off.
const HEADROOM: f32 = 1.0 / 1.2;

/// The game's own save file, which the SAVE menu, a box change and the Hall of Fame write. It is
/// what CONTINUE resumes from, so the window reads it back at every power-on.
fn save_path() -> PathBuf {
    std::env::var("POKERED_SAVE").unwrap_or_else(|_| "pokered.sav".to_string()).into()
}

/// The world the save file holds, or `None` for a file that is missing or from another version,
/// which is `TryLoadSaveFile`'s bad checksum: the main menu then offers NEW GAME alone.
fn saved_world() -> Option<World> {
    let bytes = std::fs::read(save_path()).ok()?;
    match Game::load(&bytes, Pacing::Faithful) {
        Ok(game) => Some(game.world().clone()),
        Err(e) => {
            eprintln!("ignoring {}: {e}", save_path().display());
            None
        }
    }
}

fn boot() -> Game {
    Game::power_on(saved_world(), GameRng::from_entropy(), Pacing::Faithful)
}

fn buttons(keys: &sdl2::keyboard::KeyboardState) -> Joypad {
    [
        (Scancode::X, Joypad::A), (Scancode::Z, Joypad::B), (Scancode::Return, Joypad::START),
        (Scancode::RShift, Joypad::SELECT), (Scancode::Up, Joypad::UP), (Scancode::Down, Joypad::DOWN),
        (Scancode::Left, Joypad::LEFT), (Scancode::Right, Joypad::RIGHT),
    ].into_iter().filter(|(key, _)| keys.is_scancode_pressed(*key)).fold(Joypad::empty(), |held, (_, b)| held | b)
}

/// The audio engine writes no `NR52`, because on the cartridge `stopAllAudio` powers the APU on
/// before any music starts. A host driving a backend itself does it here instead, or every write
/// after it is ignored and nothing sounds.
fn synth() -> Synth {
    let mut synth = Synth::new();
    synth.write(Write::Power(true));
    synth
}

fn main() -> Result<(), String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let mut mode = ColourMode::Dmg;
    let (width, height) = mode.size();
    let window = video
        .window("pokered", width as u32 * SCALE, height as u32 * SCALE)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
    let creator = canvas.texture_creator();
    let mut texture = creator
        .create_texture_streaming(PixelFormatEnum::RGB24, width as u32, height as u32)
        .map_err(|e| e.to_string())?;

    let queue: AudioQueue<f32> = sdl.audio()?.open_queue(
        None,
        &AudioSpecDesired { freq: Some(SAMPLE_RATE as i32), channels: Some(2), samples: Some(512) },
    )?;
    queue.resume();
    let mut voices = synth();
    let mut muted = false;
    // One frame's stereo samples, with room for the frame the pacing gives back after a long one.
    let mut samples = vec![0.0f32; SAMPLE_RATE as usize / 8 * 2];

    let mut events = sdl.event_pump()?;
    let mut game = boot();
    let mut state: Option<Vec<u8>> = None;
    let mut next = Instant::now();
    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } | Event::KeyDown { keycode: Some(Keycode::Escape), .. } => break 'running,
                Event::KeyDown { keycode: Some(Keycode::F5), .. } => state = Some(game.save()),
                Event::KeyDown { keycode: Some(Keycode::F9), repeat: false, .. } => if let Some(bytes) = &state {
                    game = Game::load(bytes, Pacing::Faithful)?;
                    // A save carries no oscillator state, so the backend starts over and is told
                    // what the engine is holding, or a note playing across the load goes silent.
                    voices = synth();
                    for write in game.audio().standing_writes() {
                        voices.write(write);
                    }
                    queue.clear();
                },
                Event::KeyDown { keycode: Some(Keycode::C), repeat: false, .. } => {
                    let next = COLOUR_MODES[(COLOUR_MODES.iter().position(|&m| m == mode).unwrap() + 1) % COLOUR_MODES.len()];
                    if next.size() != mode.size() {
                        let (width, height) = next.size();
                        canvas.window_mut().set_size(width as u32 * SCALE, height as u32 * SCALE).map_err(|e| e.to_string())?;
                        texture = creator
                            .create_texture_streaming(PixelFormatEnum::RGB24, width as u32, height as u32)
                            .map_err(|e| e.to_string())?;
                    }
                    mode = next;
                }
                Event::KeyDown { keycode: Some(Keycode::M), repeat: false, .. } => {
                    muted = !muted;
                    queue.clear();
                }
                _ => {}
            }
        }
        // The Hall of Fame's script ends the game with a restart, and so does a blackout on the
        // title screen: what is left is an empty stack, and the console comes back on.
        if game.modes().is_empty() {
            game = boot();
        }

        let frame = game.frame(Input::Buttons(buttons(&events.keyboard_state())));
        if let Some(bytes) = frame.save {
            std::fs::write(save_path(), bytes).map_err(|e| e.to_string())?;
        }

        for write in frame.audio {
            voices.write(write);
        }
        voices.end_frame();
        // Drained whether or not it is heard: the samples are made either way.
        loop {
            let read = voices.read_samples(&mut samples);
            if read == 0 {
                break;
            }
            if !muted {
                for sample in &mut samples[..read * 2] {
                    *sample *= HEADROOM;
                }
                queue.queue_audio(&samples[..read * 2])?;
            }
        }

        let (width, _) = mode.size();
        texture.update(None, &mode.rgb(game.screen()), width * 3).map_err(|e| e.to_string())?;
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
    use pokered::command::{Command, Decision};
    use pokered::mode::{Mode, Status};

    /// What the window would show and play at the title screen, for the two judgements only an eye
    /// and an ear can make: raw RGB in each colour mode, since three of the four have no oracle,
    /// and the theme as a WAV at the level the sink is fed. `POKERED_DUMP` names the directory.
    #[test]
    #[ignore = "a tool: dumps the title screen's picture and music"]
    fn dump_title_screen() {
        let dir = std::env::var("POKERED_DUMP").unwrap();
        let mut game = Game::power_on(None, GameRng::seeded(1), Pacing::Faithful);
        let mut voices = synth();
        let mut samples = vec![0.0f32; SAMPLE_RATE as usize / 8 * 2];
        let mut played = Vec::new();
        while !matches!(game.status(), Status::Waiting(Decision::TitleScreen)) {
            game.frame(Input::None);
        }
        for mode in COLOUR_MODES {
            std::fs::write(format!("{dir}/title-{mode:?}.rgb"), mode.rgb(game.screen())).unwrap();
        }
        for _ in 0..600 {
            let frame = game.frame(Input::None);
            for write in frame.audio {
                voices.write(write);
            }
            voices.end_frame();
            loop {
                let read = voices.read_samples(&mut samples);
                if read == 0 {
                    break;
                }
                played.extend(samples[..read * 2].iter().map(|sample| sample * HEADROOM));
            }
        }
        std::fs::write(format!("{dir}/title.wav"), wav(&played)).unwrap();
    }

    /// 16-bit stereo PCM, the one format that needs no crate to write.
    fn wav(samples: &[f32]) -> Vec<u8> {
        let data: Vec<u8> = samples
            .iter()
            .flat_map(|&sample| ((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
            .collect();
        let rate = SAMPLE_RATE;
        let mut wav = b"RIFF".to_vec();
        wav.extend((36 + data.len() as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend([1u16.to_le_bytes(), 2u16.to_le_bytes()].concat());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 4).to_le_bytes());
        wav.extend([4u16.to_le_bytes(), 16u16.to_le_bytes()].concat());
        wav.extend(b"data");
        wav.extend((data.len() as u32).to_le_bytes());
        wav.extend(data);
        wav
    }

    /// The window's own boot: a new game reaches the overworld from power-on, and everything the
    /// loop does with a frame - the save it writes, the samples it plays - is driven from there.
    #[test]
    fn a_new_game_boots_into_the_overworld_and_sounds() {
        let mut game = Game::power_on(None, GameRng::seeded(1), Pacing::Instant);
        let mut voices = synth();
        let mut samples = vec![0.0f32; SAMPLE_RATE as usize / 8 * 2];
        let mut heard = false;
        for _ in 0..200_000 {
            if matches!(game.modes(), [Mode::Overworld(_)]) {
                break;
            }
            let input = match game.status() {
                Status::Waiting(Decision::TitleScreen | Decision::Text) => Input::Command(Command::Advance),
                // A row of the drawn menu rather than `NEW_GAME`: with no save there is no
                // CONTINUE row, so NEW GAME is the first.
                Status::Waiting(Decision::MainMenu) => Input::Command(Command::ChooseOption(0)),
                // Oak asks twice; the first preset name skips the naming screen either time.
                Status::Waiting(Decision::IntroNameMenu) => Input::Command(Command::ChooseOption(1)),
                _ => Input::None,
            };
            let frame = game.frame(input);
            for write in frame.audio {
                voices.write(write);
            }
            voices.end_frame();
            while voices.read_samples(&mut samples) > 0 {
                heard |= samples.iter().any(|&sample| sample != 0.0);
            }
        }
        assert!(matches!(game.modes(), [Mode::Overworld(_)]), "{:?}", game.status());
        assert!(heard, "the title screen's music reached the backend");
    }
}
