use std::thread::sleep;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use itertools::Itertools;
use rubato::{Resampler, SincInterpolationParameters, SincInterpolationType, WindowFunction, Async, FixedAsync};
use audioadapter::direct::InterleavedSlice;
use sdl2::audio::{AudioQueue, AudioSpecDesired};
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use sdl2::pixels::PixelFormatEnum;
use crate::audio::GB_SAMPLE_RATE;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::lcd_control::{TileDataMode, TileMapMode};
use crate::sdl::frame_rate::FrameRate;
use crate::ppu::{LCD_HEIGHT, LCD_WIDTH};
use crate::roms::commercial::*;
use crate::sdl::font::FontTextures;

const SCALE_FACTOR: u32 = 4; // Scale the 160x144 LCD to fit the 640x480 window
const TARGET_FRAME_TIME: Duration = Duration::from_nanos(16666666); // 60fps
const FPS_WINDOW_SIZE: usize = 600; // 10 seconds at 60fps

pub fn render() -> Result<(), String> {
    let mut gb = GameBoy::dmg(crate::roms::blargg_dmg_sound::TRIGGER);

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

    // Create audio resampler from Game Boy native frequency (1048576 Hz) to SDL2 frequency
    // TODO use a much simpler resampler with lower latency and fewer dependencies
    //      E.g. GameBoy audio is 1048576hz, to get to 48khz we need to resample by a factor of 1048576/48000 = 8192/375
    //      So, (ref: https://en.wikipedia.org/wiki/Downsampling_(signal_processing)) we can:
    //      1. Increase (resample) the sequence by a factor of 375 (i.e. insert 374 zeros between each sample)
    //      2. Apply a low-pass filter (probably an FFT, not sure what the cut off frequency should be)
    //      3. Decrease (resample) the sequence by a factor of 8192 (i.e. keep every 8192nd sample, simple decimation)
    let mut resampler = Async::<f32>::new_sinc(
        audio_spec.freq as usize as f64 / GB_SAMPLE_RATE as f64,
        2.0,  // max_resample_ratio_relative
        SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        },
        1024, // chunk_size, 1024 is a close common factor of the GB sample rate and 44100hz
        audio_spec.channels as usize,
        FixedAsync::Input,
    ).map_err(|e| e.to_string())?;
    let mut resampled_audio_buffer = vec![0.0f32; resampler.input_frames_max() * 2];

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
    let duration_per_cycle = MachineCycles::from_m(1).to_duration();
    let mut since_last_update = Duration::ZERO;
    let mut ahead_by_cycles = MachineCycles::ZERO;

    'running: loop {
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

        let mut min_cycles = MachineCycles::ZERO;
        while since_last_update >= duration_per_cycle {
            since_last_update -= duration_per_cycle;

            if ahead_by_cycles > MachineCycles::ZERO {
                ahead_by_cycles -= MachineCycles::ONE;
            } else {
                min_cycles += MachineCycles::ONE;
            }
        }

        if min_cycles > MachineCycles::ZERO {
            let cycles =  gb.run(min_cycles);
            ahead_by_cycles += cycles - min_cycles;
        }

        let audio_buffer = gb.core_mut().mmu_mut().audio_mut().buffer_mut();
        let required_input_frames = resampler.input_frames_next();
        let required_input_samples = required_input_frames * 2; // stereo
        while audio_buffer.len() >= required_input_samples {
            let audio_sample = audio_buffer.drain(..required_input_samples).collect::<Vec<f32>>();
            let input_adapter = InterleavedSlice::new(&audio_sample, 2, audio_sample.len() / 2)
                .map_err(|e| format!("could not create input_adapter: {}", e))?;
            let output_frames = resampler.output_frames_next();
            let mut output_adapter =
                InterleavedSlice::new_mut(&mut resampled_audio_buffer, audio_spec.channels as usize, output_frames * 2)
                    .map_err(|e| format!("could not create output_adapter: {}", e))?;
            let (_, frames_written) = resampler.process_into_buffer(&input_adapter, &mut output_adapter, None)
                .map_err(|e| format!("Audio error: {}", e))?;
            audio_queue.queue_audio(&resampled_audio_buffer[..frames_written * 2])?;
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

            canvas.present();
        }

        sleep(Duration::ZERO); // allow other threads to run
    }

    Ok(())
}

