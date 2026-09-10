mod cli;
mod host;
mod web;

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

    let cli::Command::Serve { port, policy, new_run } = command else {
        return ExitCode::SUCCESS;
    };
    match web::run(port, policy, new_run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
