mod sdl;

pub fn main() -> std::process::ExitCode {
    let args = match sdl::render::Args::parse(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{}", sdl::render::USAGE);
            return std::process::ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{message}\n{}", sdl::render::USAGE);
            return std::process::ExitCode::FAILURE;
        }
    };
    match sdl::render::render(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            std::process::ExitCode::FAILURE
        }
    }
}
