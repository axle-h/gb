use std::thread::sleep;
use std::time::Duration;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use crate::game_boy::GameBoy;
use crate::lcd_control::{TileDataMode, TileMapMode};
use crate::sdl::frame_rate::FrameRate;
use crate::ppu::{LCD_HEIGHT, LCD_WIDTH};
use crate::roms::blarg::*;
use crate::roms::commercial::*;

const SCALE_FACTOR: u32 = 4; // Scale the 160x144 LCD to fit the 640x480 window

pub fn render() -> Result<(), String> {
    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;

    let window = video_subsystem.window("gb", LCD_WIDTH as u32 * SCALE_FACTOR, LCD_HEIGHT as u32 * SCALE_FACTOR)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;

    let mut canvas = window.into_canvas().build()
        .map_err(|e| e.to_string())?;
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    canvas.present();

    // Create texture creator for LCD rendering
    let texture_creator = canvas.texture_creator();
    let mut lcd_texture = texture_creator.create_texture_streaming(
        PixelFormatEnum::RGB24, LCD_WIDTH as u32, LCD_HEIGHT as u32
    ).map_err(|e| e.to_string())?;

    let mut gb = GameBoy::dmg(POKEMON_RED);

    let mut frame_rate = FrameRate::default();
    let mut event_pump = sdl_context.event_pump()?;
    'running: loop {
        let delta = frame_rate.update()?;

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

        gb.update(delta);
        let lcd = gb.core().mmu().ppu().lcd();

        // Copy LCD data to texture
        lcd_texture.with_lock(None, |buffer: &mut [u8], pitch: usize| {
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

        canvas.clear();
        let (width, height) = canvas.window().size();
        canvas.copy(&lcd_texture, None, Rect::new(0, 0, width, height))
            .map_err(|e| e.to_string())?;
        canvas.present();

        sleep(Duration::from_millis(1));
    }

    Ok(())
}