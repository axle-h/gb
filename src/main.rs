mod opcode;
mod game_boy;
mod registers;
mod core;
mod mmu;
mod roms;
mod joypad;
mod interrupt;
mod header;
mod ppu;
mod lcd_control;
mod lcd_status;
mod geometry;
mod lcd_palette;
mod lcd_dma;
mod sdl;
mod serial;
mod cycles;
mod divider;
mod timer;
mod audio;
mod activation;

pub fn main() -> Result<(), String> {
    sdl::render::render()
}