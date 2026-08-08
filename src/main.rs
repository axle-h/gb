mod opcode;
mod game_boy;
mod registers;
mod core;
mod mmu;
mod roms;
mod joypad;
mod interrupt;
mod header;
mod mbc;
mod hdma;
mod model;
mod boot_palette;
mod cgb_palette;
mod ppu;
mod lcd_control;
mod lcd_status;
mod geometry;
mod lcd_palette;
mod lcd_dma;
#[cfg(feature = "sdl")]
mod sdl;
#[cfg(feature = "web")]
mod web;
#[cfg(feature = "web")]
mod host;
#[cfg(feature = "llm")]
mod llm;
mod cli;
mod serial;
mod cycles;
mod divider;
mod timer;
mod audio;
mod activation;
mod pokemon;
mod ram;
mod rtc;
mod savestate;
mod schedule;

pub fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Command::Help) => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    match run(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: cli::Command) -> Result<(), String> {
    match command {
        cli::Command::Help => Ok(()), // handled by the caller, which needs to exit zero
        cli::Command::Ui => run_ui(),
        cli::Command::Serve { port, policy } => run_serve(port, policy),
    }
}

#[cfg(feature = "web")]
fn run_serve(port: u16, policy: cli::ServePolicy) -> Result<(), String> {
    web::run(port, policy)
}

/// `gb serve` is the whole of the `web` feature, so without it there is nothing to serve.
#[cfg(not(feature = "web"))]
fn run_serve(_port: u16, _policy: cli::ServePolicy) -> Result<(), String> {
    Err(format!("this build has no web server — it was built without the `web` feature\n\n{}",
                cli::USAGE))
}

#[cfg(feature = "sdl")]
fn run_ui() -> Result<(), String> {
    sdl::render::render()
}

/// The SDL UI is the whole of a bare `gb`, so without it there is nothing to run and the usage is
/// the most useful thing to print.
#[cfg(not(feature = "sdl"))]
fn run_ui() -> Result<(), String> {
    Err(format!("this build has no desktop UI — it was built without the `sdl` feature\n\n{}",
                cli::USAGE))
}