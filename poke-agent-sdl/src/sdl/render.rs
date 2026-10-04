//! The window: the cartridge and the recreation side by side from power-on, the keyboard routed to
//! both or either (`Tab`, or a click on the toggle), the recreation's sound alone, and both games'
//! logs beneath.
//!
//! A game controller presses the pad as the keyboard does, and a state file dropped on either
//! game's half of the window is loaded into that game. The TOUR button, or `T`, plays the grand
//! tour on both games from a fresh game, kept together a step at a time; while it plays, neither
//! the keyboard nor a load reaches either game.
//!
//! Keys besides the pad's (arrows, `X` A, `Z` B, `Return` START, right `Shift` or `Backspace`
//! SELECT): `C` the native colour mode, `M` mute, `1`-`4` the speed (1x, 2x, 4x, as fast as the
//! host can), `T` the tour, `F5` the emulated agent,
//! `F8`/`F9` quick-save and quick-load both games, `Shift` the emulator's alone and `Ctrl` the
//! recreation's, `F11` the window to `window.png`, and the emulator's debugging keys `F1`-`F3`,
//! `F7`, `F10`, `F12`, `W`, `A` and `P`.

use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant};
use sdl2::audio::{AudioQueue, AudioSpecDesired};
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Mod, Scancode};
use sdl2::mouse::MouseButton;
use sdl2::pixels::PixelFormatEnum;
use gb::lcd_control::{TileDataMode, TileMapMode};
use poke_agent::pokemon::{PokemonApi, PokemonApiTrait};
use poke_agent::pokemon::policy::ConsolePolicy;
use pokered::audio::synth::SAMPLE_RATE;
use pokered::gfx::colour::ColourMode;
use pokered::input::Joypad;
use crate::sdl::controller::Controllers;
use crate::sdl::games::{Games, Side};
use crate::sdl::log::Source;
use crate::sdl::speed::Speed;
use crate::sdl::window::{Control, Layout, View};

/// The recreation's frame, about 59.73 Hz, which the emulator's frame of cycles matches.
const FRAME: Duration = Duration::from_nanos(16_742_706);
/// Cycled by `C`. The border is the only one that paints more than the screen.
const COLOUR_MODES: [ColourMode; 4] = [ColourMode::Dmg, ColourMode::Gbc, ColourMode::Sgb, ColourMode::SgbBorder];
/// Audio queued ahead of the device, past which a fast-forward's samples are dropped.
const MAX_QUEUED_SECONDS: f32 = 0.25;

/// The save state and the battery save sit beside this crate's manifest, not in the working
/// directory, so the window plays the same game whichever directory it is started from.
const SAVE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/pokemon-red.bin");
const SRAM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/pokemon-red.sav");
const NATIVE_SAVE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/pokemon-red.pkrd");

/// The recreation's save slots, the directory `pokered-sdl` keeps them in.
pub fn native_slots_dir() -> std::path::PathBuf {
    std::env::var("POKERED_SAVES").unwrap_or_else(|_| "saves".to_string()).into()
}

pub const USAGE: &str = "usage: poke-agent-sdl [--emulated <gb save state>] [--native <pokered save>]";

/// The keys, said into the log when the window opens.
const HELP: &str = "Tab routing, T grand tour on both, C colours, M mute, 1-4 speed (1x, 2x, 4x, max), F5 emulated agent, \
    F8/F9 quick-save/load both, Shift+F8/F9 emulated alone, Ctrl+F8/F9 native alone, F11 window.png. Drop a state on a game's half to load it.";

/// What the command line asks for: a state to load into either game once both are on.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub emulated: Option<std::path::PathBuf>,
    pub native: Option<std::path::PathBuf>,
}

impl Args {
    /// `None` when help is asked for.
    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut parsed = Self::default();
        while let Some(arg) = args.next() {
            let slot = match arg.as_str() {
                "-h" | "--help" => return Ok(None),
                "--emulated" => &mut parsed.emulated,
                "--native" => &mut parsed.native,
                _ => return Err(format!("unexpected argument {arg}")),
            };
            *slot = Some(args.next().ok_or(format!("{arg} needs a path"))?.into());
        }
        Ok(Some(parsed))
    }
}

