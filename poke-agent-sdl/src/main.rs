mod sdl;

pub fn main() -> std::process::ExitCode {
    match sdl::render::render() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            std::process::ExitCode::FAILURE
        }
    }
}
