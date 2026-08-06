use std::thread::sleep;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use itertools::Itertools;
use sdl2::audio::{AudioQueue, AudioSpecDesired};
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use sdl2::pixels::PixelFormatEnum;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::lcd_control::{TileDataMode, TileMapMode};
use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use crate::pokemon::map_metadata::MapMetadataCache;
use crate::pokemon::policy::ConsolePolicy;
use crate::sdl::frame_rate::FrameRate;
use crate::ppu::{LCD_HEIGHT, LCD_WIDTH};
use crate::sdl::font::FontTextures;

const SCALE_FACTOR: u32 = 4; // Scale the 160x144 LCD to fit the 640x480 window
const TARGET_FRAME_TIME: Duration = Duration::from_nanos(16666666); // 60fps
const FPS_WINDOW_SIZE: usize = 600; // 10 seconds at 60fps
const REALTIME_CYCLE_DURATION: Duration = MachineCycles::from_m(1).to_duration();

pub fn render() -> Result<(), String> {
    let mut gb = GameBoy::cgb(crate::pokemon::roms::POKERED);
    let mut map_cache = MapMetadataCache::default();
    let mut pokemon_agent = PokemonAgent::new(Box::new(ConsolePolicy::default()));

    if let Err(e) = gb.restore_sram_from_file("pokemon-red.sav") {
        println!("Could not load save file: {}", e);
    }

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let audio_subsystem = sdl_context.audio()?;

    let window = video_subsystem.window("gb", LCD_WIDTH as u32 * SCALE_FACTOR, LCD_HEIGHT as u32 * SCALE_FACTOR)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;

    let mut canvas = window.into_canvas().build()
        .map_err(|e| e.to_string())?;
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    canvas.present();

    let audio_queue: AudioQueue<f32> = audio_subsystem.open_queue(None,
        &AudioSpecDesired { freq: Some(44100), channels: Some(2), samples: Some(256) }
    )?;
    let audio_spec = audio_queue.spec();
    audio_queue.resume();

    // The APU resamples itself, band-limited, straight to the sink's rate — see `audio::blip`. The
    // rate is not part of the serialised state, so it has to be re-applied here (and again after any
    // load_state).
    gb.core_mut().mmu_mut().audio_mut().set_output_sample_rate(audio_spec.freq as u32);
    // One UI iteration's worth of audio, with generous headroom. Reused every frame.
    let mut audio_scratch = vec![0.0f32; audio_spec.freq as usize / 8 * 2];

    // Create texture creator for LCD rendering
    let texture_creator = canvas.texture_creator();
    let mut lcd_texture = texture_creator.create_texture_streaming(
        PixelFormatEnum::RGB24, LCD_WIDTH as u32, LCD_HEIGHT as u32
    ).map_err(|e| e.to_string())?;
    let mut font = FontTextures::roboto_regular(
        &texture_creator,
        16.0,
        Color::RGBA(255, 0, 0, 255)
    )?;

    let mut frame_rate = FrameRate::default();
    let mut event_pump = sdl_context.event_pump()?;

    let mut since_last_render = Duration::ZERO;
    let mut frame_timestamps = VecDeque::new();
    let mut since_last_update = Duration::ZERO;
    let mut ahead_by_cycles = MachineCycles::ZERO;

    let mut iteration_count = 0;
    let mut cycle_count = MachineCycles::ZERO;
    let mut cycle_duration = REALTIME_CYCLE_DURATION;
    // Watched so a speed change can be mirrored into the resampler; see below the event loop.
    let mut applied_cycle_duration = cycle_duration;

    let mut previous_wram = [0u8; 0x2000];
    let mut agent_running = false;
    'running: loop {
        iteration_count += 1;
        let delta = frame_rate.update()?;
        since_last_render += delta;
        since_last_update += delta;

        for event in event_pump.poll_iter() {
            match event {
                Event::Quit {..} |
                Event::KeyDown { keycode: Some(Keycode::Escape), .. } => {
                    break 'running
                },
                Event::KeyDown { keycode: Some(keycode), repeat: false, .. } => {
                    use crate::joypad::JoypadButton::*;
                    match keycode {
                        Keycode::Num1 => cycle_duration = REALTIME_CYCLE_DURATION,
                        Keycode::Num2 => cycle_duration = REALTIME_CYCLE_DURATION / 2,
                        Keycode::Num3 => cycle_duration = REALTIME_CYCLE_DURATION / 3,
                        Keycode::Num4 => cycle_duration = REALTIME_CYCLE_DURATION / 4,
                        Keycode::Num5 => cycle_duration = REALTIME_CYCLE_DURATION / 5,
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
                            // compare wram to previous wram
                            let current_wram = gb.core().mmu().work_ram();
                            let diff = current_wram.into_iter()
                                .zip(previous_wram.iter())
                                .enumerate();
                            for (index, (&current, &previous)) in diff {
                                if current != previous {
                                    println!("{:04x}: {:02x} -> {:02x}", index + 0xC000, previous, current);
                                }
                            }
                            previous_wram.copy_from_slice(current_wram);
                        }
                        Keycode::F5 => {
                            agent_running = !agent_running;
                        }
                        Keycode::F7 => {
                            // TODO write to this file on change
                            gb.dump_sram_to_file("pokemon-red.sav")?;
                        }
                        Keycode::F8 => {
                            gb.save_state_to_file("pokemon-red.bin")?;
                        }
                        Keycode::F9 => {
                            gb.load_state_from_file("pokemon-red.bin")?;
                            // The resampler is not serialised, so a restored state comes back at
                            // the default output rate.
                            gb.core_mut().mmu_mut().audio_mut().set_output_sample_rate(audio_spec.freq as u32);
                        }
                        Keycode::F10 => {
                            let pokemon_api = PokemonApi::new(&mut gb);
                            // println!("{:?}", pokemon_api.player_state());
                            // println!("{:?}", pokemon_api.pokemon_party());
                            //println!("{:?}", pokemon_api.map_state());
                            // pokemon_api.game_state()?;
                            println!("{:?}", pokemon_api.on_screen_text(false));
                        },
                        Keycode::W => {
                            let pokemon_api = PokemonApi::new(&mut gb);
                            let menu_state = pokemon_api.menu_state().unwrap();
                            println!("{:?}", menu_state);
                        },
                        Keycode::M => {
                            let pokemon_api = PokemonApi::new(&mut gb);
                            let map = pokemon_api.game_state()?.map;
                            println!("{}", map);
                            println!("{:?}", map.player_position);
                            println!("{:?}", map.player_direction);
                        },
                        Keycode::A => {
                            let pokemon_api = PokemonApi::new(&mut gb);
                            let actions = pokemon_api.game_state()?.map.actions();
                            for action in actions {
                                println!("{}", action);
                            }
                        },
                        Keycode::F12 => {
                            let mut pokemon_api = PokemonApi::new(&mut gb);
                            pokemon_api.pimp_out_pokemon()?;
                        }
                        Keycode::Up => gb.core_mut().mmu_mut().joypad_mut().press_button(Up),
                        Keycode::Down => gb.core_mut().mmu_mut().joypad_mut().press_button(Down),
                        Keycode::Left => gb.core_mut().mmu_mut().joypad_mut().press_button(Left),
                        Keycode::Right => gb.core_mut().mmu_mut().joypad_mut().press_button(Right),
                        Keycode::X => gb.core_mut().mmu_mut().joypad_mut().press_button(A),
                        Keycode::Z => gb.core_mut().mmu_mut().joypad_mut().press_button(B),
                        Keycode::Return => gb.core_mut().mmu_mut().joypad_mut().press_button(Start),
                        Keycode::Backspace => gb.core_mut().mmu_mut().joypad_mut().press_button(Select),
                        _ => {}
                    };
                }
                Event::KeyUp { keycode: Some(keycode), repeat: false, .. } => {
                    use crate::joypad::JoypadButton::*;
                    match keycode {
                        Keycode::Up => gb.core_mut().mmu_mut().joypad_mut().release_button(Up),
                        Keycode::Down => gb.core_mut().mmu_mut().joypad_mut().release_button(Down),
                        Keycode::Left => gb.core_mut().mmu_mut().joypad_mut().release_button(Left),
                        Keycode::Right => gb.core_mut().mmu_mut().joypad_mut().release_button(Right),
                        Keycode::X => gb.core_mut().mmu_mut().joypad_mut().release_button(A),
                        Keycode::Z => gb.core_mut().mmu_mut().joypad_mut().release_button(B),
                        Keycode::Return => gb.core_mut().mmu_mut().joypad_mut().release_button(Start),
                        Keycode::Backspace => gb.core_mut().mmu_mut().joypad_mut().release_button(Select),
                        _ => {}
                    };
                }
                _ => {}
            }
        }

        // Keep the resampler in step with the emulation speed, otherwise fast-forwarding just
        // out-runs the audio device and backs its queue up. Derived from `cycle_duration` rather
        // than from the key that was pressed, so it tracks the speed the emulator is *actually*
        // targeting — `REALTIME_CYCLE_DURATION / 5` truncates to 190 ns, which is 5.016x, not 5x.
        if cycle_duration != applied_cycle_duration {
            applied_cycle_duration = cycle_duration;
            let speed = REALTIME_CYCLE_DURATION.as_secs_f64() / cycle_duration.as_secs_f64();
            gb.core_mut().mmu_mut().audio_mut().set_emulation_speed(speed);
            println!("emulation speed: {speed:.3}x");
        }

        let mut min_cycles = MachineCycles::ZERO;
        while since_last_update >= cycle_duration {
            since_last_update -= cycle_duration;

            if ahead_by_cycles > MachineCycles::ZERO {
                ahead_by_cycles -= MachineCycles::ONE;
            } else {
                min_cycles += MachineCycles::ONE;
            }
        }

        let mut actual_cycles = MachineCycles::ZERO;
        if min_cycles > MachineCycles::ZERO {
            actual_cycles =  gb.run(min_cycles);
            cycle_count += actual_cycles;
            ahead_by_cycles += actual_cycles - min_cycles;
        }

        if agent_running {
            let mut api = PokemonApi::with_cache(&mut gb, &mut map_cache);
            if let Err(agent_error) = pokemon_agent.update(&mut api, actual_cycles) {
                println!("agent failed: {:?}", agent_error);
            }
        }

        // Samples are ready as soon as they are synthesised, so there is no chunk to wait for —
        // drain whatever has accumulated since the last iteration.
        loop {
            let frames = gb.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut audio_scratch);
            if frames == 0 {
                break;
            }
            audio_queue.queue_audio(&audio_scratch[..frames * 2])?;
        }

        if since_last_render >= TARGET_FRAME_TIME {
            since_last_render -= TARGET_FRAME_TIME;

            canvas.clear();

            // Copy LCD data to texture
            lcd_texture.with_lock(None, |buffer: &mut [u8], pitch: usize| {
                let lcd = gb.core().mmu().ppu().lcd();
                for y in 0..LCD_HEIGHT {
                    for x in 0..LCD_WIDTH {
                        let [r, g, b] = lcd[y * LCD_WIDTH + x].to_rgb().0;
                        let pixel_color = Color::RGB(r, g, b);
                        let offset = y * pitch + x * 3;
                        buffer[offset] = pixel_color.r;
                        buffer[offset + 1] = pixel_color.g;
                        buffer[offset + 2] = pixel_color.b;
                    }
                }
            }).map_err(|e| e.to_string())?;
            canvas.copy(&lcd_texture, None, None)
                .map_err(|e| e.to_string())?;

            frame_timestamps.push_back(Instant::now());
            while frame_timestamps.len() > FPS_WINDOW_SIZE {
                frame_timestamps.pop_front();
            }

            let frame_times: Vec<Duration> = frame_timestamps.iter()
                .tuple_windows()
                .map(|(start, end)| end.duration_since(*start))
                .collect();

            let average_fps = frame_times.len() as f64 / frame_times.iter().sum::<Duration>().as_secs_f64();
            font.render_text(
                &mut canvas,
                &format!("FPS: {:.2}", average_fps),
                5,
                5
            )?;

            let average_cycles_per_iteration = cycle_count.m_cycles() as f64 / iteration_count as f64;
            font.render_text(
                &mut canvas,
                &format!("Cycles/Iter: {:.2}", average_cycles_per_iteration),
                5,
                25
            )?;

            canvas.present();
        }

        sleep(Duration::from_nanos(0)); // allow other threads to run
    }

    Ok(())
}