/// Loads the file at `path` into `side`'s game, and says how that went.
fn load(games: &mut Games, view: &mut View, side: Side, path: &std::path::Path) {
    match games.load_file(side, path) {
        Ok(()) => view.say(Source::Window, format!("loaded {} into the {} game", path.display(), side.label())),
        Err(e) => view.say(Source::Window, e),
    }
}

/// `F8` or `F9`: the quick-save of both games, or with `Shift` the emulator's alone and with `Ctrl`
/// the recreation's, written or read in the files beside the battery save.
fn quick_save(games: &mut Games, view: &mut View, keymod: Mod, load: bool) {
    let both = [(Side::Emulated, Path::new(SAVE)), (Side::Native, Path::new(NATIVE_SAVE))];
    let files = if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) {
        &both[1..]
    } else if keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) {
        &both[..1]
    } else {
        &both[..]
    };
    let games_named = match files {
        [(side, _)] => format!("the {} game", side.label()),
        _ => "both games".to_string(),
    };
    let said = match (load, if load { games.load_files(files) } else { games.save_files(files) }) {
        (_, Err(e)) => e,
        (true, Ok(())) => format!("loaded {games_named}"),
        (false, Ok(())) => format!("saved {games_named}"),
    };
    view.say(Source::Window, said);
}

/// Starts the grand tour on both games, or stops the one playing.
fn tour(games: &mut Games, view: &mut View) {
    match &games.tour {
        Some(tour) if tour.running() => tour.stop(),
        _ => match games.start_tour() {
            Ok(()) => view.say(Source::Window, "the grand tour is on both games from a fresh game, a step at a time".to_string()),
            Err(e) => view.say(Source::Window, format!("could not start the tour: {e}")),
        },
    }
}

/// Where the cursor is over the window, for a drop, which SDL reports without a position.
fn cursor_x(window: &sdl2::video::Window) -> i32 {
    let (mut x, mut y) = (0, 0);
    // SAFETY: SDL is initialised for as long as there is a window, and both pointers are live.
    unsafe { sdl2::sys::SDL_GetGlobalMouseState(&mut x, &mut y) };
    x - window.position().0
}

fn buttons(keys: &sdl2::keyboard::KeyboardState) -> Joypad {
    [
        (Scancode::X, Joypad::A), (Scancode::Z, Joypad::B), (Scancode::Return, Joypad::START),
        (Scancode::RShift, Joypad::SELECT), (Scancode::Backspace, Joypad::SELECT), (Scancode::Up, Joypad::UP),
        (Scancode::Down, Joypad::DOWN), (Scancode::Left, Joypad::LEFT), (Scancode::Right, Joypad::RIGHT),
    ].into_iter().filter(|(key, _)| keys.is_scancode_pressed(*key)).fold(Joypad::empty(), |held, (_, b)| held | b)
}

pub fn render(args: Args) -> Result<(), String> {
    let mut games = Games::power_on(
        Some(SRAM),
        Some(native_slots_dir()),
        Box::new(ConsolePolicy::default()),
        Box::new(ConsolePolicy::default()),
    )?;

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let bounds = video_subsystem.display_usable_bounds(0)
        .map(|bounds| (bounds.width(), bounds.height()))
        .unwrap_or((1280, 960));
    let fit = |native, line_height| Layout::fit(bounds, native, line_height);
    let mut view = View::new(fit)?;
    view.say(Source::Window, HELP.to_string());
    for (side, path) in [(Side::Emulated, &args.emulated), (Side::Native, &args.native)] {
        if let Some(path) = path {
            load(&mut games, &mut view, side, path);
        }
    }
    let mut controllers = Controllers::new(sdl_context.game_controller()?);

    let window = video_subsystem.window("gb", view.layout.width, view.layout.height)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
    let texture_creator = canvas.texture_creator();
    let new_texture = |layout: &Layout| texture_creator
        .create_texture_streaming(PixelFormatEnum::RGBA32, layout.width, layout.height)
        .map_err(|e| e.to_string());
    let mut texture = new_texture(&view.layout)?;

    let audio_queue: AudioQueue<f32> = sdl_context.audio()?.open_queue(
        None,
        &AudioSpecDesired { freq: Some(SAMPLE_RATE as i32), channels: Some(2), samples: Some(512) },
    )?;
    audio_queue.resume();
    let max_queued = (SAMPLE_RATE as f32 * MAX_QUEUED_SECONDS) as u32 * 2 * size_of::<f32>() as u32;
    let mut muted = false;
    // A frame's stereo samples, with room to spare.
    let mut samples = vec![0.0f32; SAMPLE_RATE as usize / 8 * 2];

    let mut event_pump = sdl_context.event_pump()?;
    let mut frame_times = VecDeque::new();
    let mut previous_wram = [0u8; 0x2000];
    let mut next = Instant::now();
    'running: loop {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit {..} |
                Event::KeyDown { keycode: Some(Keycode::Escape), .. } => {
                    break 'running
                },
                Event::MouseButtonDown { mouse_btn: MouseButton::Left, x, y, .. } => match view.layout.control_at(x, y) {
                    Some(Control::Routing(routing)) if !games.touring() => view.routing = routing,
                    Some(Control::Tour) => tour(&mut games, &mut view),
                    Some(Control::Speed) => view.speed = view.speed.next(),
                    _ => {}
                },
                Event::DropFile { filename, .. } => {
                    let side = view.layout.side_at(cursor_x(canvas.window()));
                    load(&mut games, &mut view, side, std::path::Path::new(&filename));
                }
                Event::ControllerDeviceAdded { which, .. } => match controllers.added(which) {
                    Ok(Some(name)) => view.say(Source::Window, format!("controller connected: {name}")),
                    Ok(None) => {}
                    Err(e) => view.say(Source::Window, format!("could not open a controller: {e}")),
                },
                Event::ControllerDeviceRemoved { which, .. } => if let Some(name) = controllers.removed(which) {
                    view.say(Source::Window, format!("controller disconnected: {name}"));
                },
                Event::KeyDown { keycode: Some(keycode @ (Keycode::F8 | Keycode::F9)), keymod, repeat: false, .. } => {
                    quick_save(&mut games, &mut view, keymod, keycode == Keycode::F9);
                }
                Event::MouseWheel { precise_y, .. } => {
                    view.log.scroll((precise_y * 3.0).round() as i32, view.layout.log_rows());
                }
                Event::KeyDown { keycode: Some(keycode), repeat: false, .. } => {
                    let touring = games.touring();
                    if keycode == Keycode::T {
                        tour(&mut games, &mut view);
                    }
                    let gb = &mut games.gb;
                    match keycode {
                        Keycode::Tab if !touring => view.routing = view.routing.next(),
                        Keycode::Num1 => view.speed = Speed::One,
                        Keycode::Num2 => view.speed = Speed::Two,
                        Keycode::Num3 => view.speed = Speed::Four,
                        Keycode::Num4 => view.speed = Speed::Unthrottled,
                        Keycode::C => {
                            view.colours = COLOUR_MODES[(COLOUR_MODES.iter().position(|&m| m == view.colours).unwrap() + 1) % COLOUR_MODES.len()];
                            let layout = fit(view.colours.size(), view.layout.line_height);
                            if layout != view.layout {
                                canvas.window_mut().set_size(layout.width, layout.height).map_err(|e| e.to_string())?;
                                texture = new_texture(&layout)?;
                                view.relayout(layout);
                            }
                        }
                        Keycode::M => {
                            muted = !muted;
                            audio_queue.clear();
                        }
                        Keycode::F1 => {
                            let ppu = gb.core().mmu().ppu();
                            ppu.dump_tilemap(TileMapMode::Lower, TileDataMode::Lower)
                                .save("tilemap_lower_lower.png")
                                .map_err(|e| e.to_string())?;
                            ppu.dump_tilemap(TileMapMode::Lower, TileDataMode::Upper)
                                .save("tilemap_lower_upper.png")
                                .map_err(|e| e.to_string())?;
                            ppu.dump_tilemap(TileMapMode::Upper, TileDataMode::Lower)
                                .save("tilemap_upper_lower.png")
                                .map_err(|e| e.to_string())?;
                            ppu.dump_tilemap(TileMapMode::Upper, TileDataMode::Upper)
                                .save("tilemap_upper_upper.png")
                                .map_err(|e| e.to_string())?;
                            ppu.screenshot()
                                .save("screenshot.png")
                                .map_err(|e| e.to_string())?;
                        }
                        Keycode::F2 => {
                            previous_wram.copy_from_slice(gb.core().mmu().work_ram());
                        }
                        Keycode::F3 => {
                            // Compare wram to previous wram
                            let current_wram = gb.core().mmu().work_ram();
                            let diff = current_wram.iter()
                                .zip(previous_wram.iter())
                                .enumerate();
                            for (index, (&current, &previous)) in diff {
                                if current != previous {
                                    println!("{:04x}: {:02x} -> {:02x}", index + 0xC000, previous, current);
                                }
                            }
                            previous_wram.copy_from_slice(current_wram);
                        }
                        Keycode::F5 if touring => {
                            view.say(Source::Window, "a tour is playing both games: T stops it".to_string());
                        }
                        Keycode::F5 => {
                            games.agent_running = !games.agent_running;
                            let state = if games.agent_running { "on" } else { "off" };
                            view.say(Source::Window, format!("the emulated agent is {state}"));
                        }
                        Keycode::F7 => {
                            // TODO write to this file on change
                            gb.dump_sram_to_file(SRAM)?;
                        }
                        Keycode::F10 => {
                            let pokemon_api = PokemonApi::new(gb);
                            println!("{:?}", pokemon_api.on_screen_text(false));
                        },
                        Keycode::F11 => view.pixels.save_png(std::path::Path::new("window.png"))?,
                        Keycode::W => {
                            let pokemon_api = PokemonApi::new(gb);
                            let menu_state = pokemon_api.menu_state().unwrap();
                            println!("{:?}", menu_state);
                        },
                        Keycode::P => {
                            let pokemon_api = PokemonApi::new(gb);
                            let map = pokemon_api.game_state()?.map;
                            println!("{}", map);
                            println!("{:?}", map.player_position);
                            println!("{:?}", map.player_direction);
                        },
                        Keycode::A => {
                            let pokemon_api = PokemonApi::new(gb);
                            let actions = pokemon_api.game_state()?.map.actions();
                            for action in actions {
                                println!("{}", action);
                            }
                        },
                        Keycode::F12 => {
                            let mut pokemon_api = PokemonApi::new(gb);
                            pokemon_api.pimp_out_pokemon()?;
                        }
                        _ => {}
                    };
                }
                _ => {}
            }
        }

        let held = buttons(&event_pump.keyboard_state()) | controllers.held();
        let played = Instant::now();
        let mut frames = 0;
        // Unthrottled, as many frames as leave the host frame time to draw.
        while view.speed.frames().map_or(frames == 0 || played.elapsed() < FRAME.mul_f32(0.8), |n| frames < n) {
            games.frame(view.routing, held, &mut |source, line| view.say(source, line));
            frames += 1;
        }

        // Drained whether or not it is heard: the samples are made either way.
        loop {
            let frames = games.read_samples(&mut samples);
            if frames == 0 {
                break;
            }
            if !muted && audio_queue.size() < max_queued {
                audio_queue.queue_audio(&samples[..frames * 2])?;
            }
        }

        frame_times.push_back(Instant::now());
        while frame_times.len() > 120 {
            frame_times.pop_front();
        }
        let fps = match (frame_times.front(), frame_times.back()) {
            (Some(first), Some(last)) if frame_times.len() > 1 =>
                (frame_times.len() - 1) as f64 / last.duration_since(*first).as_secs_f64(),
            _ => 0.0,
        };
        let mut status = format!("{fps:.1} fps   {frames}x   sound {}", if muted { "off" } else { "on" });
        if let Some(tour) = games.tour.as_ref().filter(|tour| tour.running()) {
            let seconds = tour.started.elapsed().as_secs();
            status += &format!("   tour {}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60);
        }
        view.status = status;
        view.compose(&games);
        texture.update(None, &view.pixels.rgba, view.pixels.pitch()).map_err(|e| e.to_string())?;
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

    fn parse(args: &[&str]) -> Result<Option<Args>, String> {
        Args::parse(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn the_command_line_names_a_state_for_either_game() {
        assert_eq!(parse(&[]), Ok(Some(Args::default())));
        assert_eq!(
            parse(&["--native", "a.pkrd", "--emulated", "b.bin"]),
            Ok(Some(Args { emulated: Some("b.bin".into()), native: Some("a.pkrd".into()) })),
        );
        assert_eq!(parse(&["--emulated", "b.bin", "--help"]), Ok(None));
        assert!(parse(&["--native"]).is_err());
        assert!(parse(&["b.bin"]).is_err());
    }
}
